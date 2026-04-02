use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageBuffer, Luma, RgbImage};
use ort::ep::ExecutionProvider as _;
use ort::{
    ep,
    session::{builder::SessionBuilder, Session},
    value::Tensor,
};
use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Manager};

const STAGE: &str = "ort-runtime";
const CONFIG_RELATIVE_PATH: &str = "scanner-yolo-config.json";
const WINDOWS_ORT_RELATIVE_PATH: &str = "onnxruntime/windows/onnxruntime.dll";
const WINDOWS_ORT_SHARED_RELATIVE_PATH: &str =
    "onnxruntime/windows/onnxruntime_providers_shared.dll";
const WINDOWS_DIRECTML_RELATIVE_PATH: &str = "onnxruntime/windows/DirectML.dll";
const LINUX_ORT_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime.so";
const LINUX_TENSORRT_RELATIVE_PATH: &str =
    "onnxruntime/linux/libonnxruntime_providers_tensorrt.so";
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
pub struct ScannerYoloModelConfig {
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
pub struct ScannerYoloWindowsConfig {
    preferred_provider: String,
    runtime_library: String,
    shared_library: String,
    provider_library: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerYoloLinuxConfig {
    preferred_providers: Vec<String>,
    runtime_library: String,
    #[serde(default)]
    provider_libraries: Vec<String>,
    official_gpu_release_artifact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerYoloConfig {
    stage: String,
    task: String,
    intended_primary_model: ScannerYoloModelConfig,
    active_public_baseline: ScannerYoloModelConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    windows: Option<ScannerYoloWindowsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linux: Option<ScannerYoloLinuxConfig>,
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResolvedScannerModel {
    variant: ScannerModelVariant,
    config: ScannerYoloModelConfig,
}

#[derive(Debug, Clone)]
struct ScannerYoloConfigHandle {
    config: ScannerYoloConfig,
    resolved_path: PathBuf,
    source: &'static str,
}

#[derive(Debug, Clone)]
struct ResourceRootCandidate {
    source: &'static str,
    path: PathBuf,
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
struct OrtSessionSnapshot {
    ready: bool,
    session_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerYoloResourceStatus {
    key: String,
    relative_path: String,
    resolved_path: Option<String>,
    exists: bool,
    required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerYoloProbeResponse {
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
    resources: Vec<ScannerYoloResourceStatus>,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerYoloConfigResponse {
    config: ScannerYoloConfig,
    source: String,
    resolved_path: String,
    writable_path: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPoint {
    x: f32,
    y: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectDocumentRequest {
    pub source_bytes: Vec<u8>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerDetectDocumentResponse {
    stage: &'static str,
    processing_ms: f64,
    input_width: Option<u32>,
    input_height: Option<u32>,
    selected_model_id: Option<String>,
    selected_model_kind: Option<String>,
    selected_model_task: Option<String>,
    runtime_ready: bool,
    preferred_provider: String,
    preferred_provider_ready: bool,
    model_ready: bool,
    session_ready: bool,
    detection_implemented: bool,
    ort_build_info: Option<String>,
    runtime_error: Option<String>,
    session_error: Option<String>,
    points: Option<Vec<ScannerPoint>>,
    message: String,
}

#[command]
pub async fn tauri_scanner_probe_yolo(app: AppHandle) -> Result<ScannerYoloProbeResponse, String> {
    let resource_dir_hint = app.path().resource_dir().ok();
    let app_config_dir_hint = app.path().app_config_dir().ok();
    Ok(probe_native_yolo_runtime_with_hints(
        resource_dir_hint,
        app_config_dir_hint,
    ))
}

#[command]
pub async fn tauri_scanner_detect_document(
    app: AppHandle,
    request: ScannerDetectDocumentRequest,
) -> Result<ScannerDetectDocumentResponse, String> {
    let resource_dir_hint = app.path().resource_dir().ok();
    let app_config_dir_hint = app.path().app_config_dir().ok();
    tauri::async_runtime::spawn_blocking(move || {
        detect_document_native_yolo(request, resource_dir_hint, app_config_dir_hint)
    })
    .await
    .map_err(|error| format!("Native YOLO task failed: {error}"))?
}

#[command]
pub async fn tauri_scanner_read_yolo_config(
    app: AppHandle,
) -> Result<ScannerYoloConfigResponse, String> {
    let resource_dir_hint = app.path().resource_dir().ok();
    let app_config_dir_hint = app.path().app_config_dir().ok();
    let resolved = resolve_scanner_yolo_config(resource_dir_hint, app_config_dir_hint)?;
    Ok(build_scanner_yolo_config_response(
        &resolved,
        build_scanner_yolo_config_writable_path(app.path().app_config_dir().ok())?,
    ))
}

#[command]
pub async fn tauri_scanner_write_yolo_config(
    app: AppHandle,
    config: ScannerYoloConfig,
) -> Result<ScannerYoloConfigResponse, String> {
    let writable_path = build_scanner_yolo_config_writable_path(app.path().app_config_dir().ok())?;
    validate_scanner_yolo_config(&config)?;
    write_scanner_yolo_config(&writable_path, &config)?;
    reset_ort_session_cache();

    Ok(ScannerYoloConfigResponse {
        config,
        source: "app-config-override".to_string(),
        resolved_path: path_to_string(&writable_path),
        writable_path: path_to_string(&writable_path),
    })
}

pub fn probe_native_yolo_runtime() -> ScannerYoloProbeResponse {
    probe_native_yolo_runtime_with_hints(None, None)
}

pub fn probe_native_yolo_runtime_with_hints(
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> ScannerYoloProbeResponse {
    let platform = std::env::consts::OS.to_string();
    let platform_target = platform_target_for_current_platform().to_string();
    let provider_candidates = provider_candidates_for_current_platform()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();

    let resource_root_candidates = build_resource_root_candidates(resource_dir_hint);
    let selected_resource_root = select_resource_root(&resource_root_candidates);
    let resource_base_dir = selected_resource_root
        .as_ref()
        .map(|candidate| candidate.path.clone());
    let config_handle = resource_base_dir.as_ref().and_then(|base_dir| {
        resolve_scanner_yolo_config(
            Some(base_dir.clone()),
            app_config_dir_hint.clone(),
        )
        .ok()
    });
    let config_error = if resource_base_dir.is_some() && config_handle.is_none() {
        resolve_scanner_yolo_config(resource_base_dir.clone(), app_config_dir_hint.clone())
            .err()
    } else {
        None
    };
    let resource_specs = resource_specs_for_current_platform(config_handle.as_ref().map(|handle| &handle.config));
    let resources = build_resource_statuses(resource_base_dir.as_deref(), &resource_specs);
    let selected_model = config_handle
        .as_ref()
        .and_then(|handle| select_model_variant(handle, &resources));
    let preferred_provider = config_handle
        .as_ref()
        .map(|handle| preferred_provider_from_config(&handle.config))
        .unwrap_or_else(|| default_preferred_provider_for_current_platform().to_string());
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
                "No supported stage-1 model was found. Install the public baseline model or provide the planned YOLO model."
                    .to_string(),
            ),
        }
    };
    let preferred_provider_ready =
        is_provider_available(&preferred_provider, &runtime_snapshot.available_providers);

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

    ScannerYoloProbeResponse {
        stage: STAGE,
        platform,
        platform_target,
        config_source: config_handle
            .as_ref()
            .map(|handle| handle.source.to_string())
            .unwrap_or_else(|| "unresolved".to_string()),
        config_path: config_handle
            .as_ref()
            .map(|handle| path_to_string(&handle.resolved_path)),
        preferred_provider,
        provider_candidates,
        selected_model_id: selected_model.as_ref().map(|model| model.config.id.clone()),
        selected_model_kind: selected_model.as_ref().map(|model| model.config.kind.clone()),
        selected_model_task: selected_model.as_ref().map(|model| model.config.task.clone()),
        selected_model_path: selected_model.as_ref().and_then(|model| {
            resource_base_dir.as_deref().map(|base_dir| {
                path_to_string(&base_dir.join(model.config.model_path.as_str()))
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
        resource_base_dir: resource_base_dir.as_deref().map(path_to_string),
        resources,
        message,
    }
}

pub fn detect_document_native_yolo(
    request: ScannerDetectDocumentRequest,
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> Result<ScannerDetectDocumentResponse, String> {
    let started_at = Instant::now();
    let probe = probe_native_yolo_runtime_with_hints(
        resource_dir_hint.clone(),
        app_config_dir_hint.clone(),
    );
    let _requested_limits = (request.max_width, request.max_height);

    let decoded_image = if request.source_bytes.is_empty() {
        None
    } else {
        Some(
            image::load_from_memory(&request.source_bytes).map_err(|error| {
                format!("Failed to decode source image for native scanner inference: {error}")
            })?,
        )
    };

    let (input_width, input_height) = decoded_image
        .as_ref()
        .map(DynamicImage::dimensions)
        .map_or((None, None), |(width, height)| (Some(width), Some(height)));

    let resource_base_dir = select_resource_root(&build_resource_root_candidates(resource_dir_hint))
        .map(|candidate| candidate.path);
    let selected_model = if let Some(base_dir) = resource_base_dir.as_ref() {
        let config_handle =
            resolve_scanner_yolo_config(Some(base_dir.clone()), app_config_dir_hint).ok();
        let resource_specs =
            resource_specs_for_current_platform(config_handle.as_ref().map(|handle| &handle.config));
        let resources = build_resource_statuses(Some(base_dir), &resource_specs);
        config_handle
            .as_ref()
            .and_then(|handle| select_model_variant(handle, &resources))
    } else {
        None
    };

    let (points, message) = if !probe.runtime_ready {
        (None, probe.message.clone())
    } else if !probe.session_ready {
        (
            None,
            probe.session_error.clone().unwrap_or_else(|| {
                "ONNX Runtime is ready but the model session is not.".to_string()
            }),
        )
    } else if decoded_image.is_none() {
        (
            None,
            "The request did not include image bytes, so inference was skipped.".to_string(),
        )
    } else if let Some(model) = selected_model {
        match run_selected_model_inference(&model, decoded_image.as_ref().expect("image exists")) {
            Ok(points) => (
                Some(points),
                format!(
                    "Model {} completed native Rust inference.",
                    model.config.id
                ),
            ),
            Err(error) => (None, error),
        }
    } else {
        (
            None,
            "No supported stage-1 model was selected, so inference was skipped.".to_string(),
        )
    };

    Ok(ScannerDetectDocumentResponse {
        stage: STAGE,
        processing_ms: started_at.elapsed().as_secs_f64() * 1000.0,
        input_width,
        input_height,
        selected_model_id: probe.selected_model_id,
        selected_model_kind: probe.selected_model_kind,
        selected_model_task: probe.selected_model_task,
        runtime_ready: probe.runtime_ready,
        preferred_provider: probe.preferred_provider,
        preferred_provider_ready: probe.preferred_provider_ready,
        model_ready: probe.model_ready,
        session_ready: probe.session_ready,
        detection_implemented: probe.detection_implemented,
        ort_build_info: probe.ort_build_info,
        runtime_error: probe.runtime_error,
        session_error: probe.session_error,
        points,
        message,
    })
}

fn build_scanner_yolo_config_response(
    handle: &ScannerYoloConfigHandle,
    writable_path: PathBuf,
) -> ScannerYoloConfigResponse {
    ScannerYoloConfigResponse {
        config: handle.config.clone(),
        source: handle.source.to_string(),
        resolved_path: path_to_string(&handle.resolved_path),
        writable_path: path_to_string(&writable_path),
    }
}

fn build_scanner_yolo_config_writable_path(
    app_config_dir_hint: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let Some(app_config_dir) = app_config_dir_hint else {
        return Err("Could not resolve the writable app config directory.".to_string());
    };

    Ok(app_config_dir.join(CONFIG_RELATIVE_PATH))
}

fn resolve_scanner_yolo_config(
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> Result<ScannerYoloConfigHandle, String> {
    let resource_root_candidates = build_resource_root_candidates(resource_dir_hint);
    let selected_resource_root = select_resource_root(&resource_root_candidates)
        .ok_or_else(|| "Could not resolve the scanner resource directory.".to_string())?;
    let default_config_path = selected_resource_root.path.join(CONFIG_RELATIVE_PATH);
    let override_config_path = build_scanner_yolo_config_writable_path(app_config_dir_hint).ok();

    if let Some(override_path) = override_config_path.as_ref() {
        if override_path.exists() {
            let config = load_scanner_yolo_config_from_path(override_path)?;
            return Ok(ScannerYoloConfigHandle {
                config,
                resolved_path: override_path.clone(),
                source: "app-config-override",
            });
        }
    }

    let config = load_scanner_yolo_config_from_path(&default_config_path)?;
    Ok(ScannerYoloConfigHandle {
        config,
        resolved_path: default_config_path,
        source: "bundled-resource-default",
    })
}

fn load_scanner_yolo_config_from_path(path: &Path) -> Result<ScannerYoloConfig, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read scanner YOLO config {}: {error}", path_to_string(path)))?;
    let config = serde_json::from_str::<ScannerYoloConfig>(&raw)
        .map_err(|error| format!("Failed to parse scanner YOLO config {}: {error}", path_to_string(path)))?;
    validate_scanner_yolo_config(&config)?;
    Ok(config)
}

fn write_scanner_yolo_config(path: &Path, config: &ScannerYoloConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create scanner YOLO config directory {}: {error}",
                path_to_string(parent)
            )
        })?;
    }

    let payload = serde_json::to_string_pretty(config)
        .map_err(|error| format!("Failed to serialize scanner YOLO config: {error}"))?;
    fs::write(path, payload + "\n").map_err(|error| {
        format!(
            "Failed to write scanner YOLO config {}: {error}",
            path_to_string(path)
        )
    })
}

fn validate_scanner_yolo_config(config: &ScannerYoloConfig) -> Result<(), String> {
    validate_scanner_yolo_model_config(&config.intended_primary_model, "intendedPrimaryModel")?;
    validate_scanner_yolo_model_config(&config.active_public_baseline, "activePublicBaseline")?;

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

fn validate_scanner_yolo_model_config(
    config: &ScannerYoloModelConfig,
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

fn reset_ort_session_cache() {
    let mut state = runtime_state()
        .lock()
        .expect("ORT runtime state mutex should not be poisoned");
    state.session = None;
    state.session_model_path = None;
    state.session_error = None;
}

fn build_resource_root_candidates(resource_dir_hint: Option<PathBuf>) -> Vec<ResourceRootCandidate> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    let mut push_candidate = |source: &'static str, path: PathBuf| {
        if seen.insert(path.clone()) {
            candidates.push(ResourceRootCandidate { source, path });
        }
    };

    if let Some(resource_dir) = resource_dir_hint {
        push_candidate("tauri-resource-dir", resource_dir);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    push_candidate("cargo-manifest-resources", manifest_dir.join("resources"));

    if let Ok(current_dir) = std::env::current_dir() {
        push_candidate("cwd-resources", current_dir.join("resources"));
        push_candidate(
            "cwd-src-tauri-resources",
            current_dir.join("src-tauri").join("resources"),
        );
    }

    candidates
}

fn select_resource_root(candidates: &[ResourceRootCandidate]) -> Option<ResourceRootCandidate> {
    candidates
        .iter()
        .max_by_key(|candidate| score_resource_root(&candidate.path))
        .cloned()
}

fn score_resource_root(root: &Path) -> usize {
    let interesting_paths = [
        CONFIG_RELATIVE_PATH,
        "models/docaligner-fastvit_sa24.onnx",
        "models/document-boundary-yolo-pose.onnx",
        WINDOWS_ORT_RELATIVE_PATH,
        WINDOWS_ORT_SHARED_RELATIVE_PATH,
        WINDOWS_DIRECTML_RELATIVE_PATH,
        LINUX_ORT_RELATIVE_PATH,
        LINUX_TENSORRT_RELATIVE_PATH,
        LINUX_CUDA_RELATIVE_PATH,
    ];

    interesting_paths
        .iter()
        .filter(|relative_path| root.join(relative_path).exists())
        .count()
}

fn resource_specs_for_current_platform(config: Option<&ScannerYoloConfig>) -> Vec<ResourceSpec> {
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
) -> Vec<ScannerYoloResourceStatus> {
    specs
        .iter()
        .map(|spec| {
            let resolved_path = resource_base_dir.map(|base_dir| base_dir.join(&spec.relative_path));
            let exists = resolved_path
                .as_ref()
                .map(|path| path.exists())
                .unwrap_or(false);

            ScannerYoloResourceStatus {
                key: spec.key.clone(),
                relative_path: spec.relative_path.clone(),
                resolved_path: resolved_path.as_deref().map(path_to_string),
                exists,
                required: spec.required,
            }
        })
        .collect()
}

fn resource_exists(resources: &[ScannerYoloResourceStatus], key: &str) -> bool {
    resources
        .iter()
        .find(|resource| resource.key == key)
        .map(|resource| resource.exists)
        .unwrap_or(false)
}

fn platform_target_for_current_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows-directml",
        "linux" => "linux-tensorrt-cuda",
        _ => "desktop-unsupported",
    }
}

fn provider_candidates_for_current_platform() -> Vec<&'static str> {
    match std::env::consts::OS {
        "windows" => vec!["DirectML", "CPU"],
        "linux" => vec!["TensorRT", "CUDA", "CPU"],
        _ => vec!["CPU"],
    }
}

fn default_preferred_provider_for_current_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "DirectML",
        "linux" => "TensorRT",
        _ => "CPU",
    }
}

fn preferred_provider_from_config(config: &ScannerYoloConfig) -> String {
    match std::env::consts::OS {
        "windows" => config
            .windows
            .as_ref()
            .map(|windows| windows.preferred_provider.clone())
            .unwrap_or_else(|| default_preferred_provider_for_current_platform().to_string()),
        "linux" => config
            .linux
            .as_ref()
            .and_then(|linux| linux.preferred_providers.first().cloned())
            .unwrap_or_else(|| default_preferred_provider_for_current_platform().to_string()),
        _ => default_preferred_provider_for_current_platform().to_string(),
    }
}

fn normalize_provider_name(provider: &str) -> String {
    match provider.trim().to_ascii_lowercase().as_str() {
        "directml" => "DirectML".to_string(),
        "tensorrt" => "TensorRT".to_string(),
        "cuda" => "CUDA".to_string(),
        "cpu" => "CPU".to_string(),
        other => other.to_string(),
    }
}

fn is_provider_available(preferred_provider: &str, available_providers: &[String]) -> bool {
    let normalized = normalize_provider_name(preferred_provider);
    available_providers
        .iter()
        .any(|provider| normalize_provider_name(provider) == normalized)
}

fn candidate_model_variants(config: &ScannerYoloConfig) -> Vec<ResolvedScannerModel> {
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
    handle: &ScannerYoloConfigHandle,
    resources: &[ScannerYoloResourceStatus],
) -> Option<ResolvedScannerModel> {
    candidate_model_variants(&handle.config)
        .into_iter()
        .find(|model| resource_exists(resources, model.variant.resource_key()))
}

fn runtime_state() -> &'static Mutex<OrtRuntimeState> {
    static STATE: OnceLock<Mutex<OrtRuntimeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(OrtRuntimeState::default()))
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

    let Some(runtime_library_path) = runtime_library_path_for_current_platform(resource_base_dir) else {
        return OrtRuntimeSnapshot {
            ready: false,
            runtime_error: Some("This platform does not have a configured ORT runtime path yet.".to_string()),
            ort_build_info: None,
            available_providers: vec!["CPU".to_string()],
        };
    };

    if !runtime_library_path.exists() {
        return OrtRuntimeSnapshot {
            ready: false,
            runtime_error: Some(format!(
                "Missing ONNX Runtime library: {}.",
                path_to_string(&runtime_library_path)
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
            .map(|builder| builder.with_name("scanner-native-yolo").with_telemetry(false).commit())
            .map_err(|error| {
                format!(
                    "Failed to initialize ONNX Runtime from {}: {error}",
                    path_to_string(&runtime_library_path)
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
                path_to_string(&model_path)
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
            let mut builder = configure_session_builder_for_current_platform(builder)?;
            builder = builder
                .with_execution_providers(build_execution_providers(preferred_provider))
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

fn configure_session_builder_for_current_platform(
    builder: SessionBuilder,
) -> Result<SessionBuilder, String> {
    match std::env::consts::OS {
        "windows" => {
            // DirectML requires sequential execution and disabled memory-pattern optimization.
            builder
                .with_parallel_execution(false)
                .and_then(|builder: SessionBuilder| builder.with_memory_pattern(false))
                .map_err(|error| {
                    format!("Failed to apply DirectML-safe session options: {error}")
                })
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
            "The planned YOLO pose model can be loaded, but Rust-side output decoding for it is not implemented yet."
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
    let (shape, heatmap) = output_value
        .try_extract_tensor::<f32>()
        .map_err(|error| format!("Failed to extract heatmap tensor: {error}"))?;

    decode_docaligner_heatmap_output(heatmap, shape, original_width, original_height)
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
    original_width: u32,
    original_height: u32,
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

        let plane_u8 = plane
            .iter()
            .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect::<Vec<_>>();
        let plane_image = ImageBuffer::<Luma<u8>, Vec<u8>>::from_vec(
            heatmap_width as u32,
            heatmap_height as u32,
            plane_u8,
        )
        .ok_or_else(|| "Failed to materialize heatmap image buffer.".to_string())?;
        let upsampled = image::imageops::resize(
            &plane_image,
            original_width,
            original_height,
            FilterType::Triangle,
        );

        let centroid = largest_component_centroid(
            upsampled.as_raw(),
            upsampled.width(),
            upsampled.height(),
            77,
        )
        .ok_or_else(|| {
            format!(
                "DocAligner heatmap channel {channel} did not produce a detectable corner region."
            )
        })?;

        points.push(ScannerPoint {
            x: centroid.0,
            y: centroid.1,
        });
    }

    Ok(points)
}

fn largest_component_centroid(
    grayscale: &[u8],
    width: u32,
    height: u32,
    threshold: u8,
) -> Option<(f32, f32)> {
    let width_usize = width as usize;
    let height_usize = height as usize;
    let mut visited = vec![false; width_usize * height_usize];
    let mut best_count = 0usize;
    let mut best_centroid = None;

    for y in 0..height_usize {
        for x in 0..width_usize {
            let index = (y * width_usize) + x;
            if visited[index] || grayscale.get(index).copied().unwrap_or(0) <= threshold {
                continue;
            }

            let mut queue = VecDeque::from([(x, y)]);
            visited[index] = true;
            let mut count = 0usize;
            let mut sum_x = 0f64;
            let mut sum_y = 0f64;

            while let Some((cx, cy)) = queue.pop_front() {
                count += 1;
                sum_x += cx as f64;
                sum_y += cy as f64;

                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }

                        let nx = cx as isize + dx;
                        let ny = cy as isize + dy;
                        if nx < 0
                            || ny < 0
                            || nx >= width_usize as isize
                            || ny >= height_usize as isize
                        {
                            continue;
                        }

                        let nx = nx as usize;
                        let ny = ny as usize;
                        let neighbor_index = (ny * width_usize) + nx;
                        if visited[neighbor_index]
                            || grayscale.get(neighbor_index).copied().unwrap_or(0) <= threshold
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
                best_centroid = Some((
                    (sum_x / count as f64) as f32,
                    (sum_y / count as f64) as f32,
                ));
            }
        }
    }

    best_centroid
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

fn build_execution_providers(
    preferred_provider: &str,
) -> Vec<ort::execution_providers::ExecutionProviderDispatch> {
    let normalized = normalize_provider_name(preferred_provider);

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

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_scanner_yolo_config() -> ScannerYoloConfig {
        ScannerYoloConfig {
            stage: "runtime-plus-public-baseline".to_string(),
            task: "document-boundary-stage1".to_string(),
            intended_primary_model: ScannerYoloModelConfig {
                id: "document-boundary-yolo-pose-4pt".to_string(),
                kind: "planned-primary".to_string(),
                task: "document-corner-keypoints".to_string(),
                model_path: "models/document-boundary-yolo-pose.onnx".to_string(),
                input_name: None,
                output_name: None,
                input_size: None,
            },
            active_public_baseline: ScannerYoloModelConfig {
                id: "docaligner-fastvit-sa24".to_string(),
                kind: "public-baseline".to_string(),
                task: "document-corner-heatmap".to_string(),
                model_path: "models/docaligner-fastvit_sa24.onnx".to_string(),
                input_name: Some("img".to_string()),
                output_name: Some("heatmap".to_string()),
                input_size: Some([256, 256]),
            },
            windows: Some(ScannerYoloWindowsConfig {
                preferred_provider: "directml".to_string(),
                runtime_library: WINDOWS_ORT_RELATIVE_PATH.to_string(),
                shared_library: WINDOWS_ORT_SHARED_RELATIVE_PATH.to_string(),
                provider_library: WINDOWS_DIRECTML_RELATIVE_PATH.to_string(),
            }),
            linux: Some(ScannerYoloLinuxConfig {
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

    #[test]
    fn current_platform_has_expected_provider_candidates() {
        let providers = provider_candidates_for_current_platform();
        assert!(!providers.is_empty());
        assert!(providers.contains(&"CPU"));
    }

    #[test]
    fn current_platform_has_config_and_model_specs() {
        let config = sample_scanner_yolo_config();
        let specs = resource_specs_for_current_platform(Some(&config));
        assert!(specs.iter().any(|spec| spec.key == "config" && spec.required));
        assert!(specs.iter().any(|spec| spec.key == "model-active-public-baseline"));
        assert!(specs.iter().any(|spec| spec.key == "model-intended-primary"));
    }

    #[test]
    fn build_execution_providers_always_keeps_cpu_fallback() {
        let providers = build_execution_providers("TensorRT");
        assert!(!providers.is_empty());
    }

    #[test]
    fn validate_scanner_yolo_config_rejects_empty_linux_provider_entries() {
        let mut config = sample_scanner_yolo_config();
        if let Some(linux) = config.linux.as_mut() {
            linux.preferred_providers = vec!["cuda".to_string(), "".to_string()];
        }

        let error = validate_scanner_yolo_config(&config)
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
    fn largest_component_centroid_prefers_biggest_region() {
        let width = 6;
        let height = 4;
        let mut mask = vec![0u8; width * height];

        for &(x, y) in &[(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
            mask[(y * width) + x] = 255;
        }
        for &(x, y) in &[(4usize, 1usize), (5, 1)] {
            mask[(y * width) + x] = 255;
        }

        let centroid = largest_component_centroid(&mask, width as u32, height as u32, 1)
            .expect("centroid should exist");
        assert!(centroid.0 < 1.0);
        assert!(centroid.1 < 1.0);
    }
}
