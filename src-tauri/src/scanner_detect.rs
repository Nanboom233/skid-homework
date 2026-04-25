use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, RgbImage, RgbaImage};
use ort::ep::ExecutionProvider as _;
use ort::{
    ep,
    session::{builder::SessionBuilder, Session},
    value::Tensor,
};
use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Manager};

use crate::scanner_frame_protocol;
use crate::scanner_platform;
use crate::scanner_resource;
use crate::stream_decoder::get_latest_preview_frame_packet;

const STAGE: &str = "ort-runtime";
const CONFIG_RELATIVE_PATH: &str = "scanner-detect-config.json";
const WINDOWS_ORT_RELATIVE_PATH: &str = "onnxruntime/windows/onnxruntime.dll";
const WINDOWS_ORT_SHARED_RELATIVE_PATH: &str =
    "onnxruntime/windows/onnxruntime_providers_shared.dll";
const WINDOWS_DIRECTML_RELATIVE_PATH: &str = "onnxruntime/windows/DirectML.dll";
const LINUX_ORT_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime.so";
const LINUX_TENSORRT_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime_providers_tensorrt.so";
const LINUX_CUDA_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime_providers_cuda.so";


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScannerModelVariant {
    ActivePublicBaseline,
    IntendedPrimaryModel,
}

impl ScannerModelVariant {
    fn resource_key(self) -> &'static str {
        match self {
            Self::ActivePublicBaseline => "model-active-public-baseline",
            Self::IntendedPrimaryModel => "model-intended-primary",
        }
    }

    fn detection_implemented(self) -> bool {
        matches!(self, Self::ActivePublicBaseline)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectModelConfig {
    id: String,
    kind: String,
    task: String,
    model_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_size: Option<[u32; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectWindowsConfig {
    preferred_provider: String,
    runtime_library: String,
    shared_library: String,
    provider_library: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectLinuxConfig {
    preferred_providers: Vec<String>,
    runtime_library: String,
    #[serde(default)]
    provider_libraries: Vec<String>,
    official_gpu_release_artifact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectConfig {
    stage: String,
    task: String,
    intended_primary_model: ScannerDetectModelConfig,
    active_public_baseline: ScannerDetectModelConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    windows: Option<ScannerDetectWindowsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linux: Option<ScannerDetectLinuxConfig>,
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResolvedScannerModel {
    variant: ScannerModelVariant,
    config: ScannerDetectModelConfig,
}

#[derive(Debug, Clone)]
struct ScannerDetectConfigHandle {
    config: ScannerDetectConfig,
    resolved_path: PathBuf,
    source: &'static str,
}


#[derive(Debug, Clone)]
struct ResourceSpec {
    key: String,
    relative_path: String,
    required: bool,
}

#[derive(Debug, Default)]
struct OrtRuntimeState {
    environment_ready: bool,
    runtime_library_path: Option<PathBuf>,
    ort_build_info: Option<String>,
    runtime_error: Option<String>,
    available_providers: Vec<String>,
    session: Option<Session>,
    session_model_path: Option<PathBuf>,
    session_error: Option<String>,
}

#[derive(Debug, Clone)]
struct OrtRuntimeSnapshot {
    ready: bool,
    runtime_error: Option<String>,
    ort_build_info: Option<String>,
    available_providers: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SharedScannerOrtContext {
    pub preferred_provider: String,
    pub runtime_ready: bool,
    pub preferred_provider_ready: bool,
    pub runtime_error: Option<String>,
    pub context_error: Option<String>,
    pub ort_build_info: Option<String>,
    pub available_providers: Vec<String>,
}

#[derive(Debug, Clone)]
struct OrtSessionSnapshot {
    ready: bool,
    session_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectResourceStatus {
    key: String,
    relative_path: String,
    resolved_path: Option<String>,
    exists: bool,
    required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectProbeResponse {
    stage: &'static str,
    platform: String,
    platform_target: String,
    config_source: String,
    config_path: Option<String>,
    preferred_provider: String,
    provider_candidates: Vec<String>,
    selected_model_id: Option<String>,
    selected_model_kind: Option<String>,
    selected_model_task: Option<String>,
    selected_model_path: Option<String>,
    runtime_ready: bool,
    preferred_provider_ready: bool,
    model_ready: bool,
    session_ready: bool,
    detection_implemented: bool,
    ort_build_info: Option<String>,
    runtime_error: Option<String>,
    session_error: Option<String>,
    resource_resolution_source: String,
    resource_base_dir: Option<String>,
    resources: Vec<ScannerDetectResourceStatus>,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectConfigResponse {
    config: ScannerDetectConfig,
    source: String,
    resolved_path: String,
    writable_path: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPoint {
    pub x: f32,
    pub y: f32,
}

impl ScannerPoint {
    pub fn x(&self) -> f32 {
        self.x
    }

    pub fn y(&self) -> f32 {
        self.y
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectDocumentRequest {
    #[serde(default)]
    pub source_bytes: Vec<u8>,
    #[serde(default)]
    pub rgba_bytes: Vec<u8>,
    #[serde(default)]
    pub use_latest_preview_frame: bool,
    #[serde(default)]
    pub rgba_width: Option<u32>,
    #[serde(default)]
    pub rgba_height: Option<u32>,
    #[serde(default)]
    pub max_width: Option<u32>,
    #[serde(default)]
    pub max_height: Option<u32>,
    /// Detection backend: `"native-ort"` (default) or `"opencv"`.
    #[serde(default)]
    pub backend: Option<String>,
}

#[derive(Debug, Clone)]
struct PreparedInferenceImage {
    original_width: u32,
    original_height: u32,
    working_image: DynamicImage,
}

impl PreparedInferenceImage {
    fn working_dimensions(&self) -> (u32, u32) {
        self.working_image.dimensions()
    }
}

#[derive(Debug, Clone)]
struct ResolvedDetectInput {
    prepared_image: Option<PreparedInferenceImage>,
    input_transport: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DetectionContextCacheKey {
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct DetectionRuntimeContext {
    resource_base_dir: PathBuf,
    selected_model: Option<ResolvedScannerModel>,
    preferred_provider: String,
}

#[derive(Debug, Default)]
struct DetectionContextCacheState {
    key: Option<DetectionContextCacheKey>,
    context: Option<DetectionRuntimeContext>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectDocumentResponse {
    pub stage: &'static str,
    pub processing_ms: f64,
    pub input_transport: String,
    pub input_width: Option<u32>,
    pub input_height: Option<u32>,
    pub selected_model_id: Option<String>,
    pub selected_model_kind: Option<String>,
    pub selected_model_task: Option<String>,
    pub runtime_ready: bool,
    pub preferred_provider: String,
    pub preferred_provider_ready: bool,
    pub model_ready: bool,
    pub session_ready: bool,
    pub detection_implemented: bool,
    pub ort_build_info: Option<String>,
    pub runtime_error: Option<String>,
    pub session_error: Option<String>,
    pub points: Option<Vec<ScannerPoint>>,
    pub message: String,
}

impl ScannerDetectDocumentResponse {
    pub fn detected_points(&self) -> Option<&[ScannerPoint]> {
        self.points.as_deref()
    }
}

#[command]
pub async fn tauri_scanner_probe_detect(app: AppHandle) -> Result<ScannerDetectProbeResponse, String> {
    let resource_dir_hint = app.path().resource_dir().ok();
    let app_config_dir_hint = app.path().app_config_dir().ok();
    Ok(probe_native_ort_runtime_with_hints(
        resource_dir_hint,
        app_config_dir_hint,
    ))
}

#[command]
pub async fn tauri_scanner_detect_document(
    app: AppHandle,
    source_bytes: Option<Vec<u8>>,
    mut request: ScannerDetectDocumentRequest,
) -> Result<ScannerDetectDocumentResponse, String> {
    if let Some(bytes) = source_bytes {
        if !bytes.is_empty() && request.source_bytes.is_empty() {
            request.source_bytes = bytes;
        }
    }
    let backend = request.backend.as_deref().unwrap_or("native-ort");
    match backend {
        "opencv" => {
            tauri::async_runtime::spawn_blocking(move || {
                detect_document_opencv_from_bytes(request)
            })
            .await
            .map_err(|error| format!("OpenCV detect task failed: {error}"))?
        }
        _ => {
            let resource_dir_hint = app.path().resource_dir().ok();
            let app_config_dir_hint = app.path().app_config_dir().ok();
            tauri::async_runtime::spawn_blocking(move || {
                detect_document_native_ort(request, resource_dir_hint, app_config_dir_hint)
            })
            .await
            .map_err(|error| format!("Native ORT task failed: {error}"))?
        }
    }
}

#[command]
pub async fn tauri_scanner_read_detect_config(
    app: AppHandle,
) -> Result<ScannerDetectConfigResponse, String> {
    let resource_dir_hint = app.path().resource_dir().ok();
    let app_config_dir_hint = app.path().app_config_dir().ok();
    let resolved = resolve_scanner_detect_config(resource_dir_hint, app_config_dir_hint)?;
    Ok(build_scanner_detect_config_response(
        &resolved,
        build_scanner_detect_config_writable_path(app.path().app_config_dir().ok())?,
    ))
}

#[command]
pub async fn tauri_scanner_write_detect_config(
    app: AppHandle,
    config: ScannerDetectConfig,
) -> Result<ScannerDetectConfigResponse, String> {
    let writable_path = build_scanner_detect_config_writable_path(app.path().app_config_dir().ok())?;
    validate_scanner_detect_config(&config)?;
    write_scanner_detect_config(&writable_path, &config)?;
    reset_scanner_detect_runtime_caches();

    Ok(ScannerDetectConfigResponse {
        config,
        source: "app-config-override".to_string(),
        resolved_path: scanner_resource::path_to_string(&writable_path),
        writable_path: scanner_resource::path_to_string(&writable_path),
    })
}

pub fn probe_native_ort_runtime() -> ScannerDetectProbeResponse {
    probe_native_ort_runtime_with_hints(None, None)
}

pub fn probe_native_ort_runtime_with_hints(
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> ScannerDetectProbeResponse {
    let platform = std::env::consts::OS.to_string();
    let platform_target = scanner_platform::platform_target().to_string();
    let provider_candidates = scanner_platform::provider_candidates()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();

    let resource_root_candidates = scanner_resource::build_resource_root_candidates(resource_dir_hint);
    let selected_resource_root = scanner_resource::select_resource_root(&resource_root_candidates, &DETECT_INTERESTING_PATHS);
    let resource_base_dir = selected_resource_root
        .as_ref()
        .map(|candidate| candidate.path.clone());
    let config_handle = resource_base_dir.as_ref().and_then(|base_dir| {
        resolve_scanner_detect_config(Some(base_dir.clone()), app_config_dir_hint.clone()).ok()
    });
    let config_error = if resource_base_dir.is_some() && config_handle.is_none() {
        resolve_scanner_detect_config(resource_base_dir.clone(), app_config_dir_hint.clone()).err()
    } else {
        None
    };
    let resource_specs =
        resource_specs_for_current_platform(config_handle.as_ref().map(|handle| &handle.config));
    let resources = build_resource_statuses(resource_base_dir.as_deref(), &resource_specs);
    let selected_model = config_handle
        .as_ref()
        .and_then(|handle| select_model_variant(handle, &resources));
    let preferred_provider = config_handle
        .as_ref()
        .map(|handle| preferred_provider_from_config(&handle.config))
        .unwrap_or_else(|| scanner_platform::default_preferred_provider().to_string());
    let runtime_snapshot = probe_ort_runtime(resource_base_dir.as_deref());
    let session_snapshot = if let Some(ref model) = selected_model {
        if runtime_snapshot.ready {
            ensure_ort_session(resource_base_dir.as_deref(), model, &preferred_provider)
        } else {
            OrtSessionSnapshot {
                ready: false,
                session_error: Some(
                    "ONNX Runtime environment is not ready yet, so the model session was not created."
                        .to_string(),
                ),
            }
        }
    } else {
        OrtSessionSnapshot {
            ready: false,
            session_error: Some(
                "No supported stage-1 model was found. Install the public baseline model or provide the planned ORT model."
                    .to_string(),
            ),
        }
    };
    let preferred_provider_ready =
        scanner_platform::is_provider_available(&preferred_provider, &runtime_snapshot.available_providers);

    let message = if !runtime_snapshot.ready {
        runtime_snapshot
            .runtime_error
            .clone()
            .unwrap_or_else(|| "Native scanner runtime is not ready yet.".to_string())
    } else if let Some(error) = config_error.clone() {
        error
    } else if let Some(ref model) = selected_model {
        if !session_snapshot.ready {
            session_snapshot
                .session_error
                .clone()
                .unwrap_or_else(|| "The selected model session is not ready.".to_string())
        } else if !model.variant.detection_implemented() {
            format!(
                "Model {} is present and its ORT session is ready, but Rust-side output decoding is not implemented yet.",
                model.config.id
            )
        } else {
            format!(
                "Model {} is loaded and native Rust inference is ready.",
                model.config.id
            )
        }
    } else {
        "No supported stage-1 model was found under the resolved resource directory.".to_string()
    };

    ScannerDetectProbeResponse {
        stage: STAGE,
        platform,
        platform_target,
        config_source: config_handle
            .as_ref()
            .map(|handle| handle.source.to_string())
            .unwrap_or_else(|| "unresolved".to_string()),
        config_path: config_handle
            .as_ref()
            .map(|handle| scanner_resource::path_to_string(&handle.resolved_path)),
        preferred_provider,
        provider_candidates,
        selected_model_id: selected_model.as_ref().map(|model| model.config.id.clone()),
        selected_model_kind: selected_model
            .as_ref()
            .map(|model| model.config.kind.clone()),
        selected_model_task: selected_model
            .as_ref()
            .map(|model| model.config.task.clone()),
        selected_model_path: selected_model.as_ref().and_then(|model| {
            resource_base_dir
                .as_deref()
                .map(|base_dir| scanner_resource::path_to_string(&base_dir.join(model.config.model_path.as_str())))
        }),
        runtime_ready: runtime_snapshot.ready,
        preferred_provider_ready,
        model_ready: selected_model.is_some(),
        session_ready: session_snapshot.ready,
        detection_implemented: selected_model
            .map(|model| model.variant.detection_implemented())
            .unwrap_or(false),
        ort_build_info: runtime_snapshot.ort_build_info,
        runtime_error: runtime_snapshot.runtime_error.or(config_error),
        session_error: session_snapshot.session_error,
        resource_resolution_source: selected_resource_root
            .map(|candidate| candidate.source.to_string())
            .unwrap_or_else(|| "unresolved".to_string()),
        resource_base_dir: resource_base_dir.as_deref().map(scanner_resource::path_to_string),
        resources,
        message,
    }
}

/// Run OpenCV contour-based detection on encoded source bytes.
/// Used for redetect when the user has selected the `"opencv"` backend.
fn detect_document_opencv_from_bytes(
    request: ScannerDetectDocumentRequest,
) -> Result<ScannerDetectDocumentResponse, String> {
    let started_at = Instant::now();
    let dynamic_image = image::load_from_memory(&request.source_bytes)
        .map_err(|e| format!("Failed to decode source for OpenCV detect: {e}"))?;
    let (w, h) = (dynamic_image.width(), dynamic_image.height());
    let points = crate::scanner_cv_detect::detect_contour_quad(
        &dynamic_image, request.max_width, request.max_height,
    );
    let ms = started_at.elapsed().as_secs_f64() * 1000.0;
    let message = if points.is_some() {
        format!("OpenCV contour redetect completed in {ms:.1}ms.")
    } else {
        format!("No document detected via OpenCV redetect ({ms:.1}ms).")
    };
    Ok(ScannerDetectDocumentResponse {
        stage: "opencv-redetect",
        processing_ms: ms,
        input_transport: "source_bytes".to_string(),
        input_width: Some(w),
        input_height: Some(h),
        selected_model_id: None,
        selected_model_kind: None,
        selected_model_task: None,
        runtime_ready: false,
        preferred_provider: String::new(),
        preferred_provider_ready: false,
        model_ready: false,
        session_ready: false,
        detection_implemented: true,
        ort_build_info: None,
        runtime_error: None,
        session_error: None,
        points,
        message,
    })
}

pub fn detect_document_native_ort(
    mut request: ScannerDetectDocumentRequest,
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> Result<ScannerDetectDocumentResponse, String> {
    let started_at = Instant::now();
    let ResolvedDetectInput {
        prepared_image,
        input_transport: input_transport_kind,
    } = resolve_detect_input(&mut request)?;
    let input_transport = input_transport_kind.to_string();

    let (input_width, input_height) = prepared_image
        .as_ref()
        .map(|image| (Some(image.original_width), Some(image.original_height)))
        .unwrap_or((None, None));

    let detection_context_result =
        resolve_detection_runtime_context(resource_dir_hint, app_config_dir_hint);
    let default_provider = scanner_platform::default_preferred_provider().to_string();
    let (selected_model, preferred_provider, runtime_snapshot, session_snapshot, context_error) =
        match detection_context_result {
            Ok(context) => {
                let runtime_snapshot = probe_ort_runtime(Some(context.resource_base_dir.as_path()));
                let session_snapshot = if let Some(ref model) = context.selected_model {
                    if runtime_snapshot.ready {
                        ensure_ort_session(
                            Some(context.resource_base_dir.as_path()),
                            model,
                            &context.preferred_provider,
                        )
                    } else {
                        OrtSessionSnapshot {
                            ready: false,
                            session_error: Some(
                                "ONNX Runtime environment is not ready yet, so the model session was not created."
                                    .to_string(),
                            ),
                        }
                    }
                } else {
                    OrtSessionSnapshot {
                        ready: false,
                        session_error: Some(
                            "No supported stage-1 model was selected, so the model session is unavailable."
                                .to_string(),
                        ),
                    }
                };

                (
                    context.selected_model,
                    context.preferred_provider,
                    runtime_snapshot,
                    session_snapshot,
                    None,
                )
            }
            Err(error) => (
                None,
                default_provider,
                OrtRuntimeSnapshot {
                    ready: false,
                    runtime_error: Some(error.clone()),
                    ort_build_info: None,
                    available_providers: Vec::new(),
                },
                OrtSessionSnapshot {
                    ready: false,
                    session_error: Some(error.clone()),
                },
                Some(error),
            ),
        };
    let preferred_provider_ready =
        scanner_platform::is_provider_available(&preferred_provider, &runtime_snapshot.available_providers);

    let (points, message) = if !runtime_snapshot.ready {
        (
            None,
            runtime_snapshot
                .runtime_error
                .clone()
                .or(context_error.clone())
                .unwrap_or_else(|| "Native scanner runtime is not ready yet.".to_string()),
        )
    } else if !session_snapshot.ready {
        (
            None,
            session_snapshot.session_error.clone().unwrap_or_else(|| {
                "ONNX Runtime is ready but the model session is not.".to_string()
            }),
        )
    } else if prepared_image.is_none() {
        (
            None,
            match input_transport_kind {
                "latest-preview-cache" => {
                    "The latest live preview frame was unavailable, so native inference was skipped."
                        .to_string()
                }
                _ => {
                    "The request did not include image bytes or RGBA frame data, so inference was skipped."
                        .to_string()
                }
            },
        )
    } else if let Some(model) = selected_model.clone() {
        let prepared_image = prepared_image.as_ref().expect("image exists");
        let (working_width, working_height) = prepared_image.working_dimensions();
        match run_selected_model_inference(&model, &prepared_image.working_image) {
            Ok(points) => {
                let scaled_points = scale_points_between_dimensions(
                    points,
                    working_width,
                    working_height,
                    prepared_image.original_width,
                    prepared_image.original_height,
                );
                (
                    Some(scaled_points),
                    format!(
                        "Model {} completed native Rust inference at {}x{} working resolution.",
                        model.config.id, working_width, working_height
                    ),
                )
            }
            Err(error) => (None, error),
        }
    } else {
        (
            None,
            "No supported stage-1 model was selected, so inference was skipped.".to_string(),
        )
    };
    let detection_implemented = selected_model
        .as_ref()
        .map(|model| model.variant.detection_implemented())
        .unwrap_or(false);

    Ok(ScannerDetectDocumentResponse {
        stage: STAGE,
        processing_ms: started_at.elapsed().as_secs_f64() * 1000.0,
        input_transport,
        input_width,
        input_height,
        selected_model_id: selected_model.as_ref().map(|model| model.config.id.clone()),
        selected_model_kind: selected_model
            .as_ref()
            .map(|model| model.config.kind.clone()),
        selected_model_task: selected_model
            .as_ref()
            .map(|model| model.config.task.clone()),
        runtime_ready: runtime_snapshot.ready,
        preferred_provider,
        preferred_provider_ready,
        model_ready: selected_model.is_some(),
        session_ready: session_snapshot.ready,
        detection_implemented,
        ort_build_info: runtime_snapshot.ort_build_info,
        runtime_error: runtime_snapshot.runtime_error.or(context_error),
        session_error: session_snapshot.session_error,
        points,
        message,
    })
}

/// Resolve the input image for native ORT detection.
///
/// Takes `&mut` to move large pixel buffers out of the request (via `std::mem::take`)
/// instead of cloning them.
fn resolve_detect_input(
    request: &mut ScannerDetectDocumentRequest,
) -> Result<ResolvedDetectInput, String> {
    let (decoded_image, input_transport) = if request.use_latest_preview_frame {
        (resolve_detect_from_preview_cache()?, "latest-preview-cache")
    } else if !request.rgba_bytes.is_empty() {
        (
            Some(resolve_detect_from_rgba_owned(request)?),
            "rgba-ipc",
        )
    } else if !request.source_bytes.is_empty() {
        (
            Some(resolve_detect_from_encoded_owned(request)?),
            "source-bytes",
        )
    } else {
        (None, "none")
    };

    Ok(ResolvedDetectInput {
        // Skip the max_width/max_height pre-shrink for the Native ORT path.
        // The model function (`run_docaligner_fastvit_sa24`) will resize the
        // image to the model's input_size (e.g. 256×256) in a single step.
        // Applying the frontend's processing bounds here would create a wasteful
        // double-resize chain (e.g. 640×360 →320×180 →256×256) that degrades
        // the heatmap quality through accumulated interpolation blur.
        prepared_image: decoded_image.map(|image| prepare_inference_image(image, None, None)),
        input_transport,
    })
}

/// Resolve detection input from the latest preview frame cache.
fn resolve_detect_from_preview_cache() -> Result<Option<DynamicImage>, String> {
    let cached_preview_packet = get_latest_preview_frame_packet();
    cached_preview_packet
        .as_deref()
        .map(|p| build_dynamic_image_from_preview_frame_packet(p))
        .transpose()
}

/// Resolve detection input from raw RGBA bytes in the request (takes ownership).
fn resolve_detect_from_rgba_owned(
    request: &mut ScannerDetectDocumentRequest,
) -> Result<DynamicImage, String> {
    let width = request
        .rgba_width
        .ok_or_else(|| "RGBA native scanner request is missing rgbaWidth.".to_string())?;
    let height = request
        .rgba_height
        .ok_or_else(|| "RGBA native scanner request is missing rgbaHeight.".to_string())?;

    if width == 0 || height == 0 {
        return Err("RGBA native scanner dimensions must be greater than zero.".to_string());
    }

    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "RGBA native scanner dimensions overflowed.".to_string())?;

    if request.rgba_bytes.len() != expected_len {
        return Err(format!(
            "RGBA native scanner payload length mismatch: expected {expected_len} bytes for {width}x{height}, got {}.",
            request.rgba_bytes.len()
        ));
    }

    // Take ownership to avoid cloning the large pixel buffer.
    let rgba_bytes = std::mem::take(&mut request.rgba_bytes);
    let image = RgbaImage::from_raw(width, height, rgba_bytes).ok_or_else(|| {
        "Failed to materialize RGBA source frame for native scanner inference.".to_string()
    })?;
    Ok(DynamicImage::ImageRgba8(image))
}

/// Resolve detection input from encoded image bytes (PNG/JPEG) in the request (takes ownership).
fn resolve_detect_from_encoded_owned(
    request: &mut ScannerDetectDocumentRequest,
) -> Result<DynamicImage, String> {
    let source_bytes = std::mem::take(&mut request.source_bytes);
    image::load_from_memory(&source_bytes).map_err(|error| {
        format!("Failed to decode source image for native scanner inference: {error}")
    })
}

pub(crate) fn build_dynamic_image_from_preview_frame_packet(packet: &[u8]) -> Result<DynamicImage, String> {
    scanner_frame_protocol::build_dynamic_image_from_preview_frame_packet(packet)
}

fn prepare_inference_image(
    image: DynamicImage,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> PreparedInferenceImage {
    let (original_width, original_height) = image.dimensions();
    let (working_width, working_height) =
        bounded_dimensions(original_width, original_height, max_width, max_height);

    let working_image = if working_width == original_width && working_height == original_height {
        image
    } else {
        // Resize in RGB to avoid unnecessary RGBA roundtrip →the downstream
        // inference path (`run_docaligner_fastvit_sa24`) converts to RGB anyway.
        DynamicImage::ImageRgb8(image::imageops::resize(
            &image.to_rgb8(),
            working_width,
            working_height,
            FilterType::Triangle,
        ))
    };

    PreparedInferenceImage {
        original_width,
        original_height,
        working_image,
    }
}

fn bounded_dimensions(
    width: u32,
    height: u32,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> (u32, u32) {
    let max_width = max_width.filter(|value| *value > 0).unwrap_or(width);
    let max_height = max_height.filter(|value| *value > 0).unwrap_or(height);
    let scale = f64::min(
        1.0,
        f64::min(
            max_width as f64 / f64::from(width.max(1)),
            max_height as f64 / f64::from(height.max(1)),
        ),
    );

    (
        u32::max(1, (f64::from(width) * scale).round() as u32),
        u32::max(1, (f64::from(height) * scale).round() as u32),
    )
}

fn scale_points_between_dimensions(
    points: Vec<ScannerPoint>,
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> Vec<ScannerPoint> {
    if source_width == target_width && source_height == target_height {
        return points;
    }

    points
        .into_iter()
        .map(|point| ScannerPoint {
            x: scale_coordinate(point.x, source_width, target_width),
            y: scale_coordinate(point.y, source_height, target_height),
        })
        .collect()
}

fn build_scanner_detect_config_response(
    handle: &ScannerDetectConfigHandle,
    writable_path: PathBuf,
) -> ScannerDetectConfigResponse {
    ScannerDetectConfigResponse {
        config: handle.config.clone(),
        source: handle.source.to_string(),
        resolved_path: scanner_resource::path_to_string(&handle.resolved_path),
        writable_path: scanner_resource::path_to_string(&writable_path),
    }
}

fn build_scanner_detect_config_writable_path(
    app_config_dir_hint: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let Some(app_config_dir) = app_config_dir_hint else {
        return Err("Could not resolve the writable app config directory.".to_string());
    };

    Ok(app_config_dir.join(CONFIG_RELATIVE_PATH))
}

fn resolve_scanner_detect_config(
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> Result<ScannerDetectConfigHandle, String> {
    let resource_root_candidates = scanner_resource::build_resource_root_candidates(resource_dir_hint);
    let selected_resource_root = scanner_resource::select_resource_root(&resource_root_candidates, &DETECT_INTERESTING_PATHS)
        .ok_or_else(|| "Could not resolve the scanner resource directory.".to_string())?;
    let default_config_path = selected_resource_root.path.join(CONFIG_RELATIVE_PATH);
    let override_config_path = build_scanner_detect_config_writable_path(app_config_dir_hint).ok();

    if let Some(override_path) = override_config_path.as_ref() {
        if override_path.exists() {
            let config = load_scanner_detect_config_from_path(override_path)?;
            return Ok(ScannerDetectConfigHandle {
                config,
                resolved_path: override_path.clone(),
                source: "app-config-override",
            });
        }
    }

    let config = load_scanner_detect_config_from_path(&default_config_path)?;
    Ok(ScannerDetectConfigHandle {
        config,
        resolved_path: default_config_path,
        source: "bundled-resource-default",
    })
}

fn load_scanner_detect_config_from_path(path: &Path) -> Result<ScannerDetectConfig, String> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed to read scanner ORT config {}: {error}",
            scanner_resource::path_to_string(path)
        )
    })?;
    let config = serde_json::from_str::<ScannerDetectConfig>(&raw).map_err(|error| {
        format!(
            "Failed to parse scanner ORT config {}: {error}",
            scanner_resource::path_to_string(path)
        )
    })?;
    validate_scanner_detect_config(&config)?;
    Ok(config)
}

fn write_scanner_detect_config(path: &Path, config: &ScannerDetectConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create scanner ORT config directory {}: {error}",
                scanner_resource::path_to_string(parent)
            )
        })?;
    }

    let payload = serde_json::to_string_pretty(config)
        .map_err(|error| format!("Failed to serialize scanner ORT config: {error}"))?;
    fs::write(path, payload + "\n").map_err(|error| {
        format!(
            "Failed to write scanner ORT config {}: {error}",
            scanner_resource::path_to_string(path)
        )
    })
}

fn validate_scanner_detect_config(config: &ScannerDetectConfig) -> Result<(), String> {
    validate_scanner_detect_model_config(&config.intended_primary_model, "intendedPrimaryModel")?;
    validate_scanner_detect_model_config(&config.active_public_baseline, "activePublicBaseline")?;

    if let Some(windows) = config.windows.as_ref() {
        if windows.preferred_provider.trim().is_empty() {
            return Err("windows.preferredProvider must not be empty.".to_string());
        }
    }

    if let Some(linux) = config.linux.as_ref() {
        if linux.preferred_providers.is_empty() {
            return Err("linux.preferredProviders must not be empty.".to_string());
        }
        if linux
            .preferred_providers
            .iter()
            .any(|provider| provider.trim().is_empty())
        {
            return Err("linux.preferredProviders must not contain empty entries.".to_string());
        }
    }

    Ok(())
}

fn validate_scanner_detect_model_config(
    config: &ScannerDetectModelConfig,
    label: &str,
) -> Result<(), String> {
    if config.id.trim().is_empty()
        || config.kind.trim().is_empty()
        || config.task.trim().is_empty()
        || config.model_path.trim().is_empty()
    {
        return Err(format!("{label} contains empty required fields."));
    }

    if let Some(input_size) = config.input_size {
        if input_size[0] == 0 || input_size[1] == 0 {
            return Err(format!("{label}.inputSize entries must be positive."));
        }
    }

    Ok(())
}

pub fn reset_scanner_detect_runtime_caches() {
    reset_detection_context_cache();
    reset_ort_session_cache();
}

fn reset_detection_context_cache() {
    let mut state = detection_context_cache()
        .lock()
        .expect("detection context cache mutex should not be poisoned");
    *state = DetectionContextCacheState::default();
}

fn reset_ort_session_cache() {
    let mut state = runtime_state()
        .lock()
        .expect("ORT runtime state mutex should not be poisoned");
    state.session = None;
    state.session_model_path = None;
    state.session_error = None;
}


/// Interesting paths used to score resource roots for the detection subsystem.
const DETECT_INTERESTING_PATHS: [&str; 9] = [
    CONFIG_RELATIVE_PATH,
    "models/docaligner-fastvit_sa24.onnx",
    "models/document-boundary-ORT-pose.onnx",
    WINDOWS_ORT_RELATIVE_PATH,
    WINDOWS_ORT_SHARED_RELATIVE_PATH,
    WINDOWS_DIRECTML_RELATIVE_PATH,
    LINUX_ORT_RELATIVE_PATH,
    LINUX_TENSORRT_RELATIVE_PATH,
    LINUX_CUDA_RELATIVE_PATH,
];


fn resource_specs_for_current_platform(config: Option<&ScannerDetectConfig>) -> Vec<ResourceSpec> {
    let mut specs = vec![ResourceSpec {
        key: "config".to_string(),
        relative_path: CONFIG_RELATIVE_PATH.to_string(),
        required: true,
    }];

    if let Some(config) = config {
        for model in candidate_model_variants(config) {
            specs.push(ResourceSpec {
                key: model.variant.resource_key().to_string(),
                relative_path: model.config.model_path.clone(),
                required: model.variant.detection_implemented(),
            });
        }
    }

    match std::env::consts::OS {
        "windows" => {
            specs.push(ResourceSpec {
                key: "windows-ort-core".to_string(),
                relative_path: WINDOWS_ORT_RELATIVE_PATH.to_string(),
                required: true,
            });
            specs.push(ResourceSpec {
                key: "windows-ort-shared".to_string(),
                relative_path: WINDOWS_ORT_SHARED_RELATIVE_PATH.to_string(),
                required: true,
            });
            specs.push(ResourceSpec {
                key: "windows-directml".to_string(),
                relative_path: WINDOWS_DIRECTML_RELATIVE_PATH.to_string(),
                required: false,
            });
        }
        "linux" => {
            specs.push(ResourceSpec {
                key: "linux-ort-core".to_string(),
                relative_path: LINUX_ORT_RELATIVE_PATH.to_string(),
                required: true,
            });
            specs.push(ResourceSpec {
                key: "linux-tensorrt-provider".to_string(),
                relative_path: LINUX_TENSORRT_RELATIVE_PATH.to_string(),
                required: false,
            });
            specs.push(ResourceSpec {
                key: "linux-cuda-provider".to_string(),
                relative_path: LINUX_CUDA_RELATIVE_PATH.to_string(),
                required: false,
            });
        }
        _ => {}
    }

    specs
}

fn build_resource_statuses(
    resource_base_dir: Option<&Path>,
    specs: &[ResourceSpec],
) -> Vec<ScannerDetectResourceStatus> {
    specs
        .iter()
        .map(|spec| {
            let resolved_path =
                resource_base_dir.map(|base_dir| base_dir.join(&spec.relative_path));
            let exists = resolved_path
                .as_ref()
                .map(|path| path.exists())
                .unwrap_or(false);

            ScannerDetectResourceStatus {
                key: spec.key.clone(),
                relative_path: spec.relative_path.clone(),
                resolved_path: resolved_path.as_deref().map(scanner_resource::path_to_string),
                exists,
                required: spec.required,
            }
        })
        .collect()
}

fn resource_exists(resources: &[ScannerDetectResourceStatus], key: &str) -> bool {
    resources
        .iter()
        .find(|resource| resource.key == key)
        .map(|resource| resource.exists)
        .unwrap_or(false)
}

fn preferred_provider_from_config(config: &ScannerDetectConfig) -> String {
    match std::env::consts::OS {
        "windows" => config
            .windows
            .as_ref()
            .map(|windows| windows.preferred_provider.clone())
            .unwrap_or_else(|| scanner_platform::default_preferred_provider().to_string()),
        "linux" => config
            .linux
            .as_ref()
            .and_then(|linux| linux.preferred_providers.first().cloned())
            .unwrap_or_else(|| scanner_platform::default_preferred_provider().to_string()),
        _ => scanner_platform::default_preferred_provider().to_string(),
    }
}

fn candidate_model_variants(config: &ScannerDetectConfig) -> Vec<ResolvedScannerModel> {
    vec![
        ResolvedScannerModel {
            variant: ScannerModelVariant::IntendedPrimaryModel,
            config: config.intended_primary_model.clone(),
        },
        ResolvedScannerModel {
            variant: ScannerModelVariant::ActivePublicBaseline,
            config: config.active_public_baseline.clone(),
        },
    ]
}

fn select_model_variant(
    handle: &ScannerDetectConfigHandle,
    resources: &[ScannerDetectResourceStatus],
) -> Option<ResolvedScannerModel> {
    candidate_model_variants(&handle.config)
        .into_iter()
        .find(|model| resource_exists(resources, model.variant.resource_key()))
}

fn runtime_state() -> &'static Mutex<OrtRuntimeState> {
    static STATE: OnceLock<Mutex<OrtRuntimeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(OrtRuntimeState::default()))
}

fn detection_context_cache() -> &'static Mutex<DetectionContextCacheState> {
    static STATE: OnceLock<Mutex<DetectionContextCacheState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(DetectionContextCacheState::default()))
}

fn resolve_detection_runtime_context(
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> Result<DetectionRuntimeContext, String> {
    let key = DetectionContextCacheKey {
        resource_dir_hint: resource_dir_hint.clone(),
        app_config_dir_hint: app_config_dir_hint.clone(),
    };
    let mut cache = detection_context_cache()
        .lock()
        .expect("detection context cache mutex should not be poisoned");

    if cache.key.as_ref() == Some(&key) {
        if let Some(context) = cache.context.clone() {
            return Ok(context);
        }
        if let Some(error) = cache.error.clone() {
            return Err(error);
        }
    }

    let result = build_detection_runtime_context(resource_dir_hint, app_config_dir_hint);
    cache.key = Some(key);
    match result {
        Ok(context) => {
            cache.context = Some(context.clone());
            cache.error = None;
            Ok(context)
        }
        Err(error) => {
            cache.context = None;
            cache.error = Some(error.clone());
            Err(error)
        }
    }
}

fn build_detection_runtime_context(
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> Result<DetectionRuntimeContext, String> {
    let resource_root_candidates = scanner_resource::build_resource_root_candidates(resource_dir_hint);
    let selected_resource_root = scanner_resource::select_resource_root(&resource_root_candidates, &DETECT_INTERESTING_PATHS)
        .ok_or_else(|| "Could not resolve the scanner resource directory.".to_string())?;
    let resource_base_dir = selected_resource_root.path;
    let config_handle =
        resolve_scanner_detect_config(Some(resource_base_dir.clone()), app_config_dir_hint)?;
    let resource_specs = resource_specs_for_current_platform(Some(&config_handle.config));
    let resources = build_resource_statuses(Some(&resource_base_dir), &resource_specs);
    let selected_model = select_model_variant(&config_handle, &resources);
    let preferred_provider = preferred_provider_from_config(&config_handle.config);

    Ok(DetectionRuntimeContext {
        resource_base_dir,
        selected_model,
        preferred_provider,
    })
}

pub(crate) fn ensure_shared_scanner_ort_context(
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> SharedScannerOrtContext {
    let context_result = resolve_detection_runtime_context(resource_dir_hint, app_config_dir_hint);
    let resource_base_dir = context_result
        .as_ref()
        .ok()
        .map(|context| context.resource_base_dir.clone());
    let preferred_provider = context_result
        .as_ref()
        .ok()
        .map(|context| context.preferred_provider.clone())
        .unwrap_or_else(|| scanner_platform::default_preferred_provider().to_string());
    let runtime_snapshot = probe_ort_runtime(resource_base_dir.as_deref());
    let preferred_provider_ready =
        scanner_platform::is_provider_available(&preferred_provider, &runtime_snapshot.available_providers);

    SharedScannerOrtContext {
        preferred_provider,
        runtime_ready: runtime_snapshot.ready,
        preferred_provider_ready,
        runtime_error: runtime_snapshot.runtime_error,
        context_error: context_result.err(),
        ort_build_info: runtime_snapshot.ort_build_info,
        available_providers: runtime_snapshot.available_providers,
    }
}

fn probe_ort_runtime(resource_base_dir: Option<&Path>) -> OrtRuntimeSnapshot {
    let Some(resource_base_dir) = resource_base_dir else {
        return OrtRuntimeSnapshot {
            ready: false,
            runtime_error: Some("Could not resolve the scanner resource directory.".to_string()),
            ort_build_info: None,
            available_providers: Vec::new(),
        };
    };

    let Some(runtime_library_path) = runtime_library_path_for_current_platform(resource_base_dir)
    else {
        return OrtRuntimeSnapshot {
            ready: false,
            runtime_error: Some(
                "This platform does not have a configured ORT runtime path yet.".to_string(),
            ),
            ort_build_info: None,
            available_providers: vec!["CPU".to_string()],
        };
    };

    if !runtime_library_path.exists() {
        return OrtRuntimeSnapshot {
            ready: false,
            runtime_error: Some(format!(
                "Missing ONNX Runtime library: {}.",
                scanner_resource::path_to_string(&runtime_library_path)
            )),
            ort_build_info: None,
            available_providers: Vec::new(),
        };
    }

    let mut state = runtime_state()
        .lock()
        .expect("ORT runtime state mutex should not be poisoned");

    if !state.environment_ready {
        state.runtime_library_path = Some(runtime_library_path.clone());

        let init_result = ort::init_from(&runtime_library_path)
            .map(|builder| {
                builder
                    .with_name("scanner-native-ort")
                    .with_telemetry(false)
                    .commit()
            })
            .map_err(|error| {
                format!(
                    "Failed to initialize ONNX Runtime from {}: {error}",
                    scanner_resource::path_to_string(&runtime_library_path)
                )
            });

        match init_result {
            Ok(_) => {
                state.environment_ready = true;
                state.runtime_error = None;
                state.ort_build_info = Some(ort::info().to_string());
                state.available_providers = available_providers_for_current_platform();
            }
            Err(error) => {
                state.environment_ready = false;
                state.runtime_error = Some(error);
            }
        }
    }

    OrtRuntimeSnapshot {
        ready: state.environment_ready,
        runtime_error: state.runtime_error.clone(),
        ort_build_info: state.ort_build_info.clone(),
        available_providers: state.available_providers.clone(),
    }
}

fn ensure_ort_session(
    resource_base_dir: Option<&Path>,
    model: &ResolvedScannerModel,
    preferred_provider: &str,
) -> OrtSessionSnapshot {
    let Some(resource_base_dir) = resource_base_dir else {
        return OrtSessionSnapshot {
            ready: false,
            session_error: Some("Could not resolve the scanner resource directory.".to_string()),
        };
    };

    let model_path = resource_base_dir.join(model.config.model_path.as_str());
    if !model_path.exists() {
        return OrtSessionSnapshot {
            ready: false,
            session_error: Some(format!(
                "Missing ONNX model file: {}.",
                scanner_resource::path_to_string(&model_path)
            )),
        };
    }

    let mut state = runtime_state()
        .lock()
        .expect("ORT runtime state mutex should not be poisoned");

    if !state.environment_ready {
        return OrtSessionSnapshot {
            ready: false,
            session_error: Some(
                "ONNX Runtime environment is not initialized, so the model session cannot be created."
                    .to_string(),
            ),
        };
    }

    let same_model_loaded = state
        .session_model_path
        .as_ref()
        .map(|loaded_path| loaded_path == &model_path)
        .unwrap_or(false);

    if state.session.is_some() && same_model_loaded {
        return OrtSessionSnapshot {
            ready: true,
            session_error: None,
        };
    }

    let session_result = Session::builder()
        .map_err(|error| format!("Failed to create ORT session builder: {error}"))
        .and_then(|builder| {
            let mut builder = configure_scanner_session_builder_for_current_platform(builder)?;
            builder = builder
                .with_execution_providers(build_scanner_execution_providers(preferred_provider))
                .map_err(|error| format!("Failed to configure execution providers: {error}"))?;

            builder
                .commit_from_file(&model_path)
                .map_err(|error| format!("Failed to load ONNX model session: {error}"))
        });

    match session_result {
        Ok(session) => {
            state.session = Some(session);
            state.session_model_path = Some(model_path);
            state.session_error = None;
        }
        Err(error) => {
            state.session = None;
            state.session_model_path = None;
            state.session_error = Some(error);
        }
    }

    OrtSessionSnapshot {
        ready: state.session.is_some(),
        session_error: state.session_error.clone(),
    }
}

pub(crate) fn configure_scanner_session_builder_for_current_platform(
    builder: SessionBuilder,
) -> Result<SessionBuilder, String> {
    match std::env::consts::OS {
        "windows" => {
            // DirectML requires sequential execution and disabled memory-pattern optimization.
            builder
                .with_parallel_execution(false)
                .and_then(|builder: SessionBuilder| builder.with_memory_pattern(false))
                .map_err(|error| format!("Failed to apply DirectML-safe session options: {error}"))
        }
        _ => Ok(builder),
    }
}

fn run_selected_model_inference(
    model: &ResolvedScannerModel,
    image: &DynamicImage,
) -> Result<Vec<ScannerPoint>, String> {
    match model.variant {
        ScannerModelVariant::ActivePublicBaseline => run_docaligner_fastvit_sa24(model, image),
        ScannerModelVariant::IntendedPrimaryModel => Err(
            "The planned ORT pose model can be loaded, but Rust-side output decoding for it is not implemented yet."
                .to_string(),
        ),
    }
}

fn run_docaligner_fastvit_sa24(
    model: &ResolvedScannerModel,
    image: &DynamicImage,
) -> Result<Vec<ScannerPoint>, String> {
    let original = image.to_rgb8();
    let (original_width, original_height) = original.dimensions();
    let input_size = model
        .config
        .input_size
        .ok_or_else(|| "activePublicBaseline.inputSize must be configured.".to_string())?;
    let input_name = model
        .config
        .input_name
        .as_deref()
        .ok_or_else(|| "activePublicBaseline.inputName must be configured.".to_string())?;
    let output_name = model
        .config
        .output_name
        .as_deref()
        .ok_or_else(|| "activePublicBaseline.outputName must be configured.".to_string())?;
    let resized = image::imageops::resize(
        &original,
        input_size[0],
        input_size[1],
        FilterType::Triangle,
    );
    let input = build_docaligner_input(&resized);
    let input_tensor = Tensor::<f32>::from_array((
        vec![
            1_i64,
            3_i64,
            i64::from(input_size[1]),
            i64::from(input_size[0]),
        ],
        input,
    ))
    .map_err(|error| format!("Failed to build ORT input tensor: {error}"))?;

    // Lock the runtime state, run inference, extract the output tensor data
    // into owned buffers, and release the lock *before* running the expensive
    // heatmap post-processing (BFS centroid etc.).
    let (heatmap_data, heatmap_shape) = {
        let mut state = runtime_state()
            .lock()
            .expect("ORT runtime state mutex should not be poisoned");
        let session = state
            .session
            .as_mut()
            .ok_or_else(|| "ORT session is not initialized for the selected model.".to_string())?;
        let outputs = session
            .run(ort::inputs![input_name => input_tensor])
            .map_err(|error| format!("Failed to run ORT inference: {error}"))?;
        let output_value = outputs.get(output_name).unwrap_or(&outputs[0]);
        let (shape, tensor_view) = output_value
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("Failed to extract heatmap tensor: {error}"))?;
        // Copy tensor data into an owned Vec so we can drop the MutexGuard.
        (tensor_view.to_vec(), shape.to_vec())
        // MutexGuard is dropped here →lock is released before post-processing.
    };

    decode_docaligner_heatmap_output(
        &heatmap_data,
        &heatmap_shape,
        original_width,
        original_height,
    )
}

fn build_docaligner_input(image: &RgbImage) -> Vec<f32> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let plane_len = width * height;
    let mut input = vec![0.0_f32; plane_len * 3];

    for (idx, pixel) in image.pixels().enumerate() {
        let [r, g, b] = pixel.0;
        input[idx] = f32::from(b) / 255.0;
        input[plane_len + idx] = f32::from(g) / 255.0;
        input[(plane_len * 2) + idx] = f32::from(r) / 255.0;
    }

    input
}

fn decode_docaligner_heatmap_output(
    heatmap: &[f32],
    shape: &[i64],
    output_width: u32,
    output_height: u32,
) -> Result<Vec<ScannerPoint>, String> {
    if shape.len() != 4 {
        return Err(format!(
            "Unexpected DocAligner output rank {}. Expected [1, 4, H, W].",
            shape.len()
        ));
    }

    let channels =
        usize::try_from(shape[1]).map_err(|_| "Invalid heatmap channel count.".to_string())?;
    let heatmap_height =
        usize::try_from(shape[2]).map_err(|_| "Invalid heatmap height.".to_string())?;
    let heatmap_width =
        usize::try_from(shape[3]).map_err(|_| "Invalid heatmap width.".to_string())?;

    if channels < 4 {
        return Err(format!(
            "Unexpected DocAligner heatmap channel count {channels}. Expected at least 4."
        ));
    }

    let plane_len = heatmap_width * heatmap_height;
    let expected_len = plane_len * channels;
    if heatmap.len() < expected_len {
        return Err(format!(
            "Heatmap tensor is truncated: expected at least {expected_len} values, got {}.",
            heatmap.len()
        ));
    }

    let mut points = Vec::with_capacity(4);
    for channel in 0..4 {
        let start = channel * plane_len;
        let end = start + plane_len;
        let plane = &heatmap[start..end];
        let centroid =
            largest_component_centroid_f32(plane, heatmap_width, heatmap_height, 77.0 / 255.0)
                .or_else(|| argmax_point_f32(plane, heatmap_width, heatmap_height))
                .ok_or_else(|| {
                    format!(
                "DocAligner heatmap channel {channel} did not produce a detectable corner region."
            )
                })?;

        points.push(ScannerPoint {
            x: scale_coordinate(centroid.0, heatmap_width as u32, output_width),
            y: scale_coordinate(centroid.1, heatmap_height as u32, output_height),
        });
    }

    Ok(points)
}

fn largest_component_centroid_f32(
    heatmap: &[f32],
    width: usize,
    height: usize,
    threshold: f32,
) -> Option<(f32, f32)> {
    let mut visited = vec![false; width * height];
    let mut best_count = 0usize;
    let mut best_centroid = None;

    for y in 0..height {
        for x in 0..width {
            let index = (y * width) + x;
            if visited[index] || heatmap.get(index).copied().unwrap_or(0.0) <= threshold {
                continue;
            }

            let mut queue = VecDeque::from([(x, y)]);
            visited[index] = true;
            let mut count = 0usize;
            let mut sum_x = 0f64;
            let mut sum_y = 0f64;
            let mut total_weight = 0f64;

            while let Some((cx, cy)) = queue.pop_front() {
                count += 1;
                let current_index = (cy * width) + cx;
                let weight = heatmap
                    .get(current_index)
                    .copied()
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0) as f64;
                let effective_weight = if weight > 0.0 { weight } else { 1.0 };
                sum_x += cx as f64 * effective_weight;
                sum_y += cy as f64 * effective_weight;
                total_weight += effective_weight;

                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }

                        let nx = cx as isize + dx;
                        let ny = cy as isize + dy;
                        if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                            continue;
                        }

                        let nx = nx as usize;
                        let ny = ny as usize;
                        let neighbor_index = (ny * width) + nx;
                        if visited[neighbor_index]
                            || heatmap.get(neighbor_index).copied().unwrap_or(0.0) <= threshold
                        {
                            continue;
                        }

                        visited[neighbor_index] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }

            if count > best_count {
                best_count = count;
                let divisor = if total_weight > 0.0 {
                    total_weight
                } else {
                    count as f64
                };
                best_centroid = Some(((sum_x / divisor) as f32, (sum_y / divisor) as f32));
            }
        }
    }

    best_centroid
}

fn argmax_point_f32(heatmap: &[f32], width: usize, _height: usize) -> Option<(f32, f32)> {
    let (best_index, _) = heatmap
        .iter()
        .copied()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))?;

    Some(((best_index % width) as f32, (best_index / width) as f32))
}

fn scale_coordinate(value: f32, source_size: u32, target_size: u32) -> f32 {
    if source_size <= 1 || target_size <= 1 {
        return 0.0;
    }

    (((value + 0.5) / source_size as f32) * target_size as f32 - 0.5)
        .clamp(0.0, target_size.saturating_sub(1) as f32)
}

fn runtime_library_path_for_current_platform(resource_base_dir: &Path) -> Option<PathBuf> {
    match std::env::consts::OS {
        "windows" => Some(resource_base_dir.join(WINDOWS_ORT_RELATIVE_PATH)),
        "linux" => Some(resource_base_dir.join(LINUX_ORT_RELATIVE_PATH)),
        _ => None,
    }
}

fn available_providers_for_current_platform() -> Vec<String> {
    let mut providers = Vec::new();

    match std::env::consts::OS {
        "windows" => {
            let directml = ep::DirectML::default();
            if directml.supported_by_platform() && directml.is_available().unwrap_or(false) {
                providers.push("DirectML".to_string());
            }
            providers.push("CPU".to_string());
        }
        "linux" => {
            let tensorrt = ep::TensorRT::default();
            if tensorrt.supported_by_platform() && tensorrt.is_available().unwrap_or(false) {
                providers.push("TensorRT".to_string());
            }

            let cuda = ep::CUDA::default();
            if cuda.supported_by_platform() && cuda.is_available().unwrap_or(false) {
                providers.push("CUDA".to_string());
            }

            providers.push("CPU".to_string());
        }
        _ => {
            providers.push("CPU".to_string());
        }
    }

    providers
}

pub(crate) fn build_scanner_execution_providers(
    preferred_provider: &str,
) -> Vec<ort::execution_providers::ExecutionProviderDispatch> {
    let normalized = scanner_platform::normalize_provider_name(preferred_provider);

    match std::env::consts::OS {
        "windows" => {
            if normalized == "CPU" {
                vec![ep::CPU::default().build()]
            } else {
                vec![ep::DirectML::default().build(), ep::CPU::default().build()]
            }
        }
        "linux" => {
            let mut order = Vec::new();
            match normalized.as_str() {
                "CUDA" => {
                    order.push("CUDA");
                    order.push("TensorRT");
                }
                "CPU" => {}
                _ => {
                    order.push("TensorRT");
                    order.push("CUDA");
                }
            }

            let mut providers = Vec::new();
            for provider in order {
                match provider {
                    "TensorRT" => providers.push(ep::TensorRT::default().build()),
                    "CUDA" => providers.push(ep::CUDA::default().build()),
                    _ => {}
                }
            }
            providers.push(ep::CPU::default().build());
            providers
        }
        _ => vec![ep::CPU::default().build()],
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::scanner_frame_protocol::FRAME_CODEC_I420_TELEMETRY;
    use crate::stream_decoder::replace_latest_preview_frame_packet;

    fn sample_scanner_detect_config() -> ScannerDetectConfig {
        ScannerDetectConfig {
            stage: "runtime-plus-public-baseline".to_string(),
            task: "document-boundary-stage1".to_string(),
            intended_primary_model: ScannerDetectModelConfig {
                id: "document-boundary-ORT-pose-4pt".to_string(),
                kind: "planned-primary".to_string(),
                task: "document-corner-keypoints".to_string(),
                model_path: "models/document-boundary-ORT-pose.onnx".to_string(),
                input_name: None,
                output_name: None,
                input_size: None,
            },
            active_public_baseline: ScannerDetectModelConfig {
                id: "docaligner-fastvit-sa24".to_string(),
                kind: "public-baseline".to_string(),
                task: "document-corner-heatmap".to_string(),
                model_path: "models/docaligner-fastvit_sa24.onnx".to_string(),
                input_name: Some("img".to_string()),
                output_name: Some("heatmap".to_string()),
                input_size: Some([256, 256]),
            },
            windows: Some(ScannerDetectWindowsConfig {
                preferred_provider: "directml".to_string(),
                runtime_library: WINDOWS_ORT_RELATIVE_PATH.to_string(),
                shared_library: WINDOWS_ORT_SHARED_RELATIVE_PATH.to_string(),
                provider_library: WINDOWS_DIRECTML_RELATIVE_PATH.to_string(),
            }),
            linux: Some(ScannerDetectLinuxConfig {
                preferred_providers: vec!["tensorrt".to_string(), "cuda".to_string()],
                runtime_library: LINUX_ORT_RELATIVE_PATH.to_string(),
                provider_libraries: vec![
                    LINUX_TENSORRT_RELATIVE_PATH.to_string(),
                    LINUX_CUDA_RELATIVE_PATH.to_string(),
                ],
                official_gpu_release_artifact: "onnxruntime-linux-x64-gpu-1.24.4.tgz".to_string(),
            }),
            notes: Vec::new(),
        }
    }

    fn sample_preview_frame_packet() -> Vec<u8> {
        let width = 2_u32;
        let height = 2_u32;
        let y_plane = [16_u8, 64, 128, 235];
        let u_plane = [128_u8];
        let v_plane = [128_u8];

        let mut packet = Vec::new();
        packet.push(FRAME_CODEC_I420_TELEMETRY);
        packet.extend_from_slice(&width.to_be_bytes());
        packet.extend_from_slice(&height.to_be_bytes());
        packet.extend_from_slice(&0_u64.to_be_bytes());
        packet.extend_from_slice(&1_u32.to_be_bytes());
        packet.extend_from_slice(&y_plane);
        packet.extend_from_slice(&u_plane);
        packet.extend_from_slice(&v_plane);
        packet
    }

    #[test]
    fn current_platform_has_expected_provider_candidates() {
        let providers = scanner_platform::provider_candidates();
        assert!(!providers.is_empty());
        assert!(providers.contains(&"CPU"));
    }

    #[test]
    fn current_platform_has_config_and_model_specs() {
        let config = sample_scanner_detect_config();
        let specs = resource_specs_for_current_platform(Some(&config));
        assert!(specs
            .iter()
            .any(|spec| spec.key == "config" && spec.required));
        assert!(specs
            .iter()
            .any(|spec| spec.key == "model-active-public-baseline"));
        assert!(specs
            .iter()
            .any(|spec| spec.key == "model-intended-primary"));
    }

    #[test]
    fn build_execution_providers_always_keeps_cpu_fallback() {
        let providers = build_scanner_execution_providers("TensorRT");
        assert!(!providers.is_empty());
    }

    #[test]
    fn validate_scanner_detect_config_rejects_empty_linux_provider_entries() {
        let mut config = sample_scanner_detect_config();
        if let Some(linux) = config.linux.as_mut() {
            linux.preferred_providers = vec!["cuda".to_string(), "".to_string()];
        }

        let error = validate_scanner_detect_config(&config)
            .expect_err("config with empty provider should fail");
        assert!(error.contains("linux.preferredProviders"));
    }

    #[test]
    fn runtime_library_path_matches_current_platform() {
        let base_dir = PathBuf::from("C:\\tmp\\scanner-resources");
        let path = runtime_library_path_for_current_platform(&base_dir);

        match std::env::consts::OS {
            "windows" => assert_eq!(
                path.expect("windows runtime path should exist"),
                base_dir.join(WINDOWS_ORT_RELATIVE_PATH)
            ),
            "linux" => assert_eq!(
                path.expect("linux runtime path should exist"),
                base_dir.join(LINUX_ORT_RELATIVE_PATH)
            ),
            _ => assert!(path.is_none()),
        }
    }

    #[test]
    fn bounded_dimensions_scales_into_requested_limits() {
        assert_eq!(
            bounded_dimensions(640, 360, Some(320), Some(180)),
            (320, 180)
        );
        assert_eq!(
            bounded_dimensions(4032, 3024, Some(1024), Some(1024)),
            (1024, 768)
        );
        assert_eq!(
            bounded_dimensions(320, 180, Some(640), Some(360)),
            (320, 180)
        );
    }

    #[test]
    fn largest_component_centroid_prefers_biggest_region() {
        let width = 6;
        let height = 4;
        let mut mask = vec![0.0f32; width * height];

        for &(x, y) in &[(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
            mask[(y * width) + x] = 1.0;
        }
        for &(x, y) in &[(4usize, 1usize), (5, 1)] {
            mask[(y * width) + x] = 1.0;
        }

        let centroid = largest_component_centroid_f32(&mask, width, height, 0.1)
            .expect("centroid should exist");
        assert!(centroid.0 < 1.0);
        assert!(centroid.1 < 1.0);
    }

    #[test]
    fn build_dynamic_image_from_preview_frame_packet_decodes_i420() {
        let image = build_dynamic_image_from_preview_frame_packet(&sample_preview_frame_packet())
            .expect("preview frame packet should decode");
        let rgb = image.to_rgb8();
        assert_eq!(rgb.dimensions(), (2, 2));
        assert!(rgb.get_pixel(0, 0).0[0] <= rgb.get_pixel(1, 1).0[0]);
    }

    #[test]
    fn resolve_detect_input_uses_latest_preview_frame_cache() {
        replace_latest_preview_frame_packet(Some(Arc::new(sample_preview_frame_packet())));
        let mut request = ScannerDetectDocumentRequest {
            source_bytes: Vec::new(),
            rgba_bytes: Vec::new(),
            use_latest_preview_frame: true,
            rgba_width: None,
            rgba_height: None,
            max_width: Some(2),
            max_height: Some(2),
            backend: None,
        };

        let resolved =
            resolve_detect_input(&mut request).expect("latest preview cache should resolve");
        assert_eq!(resolved.input_transport, "latest-preview-cache");
        let image = resolved
            .prepared_image
            .expect("cached preview frame should provide an image");
        assert_eq!(image.working_dimensions(), (2, 2));
        replace_latest_preview_frame_packet(None);
    }

    #[test]
    fn resolve_detect_input_handles_missing_latest_preview_frame() {
        replace_latest_preview_frame_packet(None);
        let mut request = ScannerDetectDocumentRequest {
            source_bytes: Vec::new(),
            rgba_bytes: Vec::new(),
            use_latest_preview_frame: true,
            rgba_width: None,
            rgba_height: None,
            max_width: None,
            max_height: None,
            backend: None,
        };

        let resolved = resolve_detect_input(&mut request)
            .expect("missing latest preview frame should not error");
        assert_eq!(resolved.input_transport, "latest-preview-cache");
        assert!(resolved.prepared_image.is_none());
    }
}
