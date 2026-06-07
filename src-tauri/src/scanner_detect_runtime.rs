use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ort::{session::Session, value::Tensor};

use crate::scanner_detect_config::{
    build_resource_statuses, preferred_provider_from_config, resolve_scanner_detect_config,
    resource_specs_for_current_platform, select_model_variant, ResolvedScannerModel,
    DETECT_INTERESTING_PATHS,
};
use crate::scanner_ort::{create_model_session, probe_ort_runtime as probe_shared_ort_runtime};
use crate::scanner_resource;

#[derive(Debug, Default)]
struct OrtRuntimeState {
    session: Option<Session>,
    session_model_path: Option<PathBuf>,
    session_error: Option<String>,
}

pub(crate) use crate::scanner_ort::OrtRuntimeSnapshot;

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
    probe_shared_ort_runtime(resource_base_dir)
}

pub(crate) fn ensure_ort_session(
    resource_base_dir: Option<&Path>,
    model: &ResolvedScannerModel,
    _preferred_provider: &str,
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

    let runtime_snapshot = probe_ort_runtime(Some(resource_base_dir));
    if !runtime_snapshot.ready {
        return OrtSessionSnapshot {
            ready: false,
            session_error: Some(runtime_snapshot.runtime_error.unwrap_or_else(|| {
                "ONNX Runtime environment is not initialized, so the model session cannot be created."
                    .to_string()
            })),
        };
    }

    let mut state = runtime_state()
        .lock()
        .expect("ORT runtime state mutex should not be poisoned");

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

    let session_result = create_model_session(&model_path, &runtime_snapshot.available_providers);

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

pub fn reset_scanner_detect_runtime_caches() {
    {
        let mut cache = detection_context_cache()
            .lock()
            .expect("detection context cache mutex should not be poisoned");
        *cache = DetectionContextCacheState::default();
    }

    let mut state = runtime_state()
        .lock()
        .expect("ORT runtime state mutex should not be poisoned");
    state.session = None;
    state.session_model_path = None;
    state.session_error = None;
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
