use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ort::ep::ExecutionProvider as _;
use ort::{
    ep,
    session::{builder::SessionBuilder, Session},
};
use serde::Serialize;
use tauri::{command, AppHandle, Manager};

use crate::scanner_assets::{self, ScannerAssetsError};
use crate::scanner_platform;
use crate::scanner_resource;

const STAGE: &str = "scanner-ort-base";
const WINDOWS_ORT_RELATIVE_PATH: &str = "onnxruntime/windows/onnxruntime.dll";
const WINDOWS_ORT_SHARED_RELATIVE_PATH: &str =
    "onnxruntime/windows/onnxruntime_providers_shared.dll";
const WINDOWS_DIRECTML_RELATIVE_PATH: &str = "onnxruntime/windows/DirectML.dll";
const LINUX_ORT_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime.so";
const LINUX_ORT_SONAME_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime.so.1";
const LINUX_ORT_VERSIONED_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime.so.1.24.4";
const LINUX_SHARED_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime_providers_shared.so";
const LINUX_CUDA_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime_providers_cuda.so";
const LINUX_TENSORRT_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime_providers_tensorrt.so";

#[derive(Debug, Clone, Copy)]
struct ScannerOrtModelManifestEntry {
    id: &'static str,
    role: &'static str,
    relative_path: &'static str,
}

const SCANNER_ORT_MODEL_MANIFEST: [ScannerOrtModelManifestEntry; 2] = [
    ScannerOrtModelManifestEntry {
        id: "docaligner-fastvit-sa24",
        role: "stage-1-public-baseline",
        relative_path: "models/docaligner-fastvit_sa24.onnx",
    },
    ScannerOrtModelManifestEntry {
        id: "uvdoc-grid-v1",
        role: "stage-2-session-readiness",
        relative_path: "models/uvdoc-best-model.onnx",
    },
];

#[derive(Debug, Default)]
struct OrtRuntimeState {
    environment_ready: bool,
    runtime_library_path: Option<PathBuf>,
    ort_build_info: Option<String>,
    runtime_error: Option<String>,
    available_providers: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct OrtRuntimeSnapshot {
    pub(crate) ready: bool,
    pub(crate) runtime_error: Option<String>,
    pub(crate) ort_build_info: Option<String>,
    pub(crate) available_providers: Vec<String>,
    pub(crate) selected_runtime_library_path: Option<PathBuf>,
    pub(crate) loaded_runtime_library_path: Option<PathBuf>,
    pub(crate) runtime_path_mismatch: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtOutletStatus {
    pub name: String,
    pub dtype: String,
    pub shape: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtModelStatus {
    pub id: String,
    pub role: String,
    pub relative_path: String,
    pub resolved_path: Option<String>,
    pub file_exists: bool,
    pub session_ready: bool,
    pub session_error: Option<String>,
    pub inputs: Vec<ScannerOrtOutletStatus>,
    pub outputs: Vec<ScannerOrtOutletStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtResourceStatus {
    pub relative_path: String,
    pub resolved_path: Option<String>,
    pub exists: bool,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtProbeStatus {
    pub stage: &'static str,
    pub platform: String,
    pub platform_target: String,
    pub resource_resolution_source: String,
    pub resource_base_dir: Option<String>,
    pub selected_runtime_library_path: Option<String>,
    pub loaded_runtime_library_path: Option<String>,
    pub runtime_library_path: Option<String>,
    pub runtime_path_mismatch: bool,
    pub runtime_ready: bool,
    pub model_load_ready: bool,
    pub preferred_provider: String,
    pub preferred_provider_ready: bool,
    pub provider_candidates: Vec<String>,
    pub available_providers: Vec<String>,
    pub ort_build_info: Option<String>,
    pub runtime_error: Option<String>,
    pub resources: Vec<ScannerOrtResourceStatus>,
    pub models: Vec<ScannerOrtModelStatus>,
    pub message: String,
}

#[command]
pub async fn scanner_probe_ort(
    app: AppHandle,
) -> Result<ScannerOrtProbeStatus, ScannerAssetsError> {
    let resource_dir_hint = app.path().resource_dir().ok();
    let installed_assets_current_dir_hint =
        scanner_assets::installed_assets_current_dir_from_app(&app);
    tauri::async_runtime::spawn_blocking(move || {
        probe_scanner_ort_with_hints(resource_dir_hint, installed_assets_current_dir_hint)
    })
    .await
    .map_err(|error| ScannerAssetsError {
        code: "runtime.probe.taskFailed".to_string(),
        retryable: true,
        details: Some(format!("Scanner ORT probe task failed: {error}")),
    })
}

pub fn probe_scanner_ort_with_hints(
    resource_dir_hint: Option<PathBuf>,
    installed_assets_current_dir_hint: Option<PathBuf>,
) -> ScannerOrtProbeStatus {
    let platform = std::env::consts::OS.to_string();
    let platform_target = scanner_platform::platform_target().to_string();
    let preferred_provider = scanner_platform::default_preferred_provider().to_string();
    let provider_candidates = scanner_platform::provider_candidates()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let interesting_paths = interesting_paths_for_current_platform();
    let resource_root_candidates = scanner_resource::build_resource_root_candidates(
        resource_dir_hint,
        installed_assets_current_dir_hint,
    );
    let selected_resource_root =
        scanner_resource::select_resource_root(&resource_root_candidates, &interesting_paths);
    let resource_base_dir = selected_resource_root
        .as_ref()
        .map(|candidate| candidate.path.clone());
    let resources = build_resource_statuses(resource_base_dir.as_deref(), &interesting_paths);
    let runtime_snapshot = probe_ort_runtime(resource_base_dir.as_deref());
    let preferred_provider_ready = scanner_platform::is_provider_available(
        &preferred_provider,
        &runtime_snapshot.available_providers,
    );

    let models = SCANNER_ORT_MODEL_MANIFEST
        .iter()
        .map(|model| {
            probe_model_session(
                resource_base_dir.as_deref(),
                model,
                &runtime_snapshot,
                &runtime_snapshot.available_providers,
            )
        })
        .collect::<Vec<_>>();
    let model_load_ready = runtime_snapshot.ready && models.iter().all(|model| model.session_ready);
    let message = build_probe_message(&runtime_snapshot, &models, model_load_ready);
    let (selected_runtime_library_path, loaded_runtime_library_path, runtime_library_path) =
        runtime_library_path_status_texts(&runtime_snapshot);

    ScannerOrtProbeStatus {
        stage: STAGE,
        platform,
        platform_target,
        resource_resolution_source: selected_resource_root
            .as_ref()
            .map(|candidate| candidate.source.to_string())
            .unwrap_or_else(|| "unresolved".to_string()),
        resource_base_dir: resource_base_dir
            .as_deref()
            .map(scanner_resource::path_to_string),
        selected_runtime_library_path,
        loaded_runtime_library_path,
        runtime_library_path,
        runtime_path_mismatch: runtime_snapshot.runtime_path_mismatch,
        runtime_ready: runtime_snapshot.ready,
        model_load_ready,
        preferred_provider,
        preferred_provider_ready,
        provider_candidates,
        available_providers: runtime_snapshot.available_providers,
        ort_build_info: runtime_snapshot.ort_build_info,
        runtime_error: runtime_snapshot.runtime_error,
        resources,
        models,
        message,
    }
}

fn runtime_library_path_status_texts(
    runtime_snapshot: &OrtRuntimeSnapshot,
) -> (Option<String>, Option<String>, Option<String>) {
    let selected_runtime_library_path = runtime_snapshot
        .selected_runtime_library_path
        .as_deref()
        .map(scanner_resource::path_to_string);
    let loaded_runtime_library_path = runtime_snapshot
        .loaded_runtime_library_path
        .as_deref()
        .map(scanner_resource::path_to_string);
    let runtime_library_path = loaded_runtime_library_path.clone();

    (
        selected_runtime_library_path,
        loaded_runtime_library_path,
        runtime_library_path,
    )
}

fn build_probe_message(
    runtime_snapshot: &OrtRuntimeSnapshot,
    models: &[ScannerOrtModelStatus],
    model_load_ready: bool,
) -> String {
    if !runtime_snapshot.ready {
        return runtime_snapshot
            .runtime_error
            .clone()
            .unwrap_or_else(|| "Scanner ORT runtime is not ready.".to_string());
    }

    if runtime_snapshot.runtime_path_mismatch {
        return runtime_snapshot.runtime_error.clone().unwrap_or_else(|| {
            "Scanner ORT runtime is already initialized from a different library path.".to_string()
        });
    }

    if let Some(model) = models.iter().find(|model| !model.session_ready) {
        return model
            .session_error
            .clone()
            .unwrap_or_else(|| format!("Model {} session is not ready.", model.id));
    }

    if model_load_ready {
        "Scanner ORT runtime is ready and both approved model sessions loaded successfully."
            .to_string()
    } else {
        "Scanner ORT runtime is ready, but model session readiness is incomplete.".to_string()
    }
}

fn probe_model_session(
    resource_base_dir: Option<&Path>,
    model: &ScannerOrtModelManifestEntry,
    runtime_snapshot: &OrtRuntimeSnapshot,
    available_providers: &[String],
) -> ScannerOrtModelStatus {
    let resolved_path = resource_base_dir.map(|base_dir| base_dir.join(model.relative_path));
    let resolved_path_text = resolved_path
        .as_deref()
        .map(scanner_resource::path_to_string);
    let file_exists = resolved_path
        .as_ref()
        .map(|path| path.exists())
        .unwrap_or(false);

    let mut status = ScannerOrtModelStatus {
        id: model.id.to_string(),
        role: model.role.to_string(),
        relative_path: model.relative_path.to_string(),
        resolved_path: resolved_path_text,
        file_exists,
        session_ready: false,
        session_error: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
    };

    let Some(resource_base_dir) = resource_base_dir else {
        status.session_error =
            Some("Could not resolve the scanner resource directory.".to_string());
        return status;
    };

    let model_path = resource_base_dir.join(model.relative_path);
    if !file_exists {
        status.session_error = Some(format!(
            "Missing ONNX model file: {}.",
            scanner_resource::path_to_string(&model_path)
        ));
        return status;
    }

    if let Err(error) = scanner_resource::validate_path_containment(&model_path, resource_base_dir)
    {
        status.session_error = Some(error);
        return status;
    }

    if !runtime_snapshot.ready {
        status.session_error = Some(
            "ONNX Runtime environment is not initialized, so the model session cannot be created."
                .to_string(),
        );
        return status;
    }

    match create_model_session(&model_path, available_providers) {
        Ok(session) => {
            status.inputs = describe_session_outlets(session.inputs());
            status.outputs = describe_session_outlets(session.outputs());
            status.session_ready = true;
        }
        Err(error) => {
            status.session_error = Some(error);
        }
    }

    status
}

fn runtime_state() -> &'static Mutex<OrtRuntimeState> {
    static STATE: OnceLock<Mutex<OrtRuntimeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(OrtRuntimeState::default()))
}

pub(crate) fn probe_ort_runtime(resource_base_dir: Option<&Path>) -> OrtRuntimeSnapshot {
    let Some(resource_base_dir) = resource_base_dir else {
        return OrtRuntimeSnapshot {
            ready: false,
            runtime_error: Some("Could not resolve the scanner resource directory.".to_string()),
            ort_build_info: None,
            available_providers: Vec::new(),
            selected_runtime_library_path: None,
            loaded_runtime_library_path: current_loaded_runtime_library_path(),
            runtime_path_mismatch: false,
        };
    };

    let Some(runtime_library_path) = runtime_library_path_for_current_platform(resource_base_dir)
    else {
        return OrtRuntimeSnapshot {
            ready: false,
            runtime_error: Some(
                "This platform does not have a configured scanner ORT runtime path.".to_string(),
            ),
            ort_build_info: None,
            available_providers: vec!["CPU".to_string()],
            selected_runtime_library_path: None,
            loaded_runtime_library_path: current_loaded_runtime_library_path(),
            runtime_path_mismatch: false,
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
            selected_runtime_library_path: Some(runtime_library_path),
            loaded_runtime_library_path: current_loaded_runtime_library_path(),
            runtime_path_mismatch: false,
        };
    }

    let mut state = runtime_state()
        .lock()
        .expect("ORT runtime state mutex should not be poisoned");

    if !state.environment_ready {
        let init_result = ort::init_from(&runtime_library_path)
            .map(|builder| {
                builder
                    .with_name("scanner-ort-base")
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
                state.runtime_library_path = Some(runtime_library_path.clone());
                state.runtime_error = None;
                state.ort_build_info = Some(ort::info().to_string());
                state.available_providers = available_providers_for_current_platform();
            }
            Err(error) => {
                state.environment_ready = false;
                state.runtime_library_path = None;
                state.runtime_error = Some(error);
                state.available_providers.clear();
            }
        }
    }

    let loaded_runtime_library_path = state.runtime_library_path.clone();
    let mismatch_error = runtime_path_mismatch_error(
        Some(runtime_library_path.as_path()),
        loaded_runtime_library_path.as_deref(),
    );
    let runtime_path_mismatch = mismatch_error.is_some();

    OrtRuntimeSnapshot {
        ready: state.environment_ready,
        runtime_error: mismatch_error.or_else(|| state.runtime_error.clone()),
        ort_build_info: state.ort_build_info.clone(),
        available_providers: state.available_providers.clone(),
        selected_runtime_library_path: Some(runtime_library_path),
        loaded_runtime_library_path,
        runtime_path_mismatch,
    }
}

fn current_loaded_runtime_library_path() -> Option<PathBuf> {
    runtime_state()
        .lock()
        .expect("ORT runtime state mutex should not be poisoned")
        .runtime_library_path
        .clone()
}

fn runtime_path_mismatch_error(selected: Option<&Path>, loaded: Option<&Path>) -> Option<String> {
    let selected = selected?;
    let loaded = loaded?;
    if selected == loaded {
        return None;
    }

    Some(format!(
        "ONNX Runtime is already initialized from {}, but the selected scanner asset runtime is {}. Restart the app to switch runtime libraries.",
        scanner_resource::path_to_string(loaded),
        scanner_resource::path_to_string(selected)
    ))
}

pub(crate) fn create_model_session(
    model_path: &Path,
    available_providers: &[String],
) -> Result<Session, String> {
    Session::builder()
        .map_err(|error| format!("Failed to create ORT session builder: {error}"))
        .and_then(|builder| {
            let mut builder = configure_scanner_session_builder_for_current_platform(builder)?;
            builder = builder
                .with_execution_providers(build_scanner_execution_providers(available_providers))
                .map_err(|error| format!("Failed to configure execution providers: {error}"))?;

            builder
                .commit_from_file(model_path)
                .map_err(|error| format!("Failed to load ONNX model session: {error}"))
        })
}

pub(crate) fn configure_scanner_session_builder_for_current_platform(
    builder: SessionBuilder,
) -> Result<SessionBuilder, String> {
    match std::env::consts::OS {
        "windows" => builder
            .with_parallel_execution(false)
            .and_then(|builder| builder.with_memory_pattern(false))
            .map_err(|error| format!("Failed to apply DirectML-safe session options: {error}")),
        _ => Ok(builder),
    }
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
    available_providers: &[String],
) -> Vec<ort::execution_providers::ExecutionProviderDispatch> {
    let provider_available = |provider: &str| {
        available_providers
            .iter()
            .any(|available| scanner_platform::normalize_provider_name(available) == provider)
    };

    let mut providers = Vec::new();
    match std::env::consts::OS {
        "windows" => {
            if provider_available("DirectML") {
                providers.push(ep::DirectML::default().build());
            }
        }
        "linux" => {
            if provider_available("TensorRT") {
                providers.push(ep::TensorRT::default().build());
            }
            if provider_available("CUDA") {
                providers.push(ep::CUDA::default().build());
            }
        }
        _ => {}
    }

    providers.push(ep::CPU::default().build());
    providers
}

fn interesting_paths_for_current_platform() -> Vec<&'static str> {
    let mut paths = SCANNER_ORT_MODEL_MANIFEST
        .iter()
        .map(|model| model.relative_path)
        .collect::<Vec<_>>();

    match std::env::consts::OS {
        "windows" => {
            paths.extend([
                WINDOWS_ORT_RELATIVE_PATH,
                WINDOWS_ORT_SHARED_RELATIVE_PATH,
                WINDOWS_DIRECTML_RELATIVE_PATH,
            ]);
        }
        "linux" => {
            paths.extend([
                LINUX_ORT_RELATIVE_PATH,
                LINUX_ORT_SONAME_RELATIVE_PATH,
                LINUX_ORT_VERSIONED_RELATIVE_PATH,
                LINUX_SHARED_RELATIVE_PATH,
                LINUX_CUDA_RELATIVE_PATH,
                LINUX_TENSORRT_RELATIVE_PATH,
            ]);
        }
        _ => {}
    }

    paths
}

fn build_resource_statuses(
    resource_base_dir: Option<&Path>,
    resource_paths: &[&'static str],
) -> Vec<ScannerOrtResourceStatus> {
    resource_paths
        .iter()
        .map(|relative_path| {
            let resolved_path = resource_base_dir.map(|base_dir| base_dir.join(relative_path));
            ScannerOrtResourceStatus {
                relative_path: (*relative_path).to_string(),
                resolved_path: resolved_path
                    .as_deref()
                    .map(scanner_resource::path_to_string),
                exists: resolved_path
                    .as_ref()
                    .map(|path| path.exists())
                    .unwrap_or(false),
                required: true,
            }
        })
        .collect()
}

fn describe_session_outlets(outlets: &[ort::value::Outlet]) -> Vec<ScannerOrtOutletStatus> {
    outlets
        .iter()
        .map(|outlet| ScannerOrtOutletStatus {
            name: outlet.name().to_string(),
            dtype: outlet
                .dtype()
                .tensor_type()
                .map(|tensor_type| tensor_type.to_string())
                .unwrap_or_else(|| outlet.dtype().to_string()),
            shape: outlet.dtype().tensor_shape().map(|shape| shape.to_string()),
        })
        .collect()
}
