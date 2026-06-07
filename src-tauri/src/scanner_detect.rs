use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Manager};

use crate::scanner_assets;
use crate::scanner_detect_config::{
    build_resource_statuses, preferred_provider_from_config, resolve_scanner_detect_config,
    resource_specs_for_current_platform, select_model_variant, DETECT_INTERESTING_PATHS,
};
use crate::scanner_detect_image::{
    decode_detect_image_with_limits, resolve_detect_input, scale_points_between_dimensions,
    ResolvedDetectInput,
};
use crate::scanner_detect_model::run_selected_model_inference;
use crate::scanner_detect_runtime::{
    ensure_ort_session, probe_ort_runtime, resolve_detection_runtime_context, OrtRuntimeSnapshot,
    OrtSessionSnapshot,
};
use crate::scanner_platform;
use crate::scanner_resource;

pub(crate) use crate::scanner_detect_image::build_dynamic_image_from_preview_frame_packet;

const STAGE: &str = "ort-runtime";

pub use crate::scanner_detect_runtime::reset_scanner_detect_runtime_caches;
pub(crate) use crate::scanner_ort::{
    build_scanner_execution_providers, configure_scanner_session_builder_for_current_platform,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectResourceStatus {
    pub(crate) key: String,
    pub(crate) relative_path: String,
    pub(crate) resolved_path: Option<String>,
    pub(crate) exists: bool,
    pub(crate) required: bool,
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct SharedScannerOrtContext {
    pub(crate) runtime_ready: bool,
    pub(crate) preferred_provider: String,
    pub(crate) preferred_provider_ready: bool,
    pub(crate) available_providers: Vec<String>,
    pub(crate) ort_build_info: Option<String>,
    pub(crate) runtime_error: Option<String>,
    pub(crate) context_error: Option<String>,
}

pub(crate) fn ensure_shared_scanner_ort_context(
    resource_dir_hint: Option<PathBuf>,
    _app_config_dir_hint: Option<PathBuf>,
) -> SharedScannerOrtContext {
    let resource_root_candidates =
        scanner_resource::build_resource_root_candidates(resource_dir_hint, None);
    let selected_resource_root = scanner_resource::select_resource_root(
        &resource_root_candidates,
        &DETECT_INTERESTING_PATHS,
    );
    let resource_base_dir = selected_resource_root
        .as_ref()
        .map(|candidate| candidate.path.clone());
    let runtime_snapshot = probe_ort_runtime(resource_base_dir.as_deref());
    let preferred_provider = scanner_platform::default_preferred_provider().to_string();
    let preferred_provider_ready = scanner_platform::is_provider_available(
        &preferred_provider,
        &runtime_snapshot.available_providers,
    );
    let context_error = if resource_base_dir.is_none() {
        Some("Could not resolve the scanner resource directory.".to_string())
    } else {
        None
    };

    SharedScannerOrtContext {
        runtime_ready: runtime_snapshot.ready,
        preferred_provider,
        preferred_provider_ready,
        available_providers: runtime_snapshot.available_providers,
        ort_build_info: runtime_snapshot.ort_build_info,
        runtime_error: runtime_snapshot.runtime_error,
        context_error,
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
pub async fn tauri_scanner_probe_detect(
    app: AppHandle,
) -> Result<ScannerDetectProbeResponse, String> {
    let resource_dir_hint = app.path().resource_dir().ok();
    let installed_assets_current_dir_hint =
        scanner_assets::installed_assets_current_dir_from_app(&app);
    let app_config_dir_hint = app.path().app_config_dir().ok();
    Ok(probe_native_ort_runtime_with_hints(
        resource_dir_hint,
        installed_assets_current_dir_hint,
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
            tauri::async_runtime::spawn_blocking(move || detect_document_opencv_from_bytes(request))
                .await
                .map_err(|error| format!("OpenCV detect task failed: {error}"))?
        }
        _ => {
            let resource_dir_hint = app.path().resource_dir().ok();
            let installed_assets_current_dir_hint =
                scanner_assets::installed_assets_current_dir_from_app(&app);
            let app_config_dir_hint = app.path().app_config_dir().ok();
            tauri::async_runtime::spawn_blocking(move || {
                detect_document_native_ort(
                    request,
                    resource_dir_hint,
                    installed_assets_current_dir_hint,
                    app_config_dir_hint,
                )
            })
            .await
            .map_err(|error| format!("Native ORT task failed: {error}"))?
        }
    }
}

pub fn probe_native_ort_runtime_with_hints(
    resource_dir_hint: Option<PathBuf>,
    installed_assets_current_dir_hint: Option<PathBuf>,
    _app_config_dir_hint: Option<PathBuf>,
) -> ScannerDetectProbeResponse {
    let platform = std::env::consts::OS.to_string();
    let platform_target = scanner_platform::platform_target().to_string();
    let provider_candidates = scanner_platform::provider_candidates()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();

    let resource_root_candidates = scanner_resource::build_resource_root_candidates(
        resource_dir_hint,
        installed_assets_current_dir_hint,
    );
    let selected_resource_root = scanner_resource::select_resource_root(
        &resource_root_candidates,
        &DETECT_INTERESTING_PATHS,
    );
    let resource_base_dir = selected_resource_root
        .as_ref()
        .map(|candidate| candidate.path.clone());
    let config_handle = resource_base_dir
        .as_ref()
        .map(|_base_dir| resolve_scanner_detect_config());
    let config_error: Option<String> = None;
    let resource_specs =
        resource_specs_for_current_platform(config_handle.as_ref().map(|handle| &handle.config));
    let resources = build_resource_statuses(resource_base_dir.as_deref(), &resource_specs);
    let selected_model = config_handle
        .as_ref()
        .and_then(|handle| select_model_variant(&handle.config, &resources));
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
    let preferred_provider_ready = scanner_platform::is_provider_available(
        &preferred_provider,
        &runtime_snapshot.available_providers,
    );

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
        config_path: None,
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
            resource_base_dir.as_deref().map(|base_dir| {
                scanner_resource::path_to_string(&base_dir.join(model.config.model_path.as_str()))
            })
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
        resource_base_dir: resource_base_dir
            .as_deref()
            .map(scanner_resource::path_to_string),
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
    let dynamic_image =
        decode_detect_image_with_limits(&request.source_bytes, "source for OpenCV detect")?;
    let (w, h) = (dynamic_image.width(), dynamic_image.height());
    let points = crate::scanner_cv_detect::detect_contour_quad(
        &dynamic_image,
        request.max_width,
        request.max_height,
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
    installed_assets_current_dir_hint: Option<PathBuf>,
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

    let detection_context_result = resolve_detection_runtime_context(
        resource_dir_hint,
        installed_assets_current_dir_hint,
        app_config_dir_hint,
    );
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
                    selected_runtime_library_path: None,
                    loaded_runtime_library_path: None,
                    runtime_path_mismatch: false,
                },
                OrtSessionSnapshot {
                    ready: false,
                    session_error: Some(error.clone()),
                },
                Some(error),
            ),
        };
    let preferred_provider_ready = scanner_platform::is_provider_available(
        &preferred_provider,
        &runtime_snapshot.available_providers,
    );

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
