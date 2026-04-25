use std::collections::VecDeque;
use std::io::Cursor;

use std::time::Instant;


use image::imageops::{rotate180, rotate270, rotate90};
use image::{ColorType, GrayImage, ImageEncoder, Luma, Rgba, RgbaImage};
use imageproc::contrast::otsu_level;
use imageproc::filter::gaussian_blur_f32;
use imageproc::geometric_transformations::{warp_into, Interpolation, Projection};
use serde::{Deserialize, Serialize};
use tauri::{
    command,
    ipc::{Channel, InvokeResponseBody},
    AppHandle, Manager,
};

use crate::scanner_postprocess_model::{
    describe_native_postprocess_model_with_runtime_hints,
    run_native_postprocess_model_with_runtime_hints,
};

use crate::scanner_detect::ScannerPoint;


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPostProcessRequest {
    #[serde(default)]
    source_bytes: Vec<u8>,
    document_points: Option<Vec<ScannerPoint>>,
    output_rotation: u16,
    image_enhancement: bool,
    /// Controls the color mode for enhancement output.
    /// - `"auto"` (default): decides based on content analysis
    /// - `"color"`: preserve original colors, only flatten background luminance
    /// - `"grayscale"`: output grayscale with normalized tones
    /// - `"binary"`: output black & white
    #[serde(default = "default_color_mode")]
    color_mode: String,
    /// Controls whether the native phase-2 residual-control-point branch should be attempted.
    #[serde(default = "default_postprocess_backend")]
    postprocess_backend: String,
    #[serde(default = "default_true")]
    spine_flattening: bool,
    /// Whether to apply perspective transform to produce a top-down view.
    #[serde(default = "default_true")]
    perspective_transform: bool,
    /// Grid post-processing mode for UVDoc output.
    /// - `"none"` (default): raw grid, no post-processing.
    /// - `"x-stretch-equalize"`: per-row X linspace equalization.
    #[serde(default = "default_grid_postprocess")]
    grid_postprocess: String,
    /// Whether to save intermediate pipeline images to disk for debugging.
    #[serde(default)]
    pipeline_debug: bool,
}

fn default_true() -> bool {
    true
}   

fn default_color_mode() -> String {
    "none".to_string()
}

fn default_postprocess_backend() -> String {
    "heuristic".to_string()
}

fn default_grid_postprocess() -> String {
    "none".to_string()
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPostProcessResponse {
    processing_ms: f64,
    decode_ms: f64,
    refine_ms: Option<f64>,
    perspective_ms: Option<f64>,
    flatten_ms: Option<f64>,
    enhance_ms: Option<f64>,
    model_ms: Option<f64>,
    residual_warp_ms: Option<f64>,
    rotate_ms: Option<f64>,
    encode_ms: f64,
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    encoded_mime_type: &'static str,
    postprocess_backend: String,
    model_id: Option<String>,
    control_grid_shape: Option<String>,
    effective_document_points: Option<Vec<ScannerPoint>>,
    refinement_applied: bool,
    local_flattening_applied: bool,
    residual_warp_applied: bool,
    residual_warp_fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScannerPostProcessBackend {
    Heuristic,
    NativeMlV1,
}

impl ScannerPostProcessBackend {
    fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "native-ml-v1" => Self::NativeMlV1,
            _ => Self::Heuristic,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::NativeMlV1 => "native-ml-v1",
        }
    }
}

#[derive(Debug)]
struct ResidualWarpAttempt {
    backend_used: ScannerPostProcessBackend,
    model_id: Option<String>,
    control_grid_shape: Option<String>,
    model_ms: Option<f64>,
    residual_warp_ms: Option<f64>,
    residual_warp_applied: bool,
    fallback_reason: Option<String>,
    image: Option<RgbaImage>,
}

fn send_raw_payload(
    channel: &Channel<InvokeResponseBody>,
    bytes: Vec<u8>,
    context: &str,
) -> Result<(), String> {
    channel
        .send(InvokeResponseBody::Raw(bytes))
        .map_err(|error| format!("Failed to deliver {context} to the frontend: {error}"))
}

#[command]
pub async fn tauri_scanner_postprocess_image(
    app: AppHandle,
    source_bytes: Vec<u8>,
    mut request: ScannerPostProcessRequest,
    payload_channel: Channel<InvokeResponseBody>,
) -> Result<ScannerPostProcessResponse, String> {
    request.source_bytes = source_bytes;
    let resource_dir_hint = app.path().resource_dir().ok();
    let app_config_dir_hint = app.path().app_config_dir().ok();
    let (response, encoded_png) = tauri::async_runtime::spawn_blocking(move || {
        process_image_request(request, resource_dir_hint, app_config_dir_hint)
    })
    .await
    .map_err(|error| format!("Native scanner post-process task failed: {error}"))??;

    send_raw_payload(
        &payload_channel,
        encoded_png,
        "native scanner post-process result",
    )?;
    Ok(response)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefineDocumentCornersRequest {
    #[serde(default)]
    source_bytes: Vec<u8>,
    document_points: Vec<ScannerPoint>,
}

#[command]
pub async fn tauri_scanner_refine_document_corners(
    source_bytes: Vec<u8>,
    mut request: RefineDocumentCornersRequest,
) -> Result<Vec<ScannerPoint>, String> {
    request.source_bytes = source_bytes;
    tauri::async_runtime::spawn_blocking(move || {
        let decoded = image::load_from_memory(&request.source_bytes)
            .map_err(|error| format!("Failed to decode source image: {error}"))?;
        let current = decoded.into_rgba8();

        let points = normalize_document_points(Some(&request.document_points))?
            .ok_or_else(|| "Need exactly 4 document points for refinement.".to_string())?;

        let refined = refine_document_points(&current, &points);
        Ok(refined.to_vec())
    })
    .await
    .map_err(|error| format!("Corner refinement task failed: {error}"))?
}

pub fn postprocess_image_bytes(
    source_bytes: Vec<u8>,
    document_points: Option<&[crate::scanner_detect::ScannerPoint]>,
    output_rotation: u16,
    image_enhancement: bool,
) -> Result<(ScannerPostProcessResponse, Vec<u8>), String> {
    postprocess_image_bytes_with_options(
        source_bytes,
        document_points,
        output_rotation,
        image_enhancement,
        default_color_mode(),
        default_postprocess_backend(),
        None,
        None,
        true,
        true,
        default_grid_postprocess(),
    )
}


pub fn postprocess_image_bytes_with_options(
    source_bytes: Vec<u8>,
    document_points: Option<&[crate::scanner_detect::ScannerPoint]>,
    output_rotation: u16,
    image_enhancement: bool,
    color_mode: String,
    postprocess_backend: String,
    resource_dir_hint: Option<std::path::PathBuf>,
    app_config_dir_hint: Option<std::path::PathBuf>,
    spine_flattening: bool,
    perspective_transform: bool,
    grid_postprocess: String,
) -> Result<(ScannerPostProcessResponse, Vec<u8>), String> {
    let request = ScannerPostProcessRequest {
        source_bytes,
        document_points: document_points.map(|points| {
            points
                .iter()
                .map(|point| ScannerPoint {
                    x: point.x(),
                    y: point.y(),
                })
                .collect()
        }),
        output_rotation,
        image_enhancement,
        color_mode,
        postprocess_backend,
        spine_flattening,
        perspective_transform,
        grid_postprocess,
        pipeline_debug: false,
    };
    process_image_request(request, resource_dir_hint, app_config_dir_hint)
}

pub fn process_scanner_postprocess_request(
    request: ScannerPostProcessRequest,
    resource_dir_hint: Option<std::path::PathBuf>,
    app_config_dir_hint: Option<std::path::PathBuf>,
) -> Result<(ScannerPostProcessResponse, Vec<u8>), String> {
    process_image_request(request, resource_dir_hint, app_config_dir_hint)
}

/// Fire-and-forget: clone the image and save it on a background thread
/// so the pipeline is not blocked by PNG encoding + disk I/O.
fn debug_save_image(image: &RgbaImage, path: std::path::PathBuf) {
    let cloned = image.clone();
    std::thread::spawn(move || {
        if let Err(e) = cloned.save(&path) {
            eprintln!("[DEBUG] failed to save {:?}: {}", path, e);
        }
    });
}

fn process_image_request(
    request: ScannerPostProcessRequest,
    resource_dir_hint: Option<std::path::PathBuf>,
    app_config_dir_hint: Option<std::path::PathBuf>,
) -> Result<(ScannerPostProcessResponse, Vec<u8>), String> {
    let started_at = Instant::now();
    let decode_started_at = Instant::now();
    let decoded = image::load_from_memory(&request.source_bytes)
        .map_err(|error| format!("Failed to decode source image: {error}"))?;
    let decode_ms = decode_started_at.elapsed().as_secs_f64() * 1000.0;

    let mut current = decoded.into_rgba8();
    let pre_rotation_width = current.width();
    let pre_rotation_height = current.height();

    // ── Decode-time rotation: rotate immediately, overwrite source ──
    let mut rotate_ms = None;
    if request.output_rotation != 0 {
        let t = Instant::now();
        current = rotate_image(current, request.output_rotation)?;
        rotate_ms = Some(t.elapsed().as_secs_f64() * 1000.0);
    }
    let input_width = current.width();   // rotated dimensions
    let input_height = current.height();

    // Frontend sends points in source (un-rotated) coordinate space.
    // Transform coordinates to match the now-rotated image, then
    // deterministically permute the [TL,TR,BR,BL] slots so the ordering
    // matches the rotated image space (required by compute_document_projection).
    let effective_document_points =
        normalize_document_points(request.document_points.as_deref())?;
    let effective_document_points = effective_document_points.map(|points| {
        if request.output_rotation == 0 {
            points
        } else {
            let rotated = transform_points_for_rotation(
                &points, pre_rotation_width, pre_rotation_height,
                request.output_rotation,
            );
            reorder_points_after_rotation(rotated, request.output_rotation)
        }
    });

    let mut perspective_ms = None;
    let mut flatten_ms = None;
    let mut enhance_ms = None;
    let mut model_ms = None;
    let mut residual_warp_ms = None;
    let mut local_flattening_applied = false;
    let mut residual_warp_applied = false;
    let residual_warp_fallback_reason = None;
    let mut postprocess_backend =
        ScannerPostProcessBackend::from_str(request.postprocess_backend.as_str());
    let mut model_id = None;
    let mut control_grid_shape = None;

    // ── Debug image output (gated by request.pipeline_debug) ──
    let debug_dir = if request.pipeline_debug {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("pipeline_debug_{}", ts));
        std::fs::create_dir_all(&dir).ok();
        eprintln!("[DEBUG] writing intermediate images to {:?}", dir);
        Some(dir)
    } else {
        None
    };
    if let Some(ref dir) = debug_dir {
        debug_save_image(&current, dir.join("step0_input.png"));
    }

    eprintln!(
        "[Pipeline] begin | source={}x{} | rotated={}x{} | rotate={} | perspective={} | flatten={} | enhance={} | backend={} | grid_pp={}",
        pre_rotation_width, pre_rotation_height,
        input_width, input_height,
        request.output_rotation,
        request.perspective_transform,
        request.spine_flattening,
        request.image_enhancement,
        request.postprocess_backend,
        request.grid_postprocess,
    );

    // ── Step 1: Perspective Transform / Crop ──

    if let Some(ref points) = effective_document_points {
        validate_quad_geometry(points, current.width(), current.height())?;

        if request.perspective_transform {
            let perspective_started_at = Instant::now();
            let (target_w, target_h, projection, _from, _to) =
                compute_document_projection(points, current.width() as f32, current.height() as f32)?;
            perspective_ms = Some(perspective_started_at.elapsed().as_secs_f64() * 1000.0);

            let mut warped = RgbaImage::new(target_w, target_h);
            warp_into(
                &current,
                &projection,
                Interpolation::Bilinear,
                Rgba([0, 0, 0, 0]),
                &mut warped,
            );
            current = warped;
            eprintln!("[Pipeline] step1:perspective → {}x{} ({:.1}ms)", current.width(), current.height(), perspective_ms.unwrap());
            if let Some(ref dir) = debug_dir {
                debug_save_image(&current, dir.join("step1_perspective.png"));
            }
        } else {
            // Perspective is off — still crop to the bounding box of the 4 corner points.
            let min_x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min).max(0.0) as u32;
            let min_y = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min).max(0.0) as u32;
            let max_x = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max).ceil() as u32;
            let max_y = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max).ceil() as u32;
            let max_x = max_x.min(current.width());
            let max_y = max_y.min(current.height());
            if max_x > min_x && max_y > min_y {
                let cropped = image::imageops::crop_imm(&current, min_x, min_y, max_x - min_x, max_y - min_y).to_image();
                current = cropped;
                eprintln!("[Pipeline] step1:crop → {}x{}", current.width(), current.height());
            }
        }
    }

    // ── Step 2: Spine Flattening ──
    if request.spine_flattening {
        if postprocess_backend == ScannerPostProcessBackend::NativeMlV1 {
            let result = attempt_residual_control_point_stage(
                resource_dir_hint.clone(),
                app_config_dir_hint.clone(),
                &current,
                None,
                &request.grid_postprocess,
            );

            postprocess_backend = result.backend_used.clone();
            model_id = result.model_id;
            control_grid_shape = result.control_grid_shape;
            model_ms = result.model_ms;
            residual_warp_ms = result.residual_warp_ms;
            residual_warp_applied = result.residual_warp_applied;

            if let Some(image) = result.image {
                current = image;
                eprintln!(
                    "[Pipeline] step2:flatten(ML) → {}x{} (model={:.1}ms warp={:.1}ms)",
                    current.width(), current.height(),
                    model_ms.unwrap_or(0.0), residual_warp_ms.unwrap_or(0.0),
                );
                if let Some(ref dir) = debug_dir {
                    debug_save_image(&current, dir.join("step2_flatten.png"));
                }
            } else {
                let reason = result.fallback_reason
                    .unwrap_or_else(|| "Unknown ML pipeline error".to_string());
                eprintln!("[Pipeline] step2:flatten(ML) FAILED: {}", reason);
                return Err(reason);
            }
        } else {
            let flatten_started_at = Instant::now();
            let flatten_result = apply_local_spine_flattening(&current);
            flatten_ms = Some(flatten_started_at.elapsed().as_secs_f64() * 1000.0);
            current = flatten_result.image;
            local_flattening_applied = flatten_result.applied;
            eprintln!(
                "[Pipeline] step2:flatten(heuristic) applied={} ({:.1}ms)",
                local_flattening_applied, flatten_ms.unwrap(),
            );
        }
    }

    // ── Step 3: Enhancement ──
    if request.image_enhancement {
        let enhance_started_at = Instant::now();
        current = enhance_document_image(&current, local_flattening_applied, &request.color_mode);
        enhance_ms = Some(enhance_started_at.elapsed().as_secs_f64() * 1000.0);
        eprintln!("[Pipeline] step3:enhance mode={} ({:.1}ms)", request.color_mode, enhance_ms.unwrap());
    }

    // ── Step 4: Encode ──
    let encode_started_at = Instant::now();
    let encoded_png = encode_png(&current)?;
    let encode_ms = encode_started_at.elapsed().as_secs_f64() * 1000.0;
    let total_ms = started_at.elapsed().as_secs_f64() * 1000.0;

    eprintln!(
        "[Pipeline] done | output={}x{} | encode={:.1}ms | total={:.1}ms",
        current.width(), current.height(), encode_ms, total_ms,
    );

    let response = ScannerPostProcessResponse {
        processing_ms: started_at.elapsed().as_secs_f64() * 1000.0,
        decode_ms,
        refine_ms: None,
        perspective_ms,
        flatten_ms,
        enhance_ms,
        model_ms,
        residual_warp_ms,
        rotate_ms,
        encode_ms,
        input_width,
        input_height,
        output_width: current.width(),
        output_height: current.height(),
        encoded_mime_type: "image/png",
        postprocess_backend: postprocess_backend.as_str().to_string(),
        model_id,
        control_grid_shape,
        effective_document_points: effective_document_points.map(|points| {
            if request.output_rotation == 0 {
                points.to_vec()
            } else {
                // Reverse the entry transform: un-reorder then un-transform.
                let inverse_rotation = (360 - request.output_rotation) % 360;
                let unreordered = reorder_points_after_rotation(points, inverse_rotation);
                reverse_transform_points_for_rotation(
                    &unreordered, pre_rotation_width, pre_rotation_height,
                    request.output_rotation,
                ).to_vec()
            }
        }),
        refinement_applied: false,
        local_flattening_applied,
        residual_warp_applied,
        residual_warp_fallback_reason,
    };

    Ok((response, encoded_png))
}

fn attempt_residual_control_point_stage(
    resource_dir_hint: Option<std::path::PathBuf>,
    app_config_dir_hint: Option<std::path::PathBuf>,
    proxy_image: &RgbaImage,
    target_size: Option<(u32, u32)>,
    grid_postprocess: &str,
) -> ResidualWarpAttempt {
    match run_native_postprocess_model_with_runtime_hints(
        resource_dir_hint.clone(),
        app_config_dir_hint.clone(),
        proxy_image,
        target_size,
        grid_postprocess,
    ) {
        Ok(result) => ResidualWarpAttempt {
            backend_used: ScannerPostProcessBackend::NativeMlV1,
            model_id: Some(result.model_id),
            control_grid_shape: result.control_grid_shape,
            model_ms: Some(result.model_ms),
            residual_warp_ms: Some(result.residual_warp_ms),
            residual_warp_applied: true,
            fallback_reason: None,
            image: Some(result.image),
        },
        Err(error) => {
            let status = describe_native_postprocess_model_with_runtime_hints(
                resource_dir_hint,
                app_config_dir_hint,
            );
            ResidualWarpAttempt {
                backend_used: ScannerPostProcessBackend::Heuristic,
                model_id: status.model_id,
                control_grid_shape: status.control_grid_shape,
                model_ms: None,
                residual_warp_ms: None,
                residual_warp_applied: false,
                fallback_reason: Some(error),
                image: None,
            }
        }
    }
}

fn normalize_document_points(
    points: Option<&[ScannerPoint]>,
) -> Result<Option<[ScannerPoint; 4]>, String> {
    let Some(points) = points else {
        return Ok(None);
    };

    if points.len() != 4 {
        return Err(format!(
            "Perspective transform requires exactly 4 points, got {}.",
            points.len()
        ));
    }

    let mut ordered = *<&[ScannerPoint; 4]>::try_from(points)
        .map_err(|_| "Failed to normalize document points.".to_string())?;
    ordered = order_points(ordered);
    Ok(Some(ordered))
}

fn order_points(points: [ScannerPoint; 4]) -> [ScannerPoint; 4] {
    let mut sums = [0.0f32; 4];
    let mut diffs = [0.0f32; 4];

    for (index, point) in points.iter().enumerate() {
        sums[index] = point.x + point.y;
        diffs[index] = point.y - point.x;
    }

    [
        points[index_of_min(&sums)],
        points[index_of_min(&diffs)],
        points[index_of_max(&sums)],
        points[index_of_max(&diffs)],
    ]
}

#[derive(Debug, Clone, Copy)]
struct LineFit {
    point: ScannerPoint,
    direction: (f32, f32),
}

const REFINE_SAMPLE_MARGIN_RATIO: f32 = 0.12;
const REFINE_SAMPLE_SPACING_PX: f32 = 18.0;
const REFINE_MIN_SAMPLE_COUNT: usize = 8;
const REFINE_MAX_SAMPLE_COUNT: usize = 32;
const REFINE_SEARCH_RADIUS_RATIO: f32 = 0.08;
const REFINE_MIN_SEARCH_RADIUS_PX: f32 = 4.0;
const REFINE_MAX_SEARCH_RADIUS_PX: f32 = 160.0;
const REFINE_MAX_CORNER_SHIFT_RATIO: f32 = 0.10;
const REFINE_MIN_CORNER_SHIFT_PX: f32 = 8.0;
const REFINE_MAX_CORNER_SHIFT_PX: f32 = 220.0;

const FLATTEN_DETECTION_BAND_RATIO: f32 = 0.35;
const FLATTEN_APPLY_BAND_RATIO: f32 = 0.35;
const FLATTEN_MIN_BAND_PX: usize = 24;
const FLATTEN_MAX_BAND_RATIO: f32 = 0.50;
const FLATTEN_MIN_VALID_ROWS_RATIO: f32 = 0.12;
const FLATTEN_MIN_MEAN_SHIFT_PX: f32 = 2.0;
const FLATTEN_MIN_MAX_SHIFT_PX: f32 = 4.0;
const FLATTEN_MAX_SHIFT_PX: f32 = 40.0;
const FLATTEN_SMOOTHING_RADIUS: usize = 6;
const FLATTEN_EDGE_ANCHOR_PX: f32 = 16.0;
const FLATTEN_MIN_EDGE_ANCHOR_RATIO: f32 = 0.15;
const FLATTEN_DOMINANT_EDGE_ANCHOR_RATIO: f32 = 1.35;
const FLATTEN_DOMINANT_SCORE_RATIO: f32 = 1.15;
const FLATTEN_GRADIENT_WINDOW: usize = 5;
const PAPER_CROP_MIN_COMPONENT_AREA_RATIO: f32 = 0.12;
#[allow(dead_code)] // Was used by removed "auto" enhancement mode.
const PAPER_CROP_MIN_SCORE_THRESHOLD: u8 = 160;
const PAPER_SCORE_BLUR_SIGMA: f32 = 6.0;
const PAPER_SCORE_LOCAL_CONTRAST_WEIGHT: f32 = 1.15;
#[allow(dead_code)] // Was used by removed "auto" enhancement mode.
const ENHANCE_MIN_PAPER_BBOX_RATIO: f32 = 0.55;


#[derive(Debug, Clone)]
struct FlattenCandidate {
    side: FlattenSide,
    row_shifts: Vec<f32>,
    score: f32,
    max_shift: f32,
    evidence_max_shift: f32,
    edge_anchor_ratio: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlattenSide {
    Left,
    Right,
}

#[derive(Debug)]
struct FlattenResult {
    image: RgbaImage,
    applied: bool,
}



#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct BoundingBox {
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
}

#[allow(dead_code)]
impl BoundingBox {
    fn width(&self) -> u32 {
        self.max_x.saturating_sub(self.min_x).saturating_add(1)
    }

    fn height(&self) -> u32 {
        self.max_y.saturating_sub(self.min_y).saturating_add(1)
    }
}

fn refine_document_points(
    source: &RgbaImage,
    coarse_points: &[ScannerPoint; 4],
) -> [ScannerPoint; 4] {
    let gray = rgba_to_gray(source);
    let side_lengths = [
        point_distance(coarse_points[0], coarse_points[1]),
        point_distance(coarse_points[1], coarse_points[2]),
        point_distance(coarse_points[2], coarse_points[3]),
        point_distance(coarse_points[3], coarse_points[0]),
    ];
    let shortest_side = side_lengths
        .into_iter()
        .filter(|length| length.is_finite() && *length > 0.0)
        .fold(f32::INFINITY, f32::min);
    if !shortest_side.is_finite() {
        return *coarse_points;
    }

    let search_radius = (shortest_side * REFINE_SEARCH_RADIUS_RATIO)
        .clamp(REFINE_MIN_SEARCH_RADIUS_PX, REFINE_MAX_SEARCH_RADIUS_PX)
        .round() as i32;
    let max_corner_shift = (shortest_side * REFINE_MAX_CORNER_SHIFT_RATIO)
        .clamp(REFINE_MIN_CORNER_SHIFT_PX, REFINE_MAX_CORNER_SHIFT_PX);

    let edges = [
        (coarse_points[0], coarse_points[1]),
        (coarse_points[1], coarse_points[2]),
        (coarse_points[2], coarse_points[3]),
        (coarse_points[3], coarse_points[0]),
    ];
    let fitted_lines = edges.map(|(start, end)| {
        let samples = collect_refined_edge_samples(&gray, start, end, search_radius);
        fit_line_to_points(&samples)
    });

    let refined_candidates = [
        fitted_lines[0]
            .zip(fitted_lines[3])
            .and_then(|(first, second)| intersect_lines(first, second)),
        fitted_lines[0]
            .zip(fitted_lines[1])
            .and_then(|(first, second)| intersect_lines(first, second)),
        fitted_lines[1]
            .zip(fitted_lines[2])
            .and_then(|(first, second)| intersect_lines(first, second)),
        fitted_lines[2]
            .zip(fitted_lines[3])
            .and_then(|(first, second)| intersect_lines(first, second)),
    ];

    let centroid = ScannerPoint {
        x: coarse_points.iter().map(|p| p.x).sum::<f32>() / 4.0,
        y: coarse_points.iter().map(|p| p.y).sum::<f32>() / 4.0,
    };
    let image_boundary_margin = shortest_side * 0.04;

    let mut limited_points = *coarse_points;
    for (index, candidate) in refined_candidates.into_iter().enumerate() {
        if let Some(candidate_point) = candidate {
            let coarse = coarse_points[index];
            let near_boundary = coarse.x < image_boundary_margin
                || coarse.y < image_boundary_margin
                || coarse.x > source.width() as f32 - image_boundary_margin
                || coarse.y > source.height() as f32 - image_boundary_margin;

            if near_boundary {
                let coarse_to_center =
                    ((centroid.x - coarse.x).powi(2) + (centroid.y - coarse.y).powi(2)).sqrt();
                let candidate_to_center = ((centroid.x - candidate_point.x).powi(2)
                    + (centroid.y - candidate_point.y).powi(2))
                .sqrt();
                if candidate_to_center < coarse_to_center {
                    // Corner is near image edge and refinement wants to pull it inward —
                    // this edge is likely the real page boundary (e.g. spine side).
                    // Skip refinement entirely for this corner.
                    continue;
                }
            }

            let limited = limit_point_shift(candidate_point, coarse, max_corner_shift);
            limited_points[index] =
                clamp_point_to_image_bounds(limited, source.width(), source.height());
        }
    }

    let coarse_area = polygon_area(coarse_points);
    let refined_area = polygon_area(&limited_points);
    if !refined_area.is_finite()
        || refined_area <= 0.0
        || refined_area < coarse_area * 0.5
        || refined_area > coarse_area * 1.5
    {
        return *coarse_points;
    }

    limited_points
}

fn collect_refined_edge_samples(
    gray: &GrayImage,
    start: ScannerPoint,
    end: ScannerPoint,
    search_radius: i32,
) -> Vec<ScannerPoint> {
    let edge_length = point_distance(start, end);
    if !edge_length.is_finite() || edge_length < 1.0 {
        return Vec::new();
    }

    let tangent_x = (end.x - start.x) / edge_length;
    let tangent_y = (end.y - start.y) / edge_length;
    let normal_x = -tangent_y;
    let normal_y = tangent_x;
    let sample_count = ((edge_length / REFINE_SAMPLE_SPACING_PX).round() as usize)
        .clamp(REFINE_MIN_SAMPLE_COUNT, REFINE_MAX_SAMPLE_COUNT);
    let mut samples: Vec<(ScannerPoint, f32)> = Vec::with_capacity(sample_count);

    for index in 0..sample_count {
        let progress = if sample_count <= 1 {
            0.5
        } else {
            REFINE_SAMPLE_MARGIN_RATIO
                + (((1.0 - (REFINE_SAMPLE_MARGIN_RATIO * 2.0)) * index as f32)
                    / (sample_count - 1) as f32)
        };
        let center_x = start.x + ((end.x - start.x) * progress);
        let center_y = start.y + ((end.y - start.y) * progress);

        let mut best_strength = f32::NEG_INFINITY;
        let mut best_point = None;
        for offset in -search_radius..=search_radius {
            let strength = compute_normal_edge_strength(
                gray,
                center_x,
                center_y,
                normal_x,
                normal_y,
                offset as f32,
            );
            if let Some(strength_value) = strength {
                if strength_value > best_strength {
                    best_strength = strength_value;
                    best_point = Some(ScannerPoint {
                        x: center_x + (offset as f32 * normal_x),
                        y: center_y + (offset as f32 * normal_y),
                    });
                }
            }
        }

        if let Some(point) = best_point {
            samples.push((point, best_strength));
        }
    }

    if samples.len() < 2 {
        return Vec::new();
    }

    let average_strength =
        samples.iter().map(|(_, strength)| *strength).sum::<f32>() / samples.len() as f32;
    let threshold = average_strength * 0.6;
    let filtered: Vec<ScannerPoint> = samples
        .iter()
        .filter(|(_, strength)| *strength >= threshold)
        .map(|(point, _)| *point)
        .collect();

    if filtered.len() >= 2 {
        filtered
    } else {
        samples.into_iter().map(|(point, _)| point).collect()
    }
}

fn compute_normal_edge_strength(
    gray: &GrayImage,
    center_x: f32,
    center_y: f32,
    normal_x: f32,
    normal_y: f32,
    offset: f32,
) -> Option<f32> {
    let outside_near = sample_gray(
        gray,
        center_x + ((offset - 1.0) * normal_x),
        center_y + ((offset - 1.0) * normal_y),
    )?;
    let outside_far = sample_gray(
        gray,
        center_x + ((offset - 2.0) * normal_x),
        center_y + ((offset - 2.0) * normal_y),
    )?;
    let inside_near = sample_gray(
        gray,
        center_x + ((offset + 1.0) * normal_x),
        center_y + ((offset + 1.0) * normal_y),
    )?;
    let inside_far = sample_gray(
        gray,
        center_x + ((offset + 2.0) * normal_x),
        center_y + ((offset + 2.0) * normal_y),
    )?;

    let outside = (outside_near + outside_far) * 0.5;
    let inside = (inside_near + inside_far) * 0.5;
    // Use absolute difference so that both light-on-dark and dark-on-light
    // document edges produce a positive edge strength signal.
    Some((inside - outside).abs())
}

/// Convert an RGBA image to grayscale without cloning the source.
fn rgba_to_gray(source: &RgbaImage) -> GrayImage {
    GrayImage::from_fn(source.width(), source.height(), |x, y| {
        let p = source.get_pixel(x, y).0;
        Luma([(0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u8])
    })
}

fn sample_gray(gray: &GrayImage, x: f32, y: f32) -> Option<f32> {
    if x < 0.0 || y < 0.0 || x > gray.width() as f32 - 1.0 || y > gray.height() as f32 - 1.0 {
        return None;
    }

    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = x0.saturating_add(1).min(gray.width().saturating_sub(1));
    let y1 = y0.saturating_add(1).min(gray.height().saturating_sub(1));
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;

    let top_left = gray.get_pixel(x0, y0).0[0] as f32;
    let top_right = gray.get_pixel(x1, y0).0[0] as f32;
    let bottom_left = gray.get_pixel(x0, y1).0[0] as f32;
    let bottom_right = gray.get_pixel(x1, y1).0[0] as f32;

    let top = top_left + ((top_right - top_left) * tx);
    let bottom = bottom_left + ((bottom_right - bottom_left) * tx);
    Some(top + ((bottom - top) * ty))
}

fn fit_line_to_points(points: &[ScannerPoint]) -> Option<LineFit> {
    if points.len() < 2 {
        return None;
    }

    let centroid = ScannerPoint {
        x: points.iter().map(|point| point.x).sum::<f32>() / points.len() as f32,
        y: points.iter().map(|point| point.y).sum::<f32>() / points.len() as f32,
    };

    let mut covariance_xx = 0.0;
    let mut covariance_xy = 0.0;
    let mut covariance_yy = 0.0;
    for point in points {
        let delta_x = point.x - centroid.x;
        let delta_y = point.y - centroid.y;
        covariance_xx += delta_x * delta_x;
        covariance_xy += delta_x * delta_y;
        covariance_yy += delta_y * delta_y;
    }

    let angle = 0.5 * (2.0 * covariance_xy).atan2(covariance_xx - covariance_yy);
    let direction = (angle.cos(), angle.sin());
    if !direction.0.is_finite() || !direction.1.is_finite() {
        return None;
    }

    Some(LineFit {
        point: centroid,
        direction,
    })
}

fn intersect_lines(first: LineFit, second: LineFit) -> Option<ScannerPoint> {
    let denominator =
        (first.direction.0 * second.direction.1) - (first.direction.1 * second.direction.0);
    if denominator.abs() < 1e-3 {
        return None;
    }

    let delta_x = second.point.x - first.point.x;
    let delta_y = second.point.y - first.point.y;
    let distance_along_first =
        ((delta_x * second.direction.1) - (delta_y * second.direction.0)) / denominator;

    Some(ScannerPoint {
        x: first.point.x + (distance_along_first * first.direction.0),
        y: first.point.y + (distance_along_first * first.direction.1),
    })
}

fn limit_point_shift(
    candidate: ScannerPoint,
    fallback: ScannerPoint,
    max_shift: f32,
) -> ScannerPoint {
    let delta_x = candidate.x - fallback.x;
    let delta_y = candidate.y - fallback.y;
    let distance = (delta_x * delta_x + delta_y * delta_y).sqrt();
    if !distance.is_finite() || distance <= max_shift {
        return candidate;
    }

    let scale = max_shift / distance;
    ScannerPoint {
        x: fallback.x + (delta_x * scale),
        y: fallback.y + (delta_y * scale),
    }
}

fn clamp_point_to_image_bounds(point: ScannerPoint, width: u32, height: u32) -> ScannerPoint {
    ScannerPoint {
        x: point.x.clamp(0.0, width.saturating_sub(1) as f32),
        y: point.y.clamp(0.0, height.saturating_sub(1) as f32),
    }
}

fn polygon_area(points: &[ScannerPoint; 4]) -> f32 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        area += (current.x * next.y) - (next.x * current.y);
    }

    area.abs() * 0.5
}

const QUAD_MIN_EDGE_RATIO: f32 = 0.25;
const QUAD_MIN_AREA_RATIO: f32 = 0.05;

fn validate_quad_geometry(
    points: &[ScannerPoint; 4],
    image_width: u32,
    image_height: u32,
) -> Result<(), String> {
    let [tl, tr, br, bl] = points;

    let top = point_distance(*tl, *tr);
    let right = point_distance(*tr, *br);
    let bottom = point_distance(*bl, *br);
    let left = point_distance(*tl, *bl);

    if !top.is_finite() || !right.is_finite() || !bottom.is_finite() || !left.is_finite() {
        return Err("Document quad has non-finite edge lengths.".to_string());
    }

    let horizontal_ratio = top.min(bottom) / top.max(bottom).max(1.0);
    if horizontal_ratio < QUAD_MIN_EDGE_RATIO {
        return Err(format!(
            "Document quad is degenerate: horizontal edge ratio {horizontal_ratio:.3} < {QUAD_MIN_EDGE_RATIO} \
             (top={top:.0}, bottom={bottom:.0})."
        ));
    }

    let vertical_ratio = left.min(right) / left.max(right).max(1.0);
    if vertical_ratio < QUAD_MIN_EDGE_RATIO {
        return Err(format!(
            "Document quad is degenerate: vertical edge ratio {vertical_ratio:.3} < {QUAD_MIN_EDGE_RATIO} \
             (left={left:.0}, right={right:.0})."
        ));
    }

    let quad_area = polygon_area(points);
    let image_area = image_width as f32 * image_height as f32;
    let area_ratio = quad_area / image_area.max(1.0);
    if area_ratio < QUAD_MIN_AREA_RATIO {
        return Err(format!(
            "Document quad is too small: area ratio {area_ratio:.3} < {QUAD_MIN_AREA_RATIO}."
        ));
    }

    let edges = [
        (tr.x - tl.x, tr.y - tl.y),
        (br.x - tr.x, br.y - tr.y),
        (bl.x - br.x, bl.y - br.y),
        (tl.x - bl.x, tl.y - bl.y),
    ];
    let mut positive = 0u32;
    let mut negative = 0u32;
    for i in 0..4 {
        let (ax, ay) = edges[i];
        let (bx, by) = edges[(i + 1) % 4];
        let cross = ax * by - ay * bx;
        if cross > 0.0 {
            positive += 1;
        } else if cross < 0.0 {
            negative += 1;
        }
    }
    if positive > 0 && negative > 0 {
        return Err("Document quad is non-convex or self-intersecting.".to_string());
    }

    Ok(())
}



fn apply_local_spine_flattening(source: &RgbaImage) -> FlattenResult {
    let Some(content_bbox) = alpha_content_bbox(source) else {
        return FlattenResult {
            image: source.clone(),
            applied: false,
        };
    };

    let uses_trimmed_region = content_bbox.min_x > 0
        || content_bbox.min_y > 0
        || content_bbox.max_x + 1 < source.width()
        || content_bbox.max_y + 1 < source.height();
    if !uses_trimmed_region {
        return apply_local_spine_flattening_region(source);
    }

    let trimmed = image::imageops::crop_imm(
        source,
        content_bbox.min_x,
        content_bbox.min_y,
        content_bbox.width(),
        content_bbox.height(),
    )
    .to_image();
    let flattened = apply_local_spine_flattening_region(&trimmed);
    if !flattened.applied {
        return FlattenResult {
            image: source.clone(),
            applied: false,
        };
    }

    let mut composited = source.clone();
    for y in 0..flattened.image.height() {
        for x in 0..flattened.image.width() {
            composited.put_pixel(
                content_bbox.min_x + x,
                content_bbox.min_y + y,
                *flattened.image.get_pixel(x, y),
            );
        }
    }

    FlattenResult {
        image: composited,
        applied: true,
    }
}



fn apply_local_spine_flattening_region(source: &RgbaImage) -> FlattenResult {
    if source.width() < 48 || source.height() < 48 {
        return FlattenResult {
            image: source.clone(),
            applied: false,
        };
    }

    let gray = rgba_to_gray(source);
    let band_width = ((source.width() as f32 * FLATTEN_DETECTION_BAND_RATIO).round() as usize)
        .clamp(
            FLATTEN_MIN_BAND_PX,
            (source.width() as f32 * 0.33)
                .floor()
                .max(FLATTEN_MIN_BAND_PX as f32) as usize,
        );
    let candidates = [
        build_flatten_candidate(&gray, FlattenSide::Left, band_width),
        build_flatten_candidate(&gray, FlattenSide::Right, band_width),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    if candidates.is_empty() {
        return FlattenResult {
            image: source.clone(),
            applied: false,
        };
    }

    let Some(selected) = select_flatten_candidate(candidates) else {
        return FlattenResult {
            image: source.clone(),
            applied: false,
        };
    };

    FlattenResult {
        image: apply_flatten_warp(source, selected.side, &selected.row_shifts),
        applied: true,
    }
}

fn alpha_content_bbox(source: &RgbaImage) -> Option<BoundingBox> {
    let mut bbox = None::<BoundingBox>;

    for (x, y, pixel) in source.enumerate_pixels() {
        if pixel.0[3] == 0 {
            continue;
        }

        bbox = Some(match bbox {
            Some(current) => BoundingBox {
                min_x: current.min_x.min(x),
                min_y: current.min_y.min(y),
                max_x: current.max_x.max(x),
                max_y: current.max_y.max(y),
            },
            None => BoundingBox {
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
            },
        });
    }

    bbox
}

fn select_flatten_candidate(mut candidates: Vec<FlattenCandidate>) -> Option<FlattenCandidate> {
    candidates.sort_by(|first, second| {
        second
            .edge_anchor_ratio
            .partial_cmp(&first.edge_anchor_ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                second
                    .score
                    .partial_cmp(&first.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                second
                    .max_shift
                    .partial_cmp(&first.max_shift)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let selected = candidates.first()?.clone();
    if selected.evidence_max_shift < FLATTEN_MIN_MAX_SHIFT_PX {
        return None;
    }

    if let Some(runner_up) = candidates.get(1) {
        let edge_anchor_dominant = selected.edge_anchor_ratio
            >= runner_up.edge_anchor_ratio * FLATTEN_DOMINANT_EDGE_ANCHOR_RATIO;
        let score_dominant = selected.score >= runner_up.score * FLATTEN_DOMINANT_SCORE_RATIO;
        if !edge_anchor_dominant && !score_dominant {
            return None;
        }
    }

    Some(selected)
}



#[allow(dead_code)]
fn build_paper_score_image(source: &RgbaImage) -> GrayImage {
    let mut score = GrayImage::new(source.width(), source.height());
    let gray = rgba_to_gray(source);
    let blurred = gaussian_blur_f32(&gray, PAPER_SCORE_BLUR_SIGMA);

    for (x, y, pixel) in source.enumerate_pixels() {
        let [red, green, blue, _] = pixel.0;
        let max_channel = red.max(green).max(blue) as f32;
        let min_channel = red.min(green).min(blue) as f32;
        let saturation = max_channel - min_channel;
        let local_luminance = gray.get_pixel(x, y).0[0] as f32;
        let neighborhood_luminance = blurred.get_pixel(x, y).0[0] as f32;
        let local_contrast = (local_luminance - neighborhood_luminance).abs();
        let paper_score = (neighborhood_luminance
            - saturation * 0.9
            - local_contrast * PAPER_SCORE_LOCAL_CONTRAST_WEIGHT)
            .clamp(0.0, 255.0) as u8;
        score.put_pixel(x, y, Luma([paper_score]));
    }

    score
}

#[allow(dead_code)]
fn largest_paper_component_bbox(score_image: &GrayImage, threshold: u8) -> Option<BoundingBox> {
    let width = score_image.width() as usize;
    let height = score_image.height() as usize;
    let min_component_area =
        ((width * height) as f32 * PAPER_CROP_MIN_COMPONENT_AREA_RATIO).round() as usize;
    let mut visited = vec![false; width * height];
    let mut best_bbox = None;
    let mut best_score = f32::NEG_INFINITY;

    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if visited[index] || score_image.get_pixel(x as u32, y as u32).0[0] < threshold {
                continue;
            }

            let mut queue = VecDeque::from([(x as u32, y as u32)]);
            visited[index] = true;
            let mut area = 0usize;
            let mut score_sum = 0f32;
            let mut bbox = BoundingBox {
                min_x: x as u32,
                min_y: y as u32,
                max_x: x as u32,
                max_y: y as u32,
            };

            while let Some((cx, cy)) = queue.pop_front() {
                area += 1;
                score_sum += score_image.get_pixel(cx, cy).0[0] as f32;
                bbox.min_x = bbox.min_x.min(cx);
                bbox.min_y = bbox.min_y.min(cy);
                bbox.max_x = bbox.max_x.max(cx);
                bbox.max_y = bbox.max_y.max(cy);

                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }

                        let nx = cx as i32 + dx;
                        let ny = cy as i32 + dy;
                        if nx < 0
                            || ny < 0
                            || nx >= score_image.width() as i32
                            || ny >= score_image.height() as i32
                        {
                            continue;
                        }

                        let nx = nx as u32;
                        let ny = ny as u32;
                        let neighbor_index = ny as usize * width + nx as usize;
                        if visited[neighbor_index] || score_image.get_pixel(nx, ny).0[0] < threshold
                        {
                            continue;
                        }

                        visited[neighbor_index] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }

            if area < min_component_area {
                continue;
            }

            let mean_score = score_sum / area as f32 / 255.0;
            let fill_ratio = area as f32 / (bbox.width() as f32 * bbox.height() as f32).max(1.0);
            let component_score = area as f32 * (mean_score + fill_ratio);
            if component_score > best_score {
                best_score = component_score;
                best_bbox = Some(bbox);
            }
        }
    }

    best_bbox
}

fn build_flatten_candidate(
    gray: &GrayImage,
    side: FlattenSide,
    band_width: usize,
) -> Option<FlattenCandidate> {
    let offsets = (0..gray.height())
        .map(|row| detect_edge_offset_for_row(gray, row, side, band_width))
        .collect::<Vec<_>>();
    let valid_offsets = offsets.iter().flatten().copied().collect::<Vec<_>>();

    if valid_offsets.len()
        < ((gray.height() as f32 * FLATTEN_MIN_VALID_ROWS_RATIO).floor() as usize).max(6)
    {
        return None;
    }

    let baseline = percentile(&valid_offsets, 0.15);
    let edge_anchor_ratio = valid_offsets
        .iter()
        .filter(|value| **value <= FLATTEN_EDGE_ANCHOR_PX)
        .count() as f32
        / valid_offsets.len() as f32;
    if edge_anchor_ratio < FLATTEN_MIN_EDGE_ANCHOR_RATIO {
        return None;
    }
    let raw_shifts = offsets
        .iter()
        .map(|value| value.map(|value| (value - baseline).clamp(0.0, FLATTEN_MAX_SHIFT_PX)))
        .collect::<Vec<_>>();
    let filled_shifts = fill_missing_offsets(&raw_shifts, 0.0);
    let evidence_max_shift = filled_shifts.iter().copied().fold(0.0, f32::max);
    let smoothed_shifts = smooth_values(&filled_shifts, FLATTEN_SMOOTHING_RADIUS);
    let positive_shifts = smoothed_shifts
        .iter()
        .copied()
        .filter(|value| *value > 0.5)
        .collect::<Vec<_>>();

    if positive_shifts.is_empty() {
        return None;
    }

    let mean_shift = positive_shifts.iter().sum::<f32>() / positive_shifts.len() as f32;
    if mean_shift < FLATTEN_MIN_MEAN_SHIFT_PX || evidence_max_shift < FLATTEN_MIN_MAX_SHIFT_PX {
        return None;
    }
    let max_shift = positive_shifts.iter().copied().fold(0.0, f32::max);

    let coverage = positive_shifts.len() as f32 / smoothed_shifts.len() as f32;
    Some(FlattenCandidate {
        side,
        row_shifts: smoothed_shifts,
        max_shift,
        evidence_max_shift,
        score: mean_shift * (1.0 + coverage),
        edge_anchor_ratio,
    })
}

/// Detect the edge offset for a single row using gradient-based detection.
///
/// Instead of looking for absolute darkness (which fails when text darkens the
/// adaptive threshold), we look for the steepest brightness gradient within
/// the detection band. This correctly detects the book spine shadow boundary
/// regardless of the surrounding content brightness.
fn detect_edge_offset_for_row(
    gray: &GrayImage,
    row: u32,
    side: FlattenSide,
    band_width: usize,
) -> Option<f32> {
    let width = gray.width() as usize;
    let band = band_width.min(width);
    if band < FLATTEN_GRADIENT_WINDOW * 2 {
        return None;
    }

    // Compute the luminance gradient magnitude across the detection band.
    // A high gradient indicates a transition from shadow (spine) to page.
    let half_window = FLATTEN_GRADIENT_WINDOW;
    let mut best_gradient = 0.0f32;
    let mut best_offset = None;

    for offset in half_window..band.saturating_sub(half_window) {
        let x = match side {
            FlattenSide::Left => offset,
            FlattenSide::Right => width - 1 - offset,
        };

        // Sample luminance to the left and right of this position.
        let (mut sum_inner, mut sum_outer) = (0.0f32, 0.0f32);
        let mut count = 0.0f32;
        for dy in -1i32..=1 {
            let sample_y = (row as i32 + dy).clamp(0, gray.height() as i32 - 1) as u32;
            for step in 1..=half_window {
                let inner_x = match side {
                    FlattenSide::Left => (x + step).min(width - 1),
                    FlattenSide::Right => x.saturating_sub(step),
                };
                let outer_x = match side {
                    FlattenSide::Left => x.saturating_sub(step),
                    FlattenSide::Right => (x + step).min(width - 1),
                };
                sum_inner += gray.get_pixel(inner_x as u32, sample_y).0[0] as f32;
                sum_outer += gray.get_pixel(outer_x as u32, sample_y).0[0] as f32;
                count += 1.0;
            }
        }

        if count <= 0.0 {
            continue;
        }

        // Gradient: inner (page side) should be brighter than outer (spine/edge side).
        // Use signed gradient so we detect the transition direction correctly.
        let gradient = (sum_inner - sum_outer) / count;
        if gradient > best_gradient {
            best_gradient = gradient;
            best_offset = Some(offset as f32);
        }
    }

    // Require a minimum gradient to avoid noise.
    if best_gradient < 8.0 {
        return None;
    }

    best_offset
}

fn percentile(values: &[f32], ratio: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|first, second| {
        first
            .partial_cmp(second)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let index = ((sorted.len() - 1) as f32 * ratio)
        .floor()
        .clamp(0.0, (sorted.len() - 1) as f32) as usize;
    sorted[index]
}

fn fill_missing_offsets(values: &[Option<f32>], fallback: f32) -> Vec<f32> {
    let mut result = values
        .iter()
        .map(|value| value.unwrap_or(f32::NAN))
        .collect::<Vec<_>>();
    let mut last_known_index = None;

    for index in 0..result.len() {
        if !result[index].is_nan() {
            if let Some(previous_index) = last_known_index {
                if index > previous_index + 1 {
                    let start_value = result[previous_index];
                    let end_value = result[index];
                    let gap = (index - previous_index) as f32;
                    for fill_index in (previous_index + 1)..index {
                        let progress = (fill_index - previous_index) as f32 / gap;
                        result[fill_index] = start_value + ((end_value - start_value) * progress);
                    }
                }
            } else {
                for fill_index in 0..index {
                    result[fill_index] = result[index];
                }
            }
            last_known_index = Some(index);
        }
    }

    if let Some(last_known_index) = last_known_index {
        for index in (last_known_index + 1)..result.len() {
            result[index] = result[last_known_index];
        }
    }

    result
        .into_iter()
        .map(|value| if value.is_nan() { fallback } else { value })
        .collect()
}

fn smooth_values(values: &[f32], radius: usize) -> Vec<f32> {
    (0..values.len())
        .map(|index| {
            let start = index.saturating_sub(radius);
            let end = (index + radius).min(values.len().saturating_sub(1));
            let window = &values[start..=end];
            window.iter().sum::<f32>() / window.len() as f32
        })
        .collect()
}

fn apply_flatten_warp(source: &RgbaImage, side: FlattenSide, row_shifts: &[f32]) -> RgbaImage {
    let apply_band_width = ((source.width() as f32 * FLATTEN_APPLY_BAND_RATIO).round() as usize)
        .clamp(
            ((source.width() as f32 * 0.12).floor() as usize).max(FLATTEN_MIN_BAND_PX),
            ((source.width() as f32 * FLATTEN_MAX_BAND_RATIO).floor() as usize)
                .max(FLATTEN_MIN_BAND_PX),
        )
        .max(1);
    let mut output = RgbaImage::new(source.width(), source.height());

    for y in 0..source.height() as usize {
        let shift = row_shifts.get(y).copied().unwrap_or(0.0);
        for x in 0..source.width() as usize {
            let distance_into_band = match side {
                FlattenSide::Left => x as f32,
                FlattenSide::Right => (source.width() as usize - 1 - x) as f32,
            };
            let weight = (1.0 - (distance_into_band / apply_band_width as f32)).clamp(0.0, 1.0);
            let eased_weight = weight * weight;
            let source_x = match side {
                FlattenSide::Left => x as f32 + (shift * eased_weight),
                FlattenSide::Right => x as f32 - (shift * eased_weight),
            };
            output.put_pixel(
                x as u32,
                y as u32,
                sample_rgba_bilinear(source, source_x, y as f32),
            );
        }
    }

    output
}


fn sample_rgba_bilinear(source: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    let clamped_x = x.clamp(0.0, source.width().saturating_sub(1) as f32);
    let clamped_y = y.clamp(0.0, source.height().saturating_sub(1) as f32);
    let x0 = clamped_x.floor() as u32;
    let y0 = clamped_y.floor() as u32;
    let x1 = x0.saturating_add(1).min(source.width().saturating_sub(1));
    let y1 = y0.saturating_add(1).min(source.height().saturating_sub(1));
    let tx = clamped_x - x0 as f32;
    let ty = clamped_y - y0 as f32;

    let top_left = source.get_pixel(x0, y0).0;
    let top_right = source.get_pixel(x1, y0).0;
    let bottom_left = source.get_pixel(x0, y1).0;
    let bottom_right = source.get_pixel(x1, y1).0;
    let mut output = [0u8; 4];

    for channel in 0..4 {
        let top = top_left[channel] as f32
            + ((top_right[channel] as f32 - top_left[channel] as f32) * tx);
        let bottom = bottom_left[channel] as f32
            + ((bottom_right[channel] as f32 - bottom_left[channel] as f32) * tx);
        output[channel] = (top + ((bottom - top) * ty)).round().clamp(0.0, 255.0) as u8;
    }

    Rgba(output)
}

fn index_of_min(values: &[f32; 4]) -> usize {
    let mut best_index = 0usize;
    for index in 1..values.len() {
        if values[index] < values[best_index] {
            best_index = index;
        }
    }
    best_index
}

fn index_of_max(values: &[f32; 4]) -> usize {
    let mut best_index = 0usize;
    for index in 1..values.len() {
        if values[index] > values[best_index] {
            best_index = index;
        }
    }
    best_index
}

fn compute_document_projection(
    points: &[ScannerPoint; 4],
    input_width: f32,
    input_height: f32,
) -> Result<(u32, u32, Projection, [(f32, f32); 4], [(f32, f32); 4]), String> {
    let [tl, tr, br, bl] = points;

    // ── Edge Length Equalization ──
    //
    // When opposite vertical edges have different lengths (h_left ≠ h_right),
    // the homography creates non-uniform scale across the output width.
    // Jacobian analysis shows this produces up to 35% h_scale variation and
    // 15% aspect ratio distortion between the free and spine sides.
    //
    // Root cause: for a curved book page, the spine edge appears shorter
    // (or longer, depending on camera position) than the free edge.  The
    // homography treats this length difference as perspective and "corrects"
    // it, creating differential scaling → visible text distortion.
    //
    // Fix: extend the shorter vertical edge along its direction to match
    // the longer edge.  This makes the quad closer to a parallelogram →
    // the homography becomes more affine-like → constant Jacobian →
    // uniform scale across the entire output.
    //
    // Proven: h_scale variation → 1.0000 for all tested cases (9 scenarios).

    let h_left = point_distance(*tl, *bl);
    let h_right = point_distance(*tr, *br);
    let h_max = h_left.max(h_right);
    let h_diff_pct = (h_left - h_right).abs() / h_max * 100.0;

    eprintln!(
        "[Perspective] corners: TL=({:.0},{:.0}) TR=({:.0},{:.0}) BR=({:.0},{:.0}) BL=({:.0},{:.0})",
        tl.x, tl.y, tr.x, tr.y, br.x, br.y, bl.x, bl.y,
    );
    eprintln!(
        "[Perspective] edges: h_left={:.0} h_right={:.0} diff={:.1}%",
        h_left, h_right, h_diff_pct,
    );

    // Only adjust if the length difference exceeds 5%.  Below that, the
    // scale variation is < 3% (visually negligible).
    let (adj_tl, adj_tr, adj_br, adj_bl) = if h_diff_pct > 5.0 {
        if h_right < h_left {
            // Right edge is shorter — extend it to match h_left.
            let dx = br.x - tr.x;
            let dy = br.y - tr.y;
            let len = h_right;
            let ux = dx / len;
            let uy = dy / len;
            let mid_x = (tr.x + br.x) / 2.0;
            let mid_y = (tr.y + br.y) / 2.0;
            let half = h_left / 2.0;
            let new_tr = ScannerPoint {
                x: (mid_x - half * ux).clamp(0.0, input_width - 1.0),
                y: (mid_y - half * uy).clamp(0.0, input_height - 1.0),
            };
            let new_br = ScannerPoint {
                x: (mid_x + half * ux).clamp(0.0, input_width - 1.0),
                y: (mid_y + half * uy).clamp(0.0, input_height - 1.0),
            };
            eprintln!(
                "[Perspective] right edge shorter by {:.1}% → extended: TR=({:.0},{:.0}) BR=({:.0},{:.0})",
                h_diff_pct, new_tr.x, new_tr.y, new_br.x, new_br.y,
            );
            (*tl, new_tr, new_br, *bl)
        } else {
            // Left edge is shorter — extend it to match h_right.
            let dx = bl.x - tl.x;
            let dy = bl.y - tl.y;
            let len = h_left;
            let ux = dx / len;
            let uy = dy / len;
            let mid_x = (tl.x + bl.x) / 2.0;
            let mid_y = (tl.y + bl.y) / 2.0;
            let half = h_right / 2.0;
            let new_tl = ScannerPoint {
                x: (mid_x - half * ux).clamp(0.0, input_width - 1.0),
                y: (mid_y - half * uy).clamp(0.0, input_height - 1.0),
            };
            let new_bl = ScannerPoint {
                x: (mid_x + half * ux).clamp(0.0, input_width - 1.0),
                y: (mid_y + half * uy).clamp(0.0, input_height - 1.0),
            };
            eprintln!(
                "[Perspective] left edge shorter by {:.1}% → extended: TL=({:.0},{:.0}) BL=({:.0},{:.0})",
                h_diff_pct, new_tl.x, new_tl.y, new_bl.x, new_bl.y,
            );
            (new_tl, *tr, *br, new_bl)
        }
    } else {
        eprintln!("[Perspective] edge diff={:.1}% < 5% → no adjustment", h_diff_pct);
        (*tl, *tr, *br, *bl)
    };

    // Compute output dimensions from the ADJUSTED corners.
    let adj_points = [adj_tl, adj_tr, adj_br, adj_bl];
    let (target_width, target_height) =
        compute_true_aspect_ratio_dimensions(&adj_points, input_width, input_height);

    let from = [(adj_tl.x, adj_tl.y), (adj_tr.x, adj_tr.y), (adj_br.x, adj_br.y), (adj_bl.x, adj_bl.y)];
    let to = [
        (0.0, 0.0),
        (target_width.saturating_sub(1) as f32, 0.0),
        (
            target_width.saturating_sub(1) as f32,
            target_height.saturating_sub(1) as f32,
        ),
        (0.0, target_height.saturating_sub(1) as f32),
    ];

    let projection = Projection::from_control_points(from, to).ok_or_else(|| {
        "Failed to build perspective projection from document points.".to_string()
    })?;

    Ok((target_width, target_height, projection, from, to))
}

fn compute_true_aspect_ratio_dimensions(
    points: &[ScannerPoint; 4],
    _image_width: f32,
    _image_height: f32,
) -> (u32, u32) {
    let [tl, tr, br, bl] = points;
    let w_top = point_distance(*tl, *tr);
    let w_bot = point_distance(*bl, *br);
    let h_left = point_distance(*tl, *bl);
    let h_right = point_distance(*tr, *br);

    // Use average of opposing edge lengths as the output dimensions.
    // Average is robust for moderate viewing angles and never flips
    // portrait ↔ landscape (unlike the old vanishing-point method).
    let out_w = ((w_top + w_bot) / 2.0).round().max(1.0) as u32;
    let out_h = ((h_left + h_right) / 2.0).round().max(1.0) as u32;

    eprintln!(
        "[Perspective] edges: w_top={:.0} w_bot={:.0} h_left={:.0} h_right={:.0} → output={}x{}",
        w_top, w_bot, h_left, h_right, out_w, out_h,
    );

    (out_w, out_h)
}



fn point_distance(a: ScannerPoint, b: ScannerPoint) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

fn enhance_document_image(
    source: &RgbaImage,
    _prefer_soft_tone: bool,
    color_mode: &str,
) -> RgbaImage {
    // "none" or unrecognized → return source unchanged.
    let effective_mode = match color_mode {
        "normalize" | "color" => "normalize",
        "grayscale" => "grayscale",
        "binary" => "binary",
        _ => return source.clone(),
    };

    let gray = rgba_to_gray(source);
    let background_sigma = compute_background_sigma(source.width(), source.height());
    let background = gaussian_blur_f32(&gray, background_sigma);

    match effective_mode {
        "normalize" => {
            // Preserve color: flatten background luminance per-channel.
            enhance_preserve_color(source, &background)
        }
        "binary" => {
            let flattened = flatten_background(&gray, &background);
            let denoised = gaussian_blur_f32(&flattened, 0.8);
            let normalized = normalize_gray(&denoised);
            let threshold = otsu_level(&normalized);
            let binary = threshold_to_binary(&normalized, threshold);
            let selected = if is_reasonable_binary_candidate(&binary) {
                binary
            } else {
                normalized
            };
            gray_to_rgba(&selected)
        }
        _ => {
            // "grayscale"
            let flattened = flatten_background(&gray, &background);
            let denoised = gaussian_blur_f32(&flattened, 0.8);
            let normalized = normalize_gray(&denoised);
            gray_to_rgba(&normalized)
        }
    }
}

/// Enhance while preserving original colors.
/// Uses the background luminance estimate to normalize per-pixel brightness,
/// then scales each color channel proportionally to maintain hue/saturation.
fn enhance_preserve_color(source: &RgbaImage, background: &GrayImage) -> RgbaImage {
    let mut output = RgbaImage::new(source.width(), source.height());
    // Find the overall background gain for normalization.
    let target_white = 240.0f32;

    for (x, y, pixel) in source.enumerate_pixels() {
        let [red, green, blue, alpha] = pixel.0;
        let bg = (background.get_pixel(x, y).0[0] as f32).max(1.0);
        // Scale factor: how much to brighten this pixel to normalize the background.
        let gain = target_white / bg;
        let out_r = ((red as f32) * gain).clamp(0.0, 255.0) as u8;
        let out_g = ((green as f32) * gain).clamp(0.0, 255.0) as u8;
        let out_b = ((blue as f32) * gain).clamp(0.0, 255.0) as u8;
        output.put_pixel(x, y, Rgba([out_r, out_g, out_b, alpha]));
    }

    output
}

#[allow(dead_code)] // Was used by removed "auto" enhancement mode.
fn should_prefer_soft_tone(source: &RgbaImage, prefer_soft_tone: bool) -> bool {
    if prefer_soft_tone {
        return true;
    }

    let Some(paper_bbox_ratio) = measure_paper_bbox_ratio(source) else {
        return true;
    };

    paper_bbox_ratio < ENHANCE_MIN_PAPER_BBOX_RATIO
}

#[allow(dead_code)] // Was used by removed "auto" enhancement mode.
fn measure_paper_bbox_ratio(source: &RgbaImage) -> Option<f32> {
    let score_image = build_paper_score_image(source);
    let threshold = otsu_level(&score_image).max(PAPER_CROP_MIN_SCORE_THRESHOLD);
    let bounding_box = largest_paper_component_bbox(&score_image, threshold)?;
    let image_area = (source.width() as f32 * source.height() as f32).max(1.0);
    let bbox_area = bounding_box.width() as f32 * bounding_box.height() as f32;
    Some(bbox_area / image_area)
}

fn compute_background_sigma(width: u32, height: u32) -> f32 {
    let shortest_side = width.min(height) as f32;
    // The sigma must be large enough to smooth out text strokes while keeping
    // page-level illumination gradients intact.  The old upper bound of 18px
    // was far too small for images larger than ~500px on the short side,
    // causing visible halo artifacts around dense text regions.
    (shortest_side * 0.08).clamp(5.0, 80.0)
}

fn flatten_background(gray: &GrayImage, background: &GrayImage) -> GrayImage {
    let mut output = GrayImage::new(gray.width(), gray.height());

    for y in 0..gray.height() {
        for x in 0..gray.width() {
            let luminance = gray.get_pixel(x, y).0[0] as f32;
            let blurred = (background.get_pixel(x, y).0[0] as f32).max(1.0);
            let value = ((luminance / blurred) * 255.0).clamp(0.0, 255.0) as u8;
            output.put_pixel(x, y, Luma([value]));
        }
    }

    output
}

fn normalize_gray(input: &GrayImage) -> GrayImage {
    let mut min_value = u8::MAX;
    let mut max_value = u8::MIN;

    for pixel in input.pixels() {
        let value = pixel.0[0];
        if value < min_value {
            min_value = value;
        }
        if value > max_value {
            max_value = value;
        }
    }

    let range = (max_value.saturating_sub(min_value)).max(1) as f32;
    let mut output = GrayImage::new(input.width(), input.height());

    for (x, y, pixel) in input.enumerate_pixels() {
        let normalized = (((pixel.0[0].saturating_sub(min_value)) as f32) * 255.0 / range)
            .clamp(0.0, 255.0) as u8;
        output.put_pixel(x, y, Luma([normalized]));
    }

    output
}

fn threshold_to_binary(input: &GrayImage, threshold: u8) -> GrayImage {
    let mut output = GrayImage::new(input.width(), input.height());

    for (x, y, pixel) in input.enumerate_pixels() {
        let value = if pixel.0[0] > threshold { 255 } else { 0 };
        output.put_pixel(x, y, Luma([value]));
    }

    output
}

fn is_reasonable_binary_candidate(binary: &GrayImage) -> bool {
    let total_pixels = (binary.width() as u64 * binary.height() as u64).max(1) as f64;
    let white_pixels = binary.pixels().filter(|pixel| pixel.0[0] > 127).count() as f64;
    let white_ratio = white_pixels / total_pixels;
    let black_ratio = 1.0 - white_ratio;

    white_ratio >= 0.45 && white_ratio <= 0.995 && black_ratio >= 0.005
}

fn gray_to_rgba(source: &GrayImage) -> RgbaImage {
    let mut output = RgbaImage::new(source.width(), source.height());

    for (x, y, pixel) in source.enumerate_pixels() {
        let value = pixel.0[0];
        output.put_pixel(x, y, Rgba([value, value, value, 255]));
    }

    output
}

fn rotate_image(image: RgbaImage, rotation: u16) -> Result<RgbaImage, String> {
    match rotation {
        0 => Ok(image),
        90 => Ok(rotate90(&image)),
        180 => Ok(rotate180(&image)),
        270 => Ok(rotate270(&image)),
        other => Err(format!("Unsupported output rotation: {other}")),
    }
}

/// Transform document points from source-image coordinate space into
/// the coordinate space of the image after it has been rotated by the
/// given `rotation` (in degrees, clockwise).  This mirrors the pixel
/// mapping performed by `rotate_image` (which delegates to
/// `image::imageops::rotate90/180/270`).
///
/// rotate90:  src(x,y) → dst(H-1-y, x)   output dims: H×W
/// rotate180: src(x,y) → dst(W-1-x, H-1-y) output dims: W×H
/// rotate270: src(x,y) → dst(y, W-1-x)   output dims: H×W
fn transform_points_for_rotation(
    points: &[ScannerPoint; 4],
    source_width: u32,
    source_height: u32,
    rotation: u16,
) -> [ScannerPoint; 4] {
    let (sw, sh) = (source_width as f32, source_height as f32);
    let transform = |p: &ScannerPoint| -> ScannerPoint {
        match rotation {
            90  => ScannerPoint { x: sh - 1.0 - p.y, y: p.x },
            180 => ScannerPoint { x: sw - 1.0 - p.x, y: sh - 1.0 - p.y },
            270 => ScannerPoint { x: p.y,             y: sw - 1.0 - p.x },
            _   => *p,
        }
    };
    [
        transform(&points[0]),
        transform(&points[1]),
        transform(&points[2]),
        transform(&points[3]),
    ]
}

/// Deterministic re-ordering of the [TL, TR, BR, BL] slots after the
/// image has been rotated.  Rotation is a circular right-shift of the
/// corner slots by `rotation / 90` positions.
///
/// ```text
///   0°:   [TL, TR, BR, BL]  →  identity
///  90°CW: source BL→TL, TL→TR, TR→BR, BR→BL  →  [3, 0, 1, 2]
/// 180°:   source BR→TL, BL→TR, TL→BR, TR→BL  →  [2, 3, 0, 1]
/// 270°CW: source TR→TL, BR→TR, BL→BR, TL→BL  →  [1, 2, 3, 0]
/// ```
fn reorder_points_after_rotation(
    points: [ScannerPoint; 4],
    rotation: u16,
) -> [ScannerPoint; 4] {
    match rotation {
        90  => [points[3], points[0], points[1], points[2]],
        180 => [points[2], points[3], points[0], points[1]],
        270 => [points[1], points[2], points[3], points[0]],
        _   => points,
    }
}

/// Inverse of `transform_points_for_rotation`: maps points FROM the
/// rotated-image coordinate space BACK to source-image coordinate space.
fn reverse_transform_points_for_rotation(
    points: &[ScannerPoint; 4],
    source_width: u32,
    source_height: u32,
    rotation: u16,
) -> [ScannerPoint; 4] {
    let inverse_rotation = match rotation {
        90  => 270,
        180 => 180,
        270 => 90,
        _   => 0,
    };
    let (rotated_w, rotated_h) = match rotation {
        90 | 270 => (source_height, source_width),
        _        => (source_width, source_height),
    };
    transform_points_for_rotation(points, rotated_w, rotated_h, inverse_rotation)
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut cursor = Cursor::new(Vec::new());
    let encoder =
        PngEncoder::new_with_quality(&mut cursor, CompressionType::Fast, FilterType::Adaptive);
    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ColorType::Rgba8.into(),
        )
        .map_err(|error| format!("Failed to encode PNG payload: {error}"))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba};

    /// Returns true if at least one point moved between `before` and `after`.
    fn refinement_applied_between(before: &[ScannerPoint; 4], after: &[ScannerPoint; 4]) -> bool {
        before.iter().zip(after.iter()).any(|(a, b)| {
            (a.x - b.x).abs() > 0.01 || (a.y - b.y).abs() > 0.01
        })
    }

    #[test]
    fn orders_points_into_tl_tr_br_bl() {
        let ordered = order_points([
            ScannerPoint { x: 90.0, y: 10.0 },
            ScannerPoint { x: 10.0, y: 90.0 },
            ScannerPoint { x: 10.0, y: 10.0 },
            ScannerPoint { x: 90.0, y: 90.0 },
        ]);

        assert_eq!(ordered[0].x, 10.0);
        assert_eq!(ordered[0].y, 10.0);
        assert_eq!(ordered[1].x, 90.0);
        assert_eq!(ordered[1].y, 10.0);
        assert_eq!(ordered[2].x, 90.0);
        assert_eq!(ordered[2].y, 90.0);
        assert_eq!(ordered[3].x, 10.0);
        assert_eq!(ordered[3].y, 90.0);
    }

    #[test]
    fn rotates_rgba_images_orthogonally() {
        let mut image = RgbaImage::new(2, 3);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        image.put_pixel(1, 0, Rgba([0, 255, 0, 255]));
        image.put_pixel(0, 2, Rgba([0, 0, 255, 255]));

        let rotated = rotate_image(image, 90).expect("rotation should succeed");
        assert_eq!(rotated.width(), 3);
        assert_eq!(rotated.height(), 2);
        assert_eq!(rotated.get_pixel(2, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn encodes_processed_image_as_png() {
        let mut image = RgbaImage::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                let value = if x < 16 { 24 } else { 232 };
                image.put_pixel(x, y, Rgba([value, value, value, 255]));
            }
        }

        let enhanced = enhance_document_image(&image, false, "auto");
        let encoded = encode_png(&enhanced).expect("png encoding should succeed");
        assert!(encoded.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn refines_coarse_quad_toward_document_edges() {
        let mut image = RgbaImage::from_pixel(160, 100, Rgba([255, 255, 255, 255]));
        for x in 20..140 {
            for thickness in 0..3 {
                image.put_pixel(x, 15 + thickness, Rgba([0, 0, 0, 255]));
                image.put_pixel(x, 84 - thickness, Rgba([0, 0, 0, 255]));
            }
        }
        for y in 15..85 {
            for thickness in 0..3 {
                image.put_pixel(20 + thickness, y, Rgba([0, 0, 0, 255]));
                image.put_pixel(139 - thickness, y, Rgba([0, 0, 0, 255]));
            }
        }

        let coarse = order_points([
            ScannerPoint { x: 26.0, y: 19.0 },
            ScannerPoint { x: 133.0, y: 22.0 },
            ScannerPoint { x: 136.0, y: 79.0 },
            ScannerPoint { x: 24.0, y: 81.0 },
        ]);
        let refined = refine_document_points(&image, &coarse);
        let expected = order_points([
            ScannerPoint { x: 20.0, y: 15.0 },
            ScannerPoint { x: 139.0, y: 15.0 },
            ScannerPoint { x: 139.0, y: 84.0 },
            ScannerPoint { x: 20.0, y: 84.0 },
        ]);

        for index in 0..4 {
            let coarse_error = point_distance(coarse[index], expected[index]);
            let refined_error = point_distance(refined[index], expected[index]);
            assert!(
                refined_error < coarse_error,
                "refined point {index} should move closer to the expected corner: coarse={coarse_error}, refined={refined_error}",
            );
        }
        assert!(refinement_applied_between(&coarse, &refined));
    }

    #[test]
    fn refinement_prefers_document_edge_over_external_strong_line() {
        let mut image = RgbaImage::from_pixel(220, 180, Rgba([150, 135, 120, 255]));
        for x in 12..208 {
            for thickness in 0..3 {
                image.put_pixel(x, 18 + thickness, Rgba([8, 8, 8, 255]));
            }
        }

        for y in 34..160 {
            for x in 28..188 {
                image.put_pixel(x, y, Rgba([248, 248, 248, 255]));
            }
        }
        for x in 28..188 {
            for thickness in 0..2 {
                image.put_pixel(x, 34 + thickness, Rgba([32, 32, 32, 255]));
                image.put_pixel(x, 159 - thickness, Rgba([32, 32, 32, 255]));
            }
        }
        for y in 34..160 {
            for thickness in 0..2 {
                image.put_pixel(28 + thickness, y, Rgba([32, 32, 32, 255]));
                image.put_pixel(187 - thickness, y, Rgba([32, 32, 32, 255]));
            }
        }

        let coarse = order_points([
            ScannerPoint { x: 36.0, y: 26.0 },
            ScannerPoint { x: 182.0, y: 24.0 },
            ScannerPoint { x: 186.0, y: 156.0 },
            ScannerPoint { x: 30.0, y: 158.0 },
        ]);
        let refined = refine_document_points(&image, &coarse);

        assert!(refined[0].y > 30.0);
        assert!(refined[1].y > 30.0);
        assert!(refined[0].x < 34.0);
        assert!(refined[1].x > 184.0);
    }





    #[test]
    fn local_flattening_prefers_soft_tone_enhancement() {
        // Book page: dark spine shadow on the left side with subtle shading variations.
        let mut image = RgbaImage::from_pixel(120, 90, Rgba([245, 245, 245, 255]));
        for y in 0..90u32 {
            let shadow_width = 8 + ((y as f32 / 89.0) * 10.0).round() as u32;
            for x in 0..shadow_width.min(120) {
                let base_brightness =
                    (60.0 + (x as f32 / shadow_width as f32) * 185.0).clamp(60.0, 245.0) as u8;
                let shade = base_brightness.saturating_sub((x % 6) as u8 * 3);
                image.put_pixel(x, y, Rgba([shade, shade, shade, 255]));
            }
        }

        let source_bytes = encode_png(&image).expect("source png encoding should succeed");
        let (response, encoded) = process_image_request(
            ScannerPostProcessRequest {
                source_bytes,
                document_points: Some(vec![
                    ScannerPoint { x: 0.0, y: 0.0 },
                    ScannerPoint { x: 119.0, y: 0.0 },
                    ScannerPoint { x: 119.0, y: 89.0 },
                    ScannerPoint { x: 0.0, y: 89.0 },
                ]),
                output_rotation: 0,
                image_enhancement: true,
                color_mode: default_color_mode(),
                postprocess_backend: default_postprocess_backend(),
                spine_flattening: true,
                perspective_transform: true,
                grid_postprocess: default_grid_postprocess(),
                pipeline_debug: false,
            },
            None,
            None,
        )
        .expect("book-like enhancement should succeed");

        assert!(response.local_flattening_applied);

        let decoded = image::load_from_memory(&encoded)
            .expect("enhanced output should decode")
            .into_luma8();
        let mut seen = [false; 256];
        let mut unique_values = 0usize;
        for value in decoded.pixels().map(|pixel| pixel.0[0]) {
            let slot = &mut seen[value as usize];
            if !*slot {
                *slot = true;
                unique_values += 1;
            }
        }

        assert!(
            unique_values > 2,
            "soft-tone enhancement should preserve grayscale detail for locally flattened book pages",
        );
    }

    #[test]
    fn background_heavy_scene_prefers_soft_tone_enhancement() {
        let mut image = RgbaImage::from_pixel(180, 120, Rgba([224, 192, 180, 255]));
        for y in 14..112 {
            for x in 34..150 {
                image.put_pixel(x, y, Rgba([242, 242, 238, 255]));
            }
        }
        for y in 0..20 {
            for x in 0..180 {
                image.put_pixel(x, y, Rgba([197, 155, 120, 255]));
            }
        }
        for y in 36..96 {
            for x in 44..140 {
                if (x + (y * 2)) % 11 == 0 {
                    image.put_pixel(x, y, Rgba([72, 72, 72, 255]));
                }
            }
        }

        let enhanced = enhance_document_image(&image, false, "auto");
        let decoded = DynamicImage::ImageRgba8(enhanced).into_luma8();
        let mut seen = [false; 256];
        let mut unique_values = 0usize;
        for value in decoded.pixels().map(|pixel| pixel.0[0]) {
            let slot = &mut seen[value as usize];
            if !*slot {
                *slot = true;
                unique_values += 1;
            }
        }

        assert!(
            unique_values > 2,
            "background-heavy book/page scenes should stay in grayscale instead of collapsing into binary",
        );
    }

    #[test]
    fn page_dominant_scene_keeps_binary_enhancement() {
        let mut image = RgbaImage::from_pixel(160, 120, Rgba([246, 246, 244, 255]));
        for y in 18..102 {
            for x in 22..138 {
                if (x + y) % 9 == 0 {
                    image.put_pixel(x, y, Rgba([24, 24, 24, 255]));
                }
            }
        }

        let enhanced = enhance_document_image(&image, false, "auto");
        let decoded = DynamicImage::ImageRgba8(enhanced).into_luma8();
        let mut seen = [false; 256];
        let mut unique_values = 0usize;
        for value in decoded.pixels().map(|pixel| pixel.0[0]) {
            let slot = &mut seen[value as usize];
            if !*slot {
                *slot = true;
                unique_values += 1;
            }
        }

        assert!(
            unique_values <= 2,
            "page-dominant single-sheet scenes should still use binary enhancement",
        );
    }

    #[test]
    fn flattens_before_rotation_for_portrait_exports() {
        let mut image = RgbaImage::from_pixel(120, 90, Rgba([255, 255, 255, 255]));
        for y in 0..90 {
            let left_inset = 8 + ((y as f32 / 89.0) * 10.0).round() as u32;
            for x in left_inset..110 {
                image.put_pixel(x, y, Rgba([18, 18, 18, 255]));
            }
        }

        let source_bytes = encode_png(&image).expect("source png encoding should succeed");
        let (response, _encoded) = process_image_request(
            ScannerPostProcessRequest {
                source_bytes,
                document_points: Some(vec![
                    ScannerPoint { x: 0.0, y: 0.0 },
                    ScannerPoint { x: 119.0, y: 0.0 },
                    ScannerPoint { x: 119.0, y: 89.0 },
                    ScannerPoint { x: 0.0, y: 89.0 },
                ]),
                output_rotation: 90,
                image_enhancement: false,
                color_mode: default_color_mode(),
                postprocess_backend: default_postprocess_backend(),
                spine_flattening: true,
                perspective_transform: true,
                grid_postprocess: default_grid_postprocess(),
                pipeline_debug: false,
            },
            None,
            None,
        )
        .expect("portrait export should succeed");

        // Image is rotated at decode time before any processing;
        // flatten and perspective operate on the already-rotated image.
        assert!(response.rotate_ms.is_some());
    }

    #[test]
    fn processes_encoded_source_bytes_end_to_end() {
        let mut image = RgbaImage::new(40, 24);
        for y in 0..24 {
            for x in 0..40 {
                let pixel = if x < 20 {
                    Rgba([32, 32, 32, 255])
                } else {
                    Rgba([224, 224, 224, 255])
                };
                image.put_pixel(x, y, pixel);
            }
        }

        let source_bytes = encode_png(&image).expect("source png encoding should succeed");
        let (response, encoded) = process_image_request(
            ScannerPostProcessRequest {
                source_bytes,
                document_points: Some(vec![
                    ScannerPoint { x: 4.0, y: 2.0 },
                    ScannerPoint { x: 35.0, y: 2.0 },
                    ScannerPoint { x: 35.0, y: 21.0 },
                    ScannerPoint { x: 4.0, y: 21.0 },
                ]),
                output_rotation: 90,
                image_enhancement: true,
                color_mode: default_color_mode(),
                postprocess_backend: default_postprocess_backend(),
                spine_flattening: true,
                perspective_transform: true,
                grid_postprocess: default_grid_postprocess(),
                pipeline_debug: false,
            },
            None,
            None,
        )
        .expect("post-process request should succeed");

        assert!(encoded.starts_with(&[0x89, b'P', b'N', b'G']));
        assert_eq!(response.input_width, 24);   // rotated: 40x24 @90° → 24x40
        assert_eq!(response.input_height, 40);
        assert!(response.output_width >= 19);
        assert!(response.output_height >= 19);
        assert!(response.perspective_ms.is_some());
        assert!(response.flatten_ms.is_some());
        assert!(response.enhance_ms.is_some());
        assert!(response.rotate_ms.is_some());
        assert!(response.effective_document_points.is_some());
    }

    #[test]
    fn rejects_degenerate_edge_ratio_quad() {
        // 5af-native style: top edge ~135px, bottom ~1554px, ratio 0.07
        let points = [
            ScannerPoint { x: 1468.0, y: 7.0 },
            ScannerPoint { x: 1603.0, y: 19.0 },
            ScannerPoint {
                x: 1568.0,
                y: 2321.0,
            },
            ScannerPoint { x: 14.0, y: 2533.0 },
        ];
        let result = validate_quad_geometry(&points, 1604, 2549);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("horizontal edge ratio"));
    }

    #[test]
    fn rejects_tiny_area_quad() {
        let points = [
            ScannerPoint { x: 100.0, y: 100.0 },
            ScannerPoint { x: 110.0, y: 100.0 },
            ScannerPoint { x: 110.0, y: 110.0 },
            ScannerPoint { x: 100.0, y: 110.0 },
        ];
        let result = validate_quad_geometry(&points, 1000, 1000);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too small"));
    }

    #[test]
    fn rejects_non_convex_quad() {
        // Concave quad: BR vertex pushed far inward past the diagonal
        let points = [
            ScannerPoint { x: 10.0, y: 10.0 },
            ScannerPoint { x: 90.0, y: 10.0 },
            ScannerPoint { x: 30.0, y: 30.0 }, // pushed deep inside — concave
            ScannerPoint { x: 10.0, y: 90.0 },
        ];
        let result = validate_quad_geometry(&points, 100, 100);
        assert!(result.is_err(), "expected error, got: {:?}", result);
    }

    #[test]
    fn accepts_valid_quad() {
        let points = [
            ScannerPoint { x: 10.0, y: 10.0 },
            ScannerPoint { x: 90.0, y: 10.0 },
            ScannerPoint { x: 90.0, y: 90.0 },
            ScannerPoint { x: 10.0, y: 90.0 },
        ];
        let result = validate_quad_geometry(&points, 100, 100);
        assert!(result.is_ok());
    }

    #[test]
    fn warp_document_to_rect_no_margin() {
        let points = order_points([
            ScannerPoint { x: 20.0, y: 18.0 },
            ScannerPoint { x: 199.0, y: 12.0 },
            ScannerPoint { x: 205.0, y: 161.0 },
            ScannerPoint { x: 16.0, y: 167.0 },
        ]);

        let (target_w, target_h, _projection, _from, _to) =
            compute_document_projection(&points, 240.0, 180.0)
                .expect("projection should succeed");
        let (max_width, max_height) =
            compute_true_aspect_ratio_dimensions(&points, 240.0, 180.0);

        // No margin: output dimensions equal content dimensions.
        assert_eq!(target_w, max_width);
        assert_eq!(target_h, max_height);
    }
}
