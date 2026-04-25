/// Background detection loop that runs document detection on preview frames
/// and pushes results to the frontend via Tauri events.
///
/// This replaces the JS-side `tick()` / `startCvLoop()` pattern with a
/// Rust-driven loop that avoids IPC round-trips for frame data.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Emitter, Manager};

use crate::scanner_cv_detect;
use crate::scanner_detect::{
    detect_document_native_ort, ScannerDetectDocumentRequest, ScannerPoint,
};
use crate::scanner_tracker::{DetectionPresenceTracker, StabilityTracker};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

static LOOP_RUNNING: AtomicBool = AtomicBool::new(false);
static LOOP_GENERATION: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Config (received from frontend)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionLoopConfig {
    /// Detection backend: `"native-ort"` or `"opencv"`.
    pub backend: String,
    /// Interval between detection ticks in milliseconds.
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    /// Number of consecutive stable frames required.
    #[serde(default = "default_stable_frames")]
    pub stable_frames: usize,
    /// Maximum per-corner standard deviation to count as stable.
    #[serde(default = "default_variance_threshold")]
    pub variance_threshold: f32,
    /// Milliseconds the document must remain stable before auto-capture fires.
    #[serde(default = "default_stable_hold_ms")]
    pub stable_hold_ms: u64,
    /// Number of missed frames before presence tracking expires.
    #[serde(default = "default_miss_grace_frames")]
    pub miss_grace_frames: u32,
    /// Milliseconds before presence tracking expires.
    #[serde(default = "default_miss_grace_ms")]
    pub miss_grace_ms: f64,
    /// Pixel distance under which smoothing is applied.
    #[serde(default = "default_smoothing_threshold_px")]
    pub smoothing_threshold_px: f32,
    /// EMA-style blending factor for smoothing (0..1).
    #[serde(default = "default_smoothing_factor")]
    pub smoothing_factor: f32,
}

fn default_interval_ms() -> u64 { 120 }
fn default_stable_frames() -> usize { 8 }
fn default_variance_threshold() -> f32 { 8.0 }
fn default_stable_hold_ms() -> u64 { 1200 }
fn default_miss_grace_frames() -> u32 { 3 }
fn default_miss_grace_ms() -> f64 { 360.0 }
fn default_smoothing_threshold_px() -> f32 { 18.0 }
fn default_smoothing_factor() -> f32 { 0.35 }

// ---------------------------------------------------------------------------
// Events pushed to frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionResultEvent {
    /// Raw detected corner points (preview coordinate space), or null.
    pub points: Option<Vec<ScannerPoint>>,
    /// Smoothed / retained effective points for overlay rendering.
    pub effective_points: Option<Vec<ScannerPoint>>,
    /// Whether the detection is currently stable.
    pub is_stable: bool,
    /// Whether auto-capture should be triggered this tick.
    pub auto_capture_triggered: bool,
    /// Detection processing time in milliseconds.
    pub detection_ms: f64,
    /// Which backend produced this result.
    pub backend: String,
    /// Preview frame dimensions.
    pub frame_width: u32,
    pub frame_height: u32,
    /// Diagnostic message from the backend.
    pub message: String,
}

const DETECTION_EVENT: &str = "scanner-detection";
const AUTO_CAPTURE_EVENT: &str = "scanner-auto-capture";

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[command]
pub async fn tauri_scanner_start_detection_loop(
    app: AppHandle,
    config: DetectionLoopConfig,
) -> Result<(), String> {
    if LOOP_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Detection loop is already running.".to_string());
    }

    let generation = LOOP_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let resource_dir = app
        .path()
        .resource_dir()
        .ok();
    let app_config_dir = app
        .path()
        .app_config_dir()
        .ok();

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        detection_loop(app_clone, config, generation, resource_dir, app_config_dir).await;

        if LOOP_GENERATION.load(Ordering::SeqCst) == generation {
            LOOP_RUNNING.store(false, Ordering::SeqCst);
        }
    });

    Ok(())
}

#[command]
pub async fn tauri_scanner_stop_detection_loop() -> Result<(), String> {
    if !LOOP_RUNNING.swap(false, Ordering::SeqCst) {
        return Err("No detection loop is currently running.".to_string());
    }
    LOOP_GENERATION.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

// ---------------------------------------------------------------------------
// Loop body
// ---------------------------------------------------------------------------

fn is_loop_current(generation: u64) -> bool {
    LOOP_RUNNING.load(Ordering::SeqCst) && LOOP_GENERATION.load(Ordering::SeqCst) == generation
}

fn now_epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}

async fn detection_loop(
    app: AppHandle,
    config: DetectionLoopConfig,
    generation: u64,
    resource_dir: Option<std::path::PathBuf>,
    app_config_dir: Option<std::path::PathBuf>,
) {
    let interval = Duration::from_millis(config.interval_ms.max(16));
    let stable_hold = Duration::from_millis(config.stable_hold_ms);
    let backend = config.backend.clone();

    let mut stability_tracker = StabilityTracker::new(
        config.stable_frames,
        config.variance_threshold,
        1, // miss_grace_frames: tolerate 1 consecutive None before clearing history
    );
    let mut presence_tracker = DetectionPresenceTracker::new(
        config.miss_grace_frames,
        config.miss_grace_ms,
        config.smoothing_threshold_px,
        config.smoothing_factor,
    );

    let mut stable_since: Option<Instant> = None;
    let mut auto_capture_cooldown_until: Option<Instant> = None;

    log::info!(
        "[DetectionLoop] Starting with backend={}, interval={}ms, stable_frames={}, hold={}ms",
        backend,
        config.interval_ms,
        config.stable_frames,
        config.stable_hold_ms,
    );

    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tick.tick().await;

        if !is_loop_current(generation) {
            break;
        }

        let (points, detection_ms, frame_width, frame_height, message) =
            if backend == "native-ort" {
                run_native_ort_detect(&resource_dir, &app_config_dir)
            } else {
                // OpenCV path — will be implemented in scanner_cv_detect.rs
                run_opencv_detect()
            };

        let quad: Option<[ScannerPoint; 4]> = points
            .as_ref()
            .and_then(|pts| {
                if pts.len() == 4 {
                    Some([pts[0], pts[1], pts[2], pts[3]])
                } else {
                    None
                }
            });

        let stable = stability_tracker.push(quad);
        let presence_state = presence_tracker.push(quad, now_epoch_ms());

        // Track auto-capture hold duration.
        if !stable || quad.is_none() {
            stable_since = None;
        } else if stable_since.is_none() {
            stable_since = Some(Instant::now());
        }

        let in_cooldown = auto_capture_cooldown_until
            .map_or(false, |until| Instant::now() < until);

        let auto_capture_triggered = stable
            && presence_state.effective_detected
            && !in_cooldown
            && stable_since
                .map_or(false, |since| since.elapsed() >= stable_hold);

        if auto_capture_triggered {
            // Set a cooldown so we don't spam auto-capture events.
            auto_capture_cooldown_until = Some(Instant::now() + Duration::from_millis(2000));
            stable_since = None;
            stability_tracker.reset();
            presence_tracker.reset();
        }

        let event = DetectionResultEvent {
            points,
            effective_points: presence_state.effective_points,
            is_stable: stable,
            auto_capture_triggered,
            detection_ms,
            backend: backend.clone(),
            frame_width,
            frame_height,
            message,
        };

        let _ = app.emit(DETECTION_EVENT, &event);

        if auto_capture_triggered {
            let _ = app.emit(AUTO_CAPTURE_EVENT, serde_json::json!({}));
        }


    }

    log::info!("[DetectionLoop] Stopped (generation={generation}).");
}

// ---------------------------------------------------------------------------
// Backend dispatch
// ---------------------------------------------------------------------------

fn run_native_ort_detect(
    resource_dir: &Option<std::path::PathBuf>,
    app_config_dir: &Option<std::path::PathBuf>,
) -> (Option<Vec<ScannerPoint>>, f64, u32, u32, String) {
    let started = Instant::now();

    let request = ScannerDetectDocumentRequest {
        source_bytes: Vec::new(),
        rgba_bytes: Vec::new(),
        use_latest_preview_frame: true,
        rgba_width: None,
        rgba_height: None,
        max_width: None,
        max_height: None,
        backend: None,
    };

    match detect_document_native_ort(request, resource_dir.clone(), app_config_dir.clone()) {
        Ok(response) => {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            let w = response.input_width.unwrap_or(0);
            let h = response.input_height.unwrap_or(0);
            (response.points, ms, w, h, response.message)
        }
        Err(error) => {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            (None, ms, 0, 0, error)
        }
    }
}

fn run_opencv_detect() -> (Option<Vec<ScannerPoint>>, f64, u32, u32, String) {
    scanner_cv_detect::detect_document_opencv(None, None)
}