use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ort::ep::ExecutionProvider as _;
use ort::{
    ep,
    session::{builder::SessionBuilder, Session},
};

use crate::scanner_ort_protocol::{
    build_runtime_path_mismatch_error, ScannerOrtModelStatus, ScannerOrtOutletStatus,
    ScannerOrtWorkerModelRequest, ScannerOrtWorkerOperation, ScannerOrtWorkerProbeResult,
    ScannerOrtWorkerRequest, ScannerOrtWorkerRequestPayload, ScannerOrtWorkerResourceRequest,
    ScannerOrtWorkerResponse, ScannerOrtWorkerResponsePayload, ScannerOrtWorkerRuntimeStatus,
};
use crate::{scanner_platform, scanner_resource};

#[derive(Debug, Default)]
struct OrtRuntimeState {
    environment_ready: bool,
    runtime_library_path: Option<PathBuf>,
    ort_build_info: Option<String>,
    runtime_error: Option<String>,
    available_providers: Vec<String>,
}

pub fn handle_worker_request(request: ScannerOrtWorkerRequest) -> (ScannerOrtWorkerResponse, bool) {
    match request.payload {
        ScannerOrtWorkerRequestPayload::Init(resource_request) => (
            ScannerOrtWorkerResponse {
                id: request.id,
                payload: ScannerOrtWorkerResponsePayload::Ok {
                    operation: ScannerOrtWorkerOperation::Init,
                    probe: Some(probe_resource_request(&resource_request)),
                },
            },
            false,
        ),
        ScannerOrtWorkerRequestPayload::Probe(resource_request) => (
            ScannerOrtWorkerResponse {
                id: request.id,
                payload: ScannerOrtWorkerResponsePayload::Ok {
                    operation: ScannerOrtWorkerOperation::Probe,
                    probe: Some(probe_resource_request(&resource_request)),
                },
            },
            false,
        ),
        ScannerOrtWorkerRequestPayload::Shutdown => (
            ScannerOrtWorkerResponse {
                id: request.id,
                payload: ScannerOrtWorkerResponsePayload::Ok {
                    operation: ScannerOrtWorkerOperation::Shutdown,
                    probe: None,
                },
            },
            true,
        ),
    }
}

fn probe_resource_request(
    request: &ScannerOrtWorkerResourceRequest,
) -> ScannerOrtWorkerProbeResult {
    let resource_base_dir = Path::new(&request.resource_base_dir);
    let runtime = probe_ort_runtime(request.runtime_library_path.as_deref().map(Path::new));
    let models = request
        .models
        .iter()
        .map(|model| probe_model_session(resource_base_dir, model, &runtime))
        .collect::<Vec<_>>();

    ScannerOrtWorkerProbeResult { runtime, models }
}

fn probe_model_session(
    resource_base_dir: &Path,
    model: &ScannerOrtWorkerModelRequest,
    runtime: &ScannerOrtWorkerRuntimeStatus,
) -> ScannerOrtModelStatus {
    let resolved_path = PathBuf::from(&model.resolved_path);
    let resolved_path_text = Some(scanner_resource::path_to_string(&resolved_path));
    let file_exists = resolved_path.exists();

    let mut status = ScannerOrtModelStatus {
        id: model.id.clone(),
        role: model.role.clone(),
        relative_path: model.relative_path.clone(),
        resolved_path: resolved_path_text,
        file_exists,
        session_ready: false,
        session_error: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
    };

    if !file_exists {
        status.session_error = Some(format!(
            "Missing ONNX model file: {}.",
            scanner_resource::path_to_string(&resolved_path)
        ));
        return status;
    }

    if let Err(error) =
        scanner_resource::validate_path_containment(&resolved_path, resource_base_dir)
    {
        status.session_error = Some(error);
        return status;
    }

    if !runtime.ready {
        status.session_error = Some(
            "ONNX Runtime environment is not initialized, so the model session cannot be created."
                .to_string(),
        );
        return status;
    }

    match create_model_session(&resolved_path, &runtime.available_providers) {
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

fn probe_ort_runtime(runtime_library_path: Option<&Path>) -> ScannerOrtWorkerRuntimeStatus {
    let Some(runtime_library_path) = runtime_library_path else {
        return ScannerOrtWorkerRuntimeStatus {
            ready: false,
            runtime_error: Some(
                "This platform does not have a configured scanner ORT runtime path.".to_string(),
            ),
            ort_build_info: None,
            available_providers: vec!["CPU".to_string()],
            selected_runtime_library_path: None,
            loaded_runtime_library_path: current_loaded_runtime_library_path_text(),
            runtime_path_mismatch: false,
        };
    };

    let selected_runtime_library_path = scanner_resource::path_to_string(&runtime_library_path);
    if !runtime_library_path.exists() {
        return ScannerOrtWorkerRuntimeStatus {
            ready: false,
            runtime_error: Some(format!(
                "Missing ONNX Runtime library: {}.",
                selected_runtime_library_path
            )),
            ort_build_info: None,
            available_providers: Vec::new(),
            selected_runtime_library_path: Some(selected_runtime_library_path),
            loaded_runtime_library_path: current_loaded_runtime_library_path_text(),
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
                    .with_name("scanner-ort-worker")
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
                state.runtime_library_path = Some(runtime_library_path.to_path_buf());
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
        Some(runtime_library_path),
        loaded_runtime_library_path.as_deref(),
    );
    let runtime_path_mismatch = mismatch_error.is_some();

    ScannerOrtWorkerRuntimeStatus {
        ready: state.environment_ready,
        runtime_error: mismatch_error.or_else(|| state.runtime_error.clone()),
        ort_build_info: state.ort_build_info.clone(),
        available_providers: state.available_providers.clone(),
        selected_runtime_library_path: Some(selected_runtime_library_path),
        loaded_runtime_library_path: loaded_runtime_library_path
            .as_deref()
            .map(scanner_resource::path_to_string),
        runtime_path_mismatch,
    }
}

fn current_loaded_runtime_library_path_text() -> Option<String> {
    runtime_state()
        .lock()
        .expect("ORT runtime state mutex should not be poisoned")
        .runtime_library_path
        .as_deref()
        .map(scanner_resource::path_to_string)
}

fn runtime_path_mismatch_error(selected: Option<&Path>, loaded: Option<&Path>) -> Option<String> {
    let selected = selected?;
    let loaded = loaded?;
    if selected == loaded {
        return None;
    }

    Some(build_runtime_path_mismatch_error(
        &scanner_resource::path_to_string(selected),
        &scanner_resource::path_to_string(loaded),
    ))
}

fn create_model_session(
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

fn configure_scanner_session_builder_for_current_platform(
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

fn build_scanner_execution_providers(
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
