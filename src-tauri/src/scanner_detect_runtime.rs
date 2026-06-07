use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ort::ep::ExecutionProvider as _;
use ort::{
    ep,
    session::{builder::SessionBuilder, Session},
    value::Tensor,
};

use crate::scanner_detect_config::{
    build_resource_statuses, preferred_provider_from_config, resolve_scanner_detect_config,
    resource_specs_for_current_platform, runtime_library_path_for_current_platform,
    select_model_variant, ResolvedScannerModel, DETECT_INTERESTING_PATHS,
};
use crate::scanner_platform;
use crate::scanner_resource;

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
pub(crate) struct OrtRuntimeSnapshot {
    pub(crate) ready: bool,
    pub(crate) runtime_error: Option<String>,
    pub(crate) ort_build_info: Option<String>,
    pub(crate) available_providers: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct OrtSessionSnapshot {
    pub(crate) ready: bool,
    pub(crate) session_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DetectionContextCacheKey {
    resource_dir_hint: Option<PathBuf>,
    installed_assets_current_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct DetectionRuntimeContext {
    pub(crate) resource_base_dir: PathBuf,
    pub(crate) selected_model: Option<ResolvedScannerModel>,
    pub(crate) preferred_provider: String,
}

#[derive(Debug, Default)]
struct DetectionContextCacheState {
    key: Option<DetectionContextCacheKey>,
    context: Option<DetectionRuntimeContext>,
    error: Option<String>,
}

pub(crate) fn resolve_detection_runtime_context(
    resource_dir_hint: Option<PathBuf>,
    installed_assets_current_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> Result<DetectionRuntimeContext, String> {
    let key = DetectionContextCacheKey {
        resource_dir_hint: resource_dir_hint.clone(),
        installed_assets_current_dir_hint: installed_assets_current_dir_hint.clone(),
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

    let result = build_detection_runtime_context(
        resource_dir_hint,
        installed_assets_current_dir_hint,
        app_config_dir_hint,
    );
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

pub(crate) fn probe_ort_runtime(resource_base_dir: Option<&Path>) -> OrtRuntimeSnapshot {
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

pub(crate) fn ensure_ort_session(
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

pub(crate) fn run_ort_session_inference(
    input_name: &str,
    input_tensor: Tensor<f32>,
    output_name: &str,
) -> Result<(Vec<f32>, Vec<i64>), String> {
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
        // MutexGuard is dropped here; the lock is released before heatmap processing.
    };

    Ok((heatmap_data, heatmap_shape))
}

fn runtime_state() -> &'static Mutex<OrtRuntimeState> {
    static STATE: OnceLock<Mutex<OrtRuntimeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(OrtRuntimeState::default()))
}

fn detection_context_cache() -> &'static Mutex<DetectionContextCacheState> {
    static STATE: OnceLock<Mutex<DetectionContextCacheState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(DetectionContextCacheState::default()))
}

fn build_detection_runtime_context(
    resource_dir_hint: Option<PathBuf>,
    installed_assets_current_dir_hint: Option<PathBuf>,
    _app_config_dir_hint: Option<PathBuf>,
) -> Result<DetectionRuntimeContext, String> {
    let resource_root_candidates = scanner_resource::build_resource_root_candidates(
        resource_dir_hint,
        installed_assets_current_dir_hint,
    );
    let selected_resource_root = scanner_resource::select_resource_root(
        &resource_root_candidates,
        &DETECT_INTERESTING_PATHS,
    )
    .ok_or_else(|| "Could not resolve the scanner resource directory.".to_string())?;
    let resource_base_dir = selected_resource_root.path;
    let config_handle = resolve_scanner_detect_config();
    let resource_specs = resource_specs_for_current_platform(Some(&config_handle.config));
    let resources = build_resource_statuses(Some(&resource_base_dir), &resource_specs);
    let selected_model = select_model_variant(&config_handle.config, &resources);
    let preferred_provider = preferred_provider_from_config(&config_handle.config);

    Ok(DetectionRuntimeContext {
        resource_base_dir,
        selected_model,
        preferred_provider,
    })
}

fn configure_scanner_session_builder_for_current_platform(
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
