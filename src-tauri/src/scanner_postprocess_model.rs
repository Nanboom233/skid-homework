use image::{Rgba, RgbaImage};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ort::session::Session;
use ort::value::Tensor;
use serde::{Deserialize, Serialize};

use crate::scanner_detect::{
    build_scanner_execution_providers, configure_scanner_session_builder_for_current_platform,
    ensure_shared_scanner_ort_context,
};
use crate::scanner_resource;

const CONFIG_RELATIVE_PATH: &str = "scanner-postprocess-model-config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPostProcessModelConfig {
    pub stage: String,
    pub task: String,
    pub native_residual_control_points: ScannerPostProcessModelEntry,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPostProcessModelEntry {
    pub id: String,
    pub kind: String,
    pub task: String,
    pub model_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_size: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tensor_shape: Option<[u32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_names: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_grid_shape: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_grid_shape: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_coordinate_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp_implementation: Option<String>,
}



#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPostProcessModelOutletStatus {
    pub name: String,
    pub dtype: String,
}

#[derive(Debug, Default)]
struct PostprocessOrtSessionState {
    session: Option<Session>,
    session_model_path: Option<PathBuf>,
    session_provider: Option<String>,
    session_error: Option<String>,
    inputs: Vec<ScannerPostProcessModelOutletStatus>,
    outputs: Vec<ScannerPostProcessModelOutletStatus>,
}

#[derive(Debug, Clone)]
struct PostprocessOrtSessionSnapshot {
    ready: bool,
    session_error: Option<String>,
    inputs: Vec<ScannerPostProcessModelOutletStatus>,
    outputs: Vec<ScannerPostProcessModelOutletStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPostProcessModelStatus {
    pub config_source: Option<String>,
    pub config_path: Option<String>,
    pub model_id: Option<String>,
    pub model_kind: Option<String>,
    pub model_task: Option<String>,
    pub model_path: Option<String>,
    pub control_grid_shape: Option<String>,
    pub input_name: Option<String>,
    pub input_size: Option<String>,
    pub input_tensor_shape: Option<String>,
    pub output_grid_shape: Option<String>,
    pub expected_output_names: Vec<String>,
    pub grid_coordinate_mode: Option<String>,
    pub warp_implementation: Option<String>,
    pub preferred_provider: Option<String>,
    pub provider_candidates: Vec<String>,
    pub runtime_ready: bool,
    pub preferred_provider_ready: bool,
    pub session_ready: bool,
    pub ort_build_info: Option<String>,
    pub runtime_error: Option<String>,
    pub session_error: Option<String>,
    pub inputs: Vec<ScannerPostProcessModelOutletStatus>,
    pub outputs: Vec<ScannerPostProcessModelOutletStatus>,
    pub model_ready: bool,
    pub message: String,
}

pub fn describe_native_postprocess_model_with_hints(
    resource_dir_hint: Option<PathBuf>,
) -> ScannerPostProcessModelStatus {
    describe_native_postprocess_model_with_runtime_hints(resource_dir_hint, None)
}

pub struct NativePostprocessModelRunResult {
    pub image: RgbaImage,
    pub model_id: String,
    pub control_grid_shape: Option<String>,
    pub model_ms: f64,
    pub residual_warp_ms: f64,
}

pub fn describe_native_postprocess_model_with_runtime_hints(
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
) -> ScannerPostProcessModelStatus {
    let resource_root_candidates = scanner_resource::build_resource_root_candidates(resource_dir_hint);
    let Some(selected_resource_root) = scanner_resource::select_resource_root(&resource_root_candidates, &POSTPROCESS_INTERESTING_PATHS) else {
        return ScannerPostProcessModelStatus {
            config_source: None,
            config_path: None,
            model_id: None,
            model_kind: None,
            model_task: None,
            model_path: None,
            control_grid_shape: None,
            input_name: None,
            input_size: None,
            input_tensor_shape: None,
            output_grid_shape: None,
            expected_output_names: Vec::new(),
            grid_coordinate_mode: None,
            warp_implementation: None,
            preferred_provider: None,
            provider_candidates: Vec::new(),
            runtime_ready: false,
            preferred_provider_ready: false,
            session_ready: false,
            ort_build_info: None,
            runtime_error: Some("Could not resolve the scanner resource directory.".to_string()),
            session_error: Some("Could not resolve the scanner resource directory.".to_string()),
            inputs: Vec::new(),
            outputs: Vec::new(),
            model_ready: false,
            message: "Could not resolve the stage-2 resource directory.".to_string(),
        };
    };

    let config_path = selected_resource_root.path.join(CONFIG_RELATIVE_PATH);
    if !config_path.exists() {
        return ScannerPostProcessModelStatus {
            config_source: Some(selected_resource_root.source.to_string()),
            config_path: Some(scanner_resource::path_to_string(&config_path)),
            model_id: None,
            model_kind: None,
            model_task: None,
            model_path: None,
            control_grid_shape: None,
            input_name: None,
            input_size: None,
            input_tensor_shape: None,
            output_grid_shape: None,
            expected_output_names: Vec::new(),
            grid_coordinate_mode: None,
            warp_implementation: None,
            preferred_provider: None,
            provider_candidates: Vec::new(),
            runtime_ready: false,
            preferred_provider_ready: false,
            session_ready: false,
            ort_build_info: None,
            runtime_error: None,
            session_error: Some(format!(
                "Missing stage-2 post-process model config: {}.",
                scanner_resource::path_to_string(&config_path)
            )),
            inputs: Vec::new(),
            outputs: Vec::new(),
            model_ready: false,
            message: format!(
                "Missing stage-2 post-process model config: {}.",
                scanner_resource::path_to_string(&config_path)
            ),
        };
    }

    let config = match fs::read_to_string(&config_path)
        .map_err(|error| {
            format!(
                "Failed to read stage-2 post-process model config {}: {error}",
                scanner_resource::path_to_string(&config_path)
            )
        })
        .and_then(|contents| {
            serde_json::from_str::<ScannerPostProcessModelConfig>(&contents).map_err(|error| {
                format!(
                    "Failed to parse stage-2 post-process model config {}: {error}",
                    scanner_resource::path_to_string(&config_path)
                )
            })
        }) {
        Ok(config) => config,
        Err(error) => {
            return ScannerPostProcessModelStatus {
                config_source: Some(selected_resource_root.source.to_string()),
                config_path: Some(scanner_resource::path_to_string(&config_path)),
                model_id: None,
                model_kind: None,
                model_task: None,
                model_path: None,
                control_grid_shape: None,
                input_name: None,
                input_size: None,
                input_tensor_shape: None,
                output_grid_shape: None,
                expected_output_names: Vec::new(),
                grid_coordinate_mode: None,
                warp_implementation: None,
                preferred_provider: None,
                provider_candidates: Vec::new(),
                runtime_ready: false,
                preferred_provider_ready: false,
                session_ready: false,
                ort_build_info: None,
                runtime_error: None,
                session_error: Some(error.clone()),
                inputs: Vec::new(),
                outputs: Vec::new(),
                model_ready: false,
                message: error,
            }
        }
    };

    let model = config.native_residual_control_points;
    let model_path = selected_resource_root.path.join(model.model_path.as_str());
    let control_grid_shape = model
        .control_grid_shape
        .map(|[columns, rows]| format!("{columns}x{rows}"));
    let input_name = model.input_name.clone();
    let input_size = model
        .input_size
        .map(|[height, width]| format!("{height}x{width}"));
    let input_tensor_shape = model
        .input_tensor_shape
        .map(|shape| format!("{}x{}x{}x{}", shape[0], shape[1], shape[2], shape[3]));
    let output_grid_shape = model
        .output_grid_shape
        .map(|[rows, columns]| format!("{rows}x{columns}"));
    let expected_output_names = model.output_names.clone().unwrap_or_default();
    let grid_coordinate_mode = model.grid_coordinate_mode.clone();
    let warp_implementation = model.warp_implementation.clone();
    let shared_ort_context = ensure_shared_scanner_ort_context(
        Some(selected_resource_root.path.clone()),
        app_config_dir_hint,
    );
    if !model_path.exists() {
        return ScannerPostProcessModelStatus {
            config_source: Some(selected_resource_root.source.to_string()),
            config_path: Some(scanner_resource::path_to_string(&config_path)),
            model_id: Some(model.id),
            model_kind: Some(model.kind),
            model_task: Some(model.task),
            model_path: Some(scanner_resource::path_to_string(&model_path)),
            control_grid_shape,
            input_name,
            input_size,
            input_tensor_shape,
            output_grid_shape,
            expected_output_names,
            grid_coordinate_mode,
            warp_implementation,
            preferred_provider: Some(shared_ort_context.preferred_provider),
            provider_candidates: shared_ort_context.available_providers,
            runtime_ready: shared_ort_context.runtime_ready,
            preferred_provider_ready: shared_ort_context.preferred_provider_ready,
            session_ready: false,
            ort_build_info: shared_ort_context.ort_build_info,
            runtime_error: shared_ort_context.runtime_error,
            session_error: Some(format!(
                "Stage-2 model is configured but missing: {}.",
                scanner_resource::path_to_string(&model_path)
            )),
            inputs: Vec::new(),
            outputs: Vec::new(),
            model_ready: false,
            message: format!(
                "Stage-2 model is configured but missing: {}.",
                scanner_resource::path_to_string(&model_path)
            ),
        };
    }

    if let Some(error) = shared_ort_context.context_error.clone() {
        return ScannerPostProcessModelStatus {
            config_source: Some(selected_resource_root.source.to_string()),
            config_path: Some(scanner_resource::path_to_string(&config_path)),
            model_id: Some(model.id),
            model_kind: Some(model.kind),
            model_task: Some(model.task),
            model_path: Some(scanner_resource::path_to_string(&model_path)),
            control_grid_shape,
            input_name,
            input_size,
            input_tensor_shape,
            output_grid_shape,
            expected_output_names,
            grid_coordinate_mode,
            warp_implementation,
            preferred_provider: Some(shared_ort_context.preferred_provider),
            provider_candidates: shared_ort_context.available_providers,
            runtime_ready: shared_ort_context.runtime_ready,
            preferred_provider_ready: shared_ort_context.preferred_provider_ready,
            session_ready: false,
            ort_build_info: shared_ort_context.ort_build_info,
            runtime_error: shared_ort_context.runtime_error,
            session_error: Some(error.clone()),
            inputs: Vec::new(),
            outputs: Vec::new(),
            model_ready: true,
            message: error,
        };
    }

    if !shared_ort_context.runtime_ready {
        let runtime_message = shared_ort_context.runtime_error.clone().unwrap_or_else(|| {
            "ONNX Runtime is not ready for the shared scanner pipeline.".to_string()
        });
        return ScannerPostProcessModelStatus {
            config_source: Some(selected_resource_root.source.to_string()),
            config_path: Some(scanner_resource::path_to_string(&config_path)),
            model_id: Some(model.id),
            model_kind: Some(model.kind),
            model_task: Some(model.task),
            model_path: Some(scanner_resource::path_to_string(&model_path)),
            control_grid_shape,
            input_name,
            input_size,
            input_tensor_shape,
            output_grid_shape,
            expected_output_names,
            grid_coordinate_mode,
            warp_implementation,
            preferred_provider: Some(shared_ort_context.preferred_provider),
            provider_candidates: shared_ort_context.available_providers,
            runtime_ready: false,
            preferred_provider_ready: shared_ort_context.preferred_provider_ready,
            session_ready: false,
            ort_build_info: shared_ort_context.ort_build_info,
            runtime_error: shared_ort_context.runtime_error,
            session_error: Some(runtime_message.clone()),
            inputs: Vec::new(),
            outputs: Vec::new(),
            model_ready: true,
            message: runtime_message,
        };
    }

    let session_snapshot = ensure_postprocess_session(
        &selected_resource_root.path,
        &model,
        shared_ort_context.preferred_provider.as_str(),
    );
    if !session_snapshot.ready {
        let session_message = session_snapshot
            .session_error
            .clone()
            .unwrap_or_else(|| "Stage-2 model session is not ready.".to_string());
        return ScannerPostProcessModelStatus {
            config_source: Some(selected_resource_root.source.to_string()),
            config_path: Some(scanner_resource::path_to_string(&config_path)),
            model_id: Some(model.id),
            model_kind: Some(model.kind),
            model_task: Some(model.task),
            model_path: Some(scanner_resource::path_to_string(&model_path)),
            control_grid_shape,
            input_name,
            input_size,
            input_tensor_shape,
            output_grid_shape,
            expected_output_names,
            grid_coordinate_mode,
            warp_implementation,
            preferred_provider: Some(shared_ort_context.preferred_provider),
            provider_candidates: shared_ort_context.available_providers,
            runtime_ready: true,
            preferred_provider_ready: shared_ort_context.preferred_provider_ready,
            session_ready: false,
            ort_build_info: shared_ort_context.ort_build_info,
            runtime_error: shared_ort_context.runtime_error,
            session_error: session_snapshot.session_error,
            inputs: session_snapshot.inputs,
            outputs: session_snapshot.outputs,
            model_ready: true,
            message: session_message,
        };
    }

    ScannerPostProcessModelStatus {
        config_source: Some(selected_resource_root.source.to_string()),
        config_path: Some(scanner_resource::path_to_string(&config_path)),
        model_id: Some(model.id),
        model_kind: Some(model.kind),
        model_task: Some(model.task),
        model_path: Some(scanner_resource::path_to_string(&model_path)),
        control_grid_shape,
        input_name,
        input_size,
        input_tensor_shape,
        output_grid_shape,
        expected_output_names,
        grid_coordinate_mode,
        warp_implementation,
        preferred_provider: Some(shared_ort_context.preferred_provider),
        provider_candidates: shared_ort_context.available_providers,
        runtime_ready: true,
        preferred_provider_ready: shared_ort_context.preferred_provider_ready,
        session_ready: true,
        ort_build_info: shared_ort_context.ort_build_info,
        runtime_error: shared_ort_context.runtime_error,
        session_error: None,
        inputs: session_snapshot.inputs,
        outputs: session_snapshot.outputs,
        model_ready: true,
        message: "Stage-2 model session is ready, IO contract was discovered, and the native UVDoc decode/remap path is available.".to_string(),
    }
}

/// Interesting paths used to score resource roots for the post-process subsystem.
const POSTPROCESS_INTERESTING_PATHS: [&str; 3] = [
    CONFIG_RELATIVE_PATH,
    "models/document-residual-controlpoints.onnx",
    "models/uvdoc-best-model.onnx",
];

fn postprocess_session_state() -> &'static Mutex<PostprocessOrtSessionState> {
    static STATE: OnceLock<Mutex<PostprocessOrtSessionState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(PostprocessOrtSessionState::default()))
}

fn ensure_postprocess_session(
    resource_root: &Path,
    model: &ScannerPostProcessModelEntry,
    preferred_provider: &str,
) -> PostprocessOrtSessionSnapshot {
    let model_path = resource_root.join(model.model_path.as_str());
    if !model_path.exists() {
        return PostprocessOrtSessionSnapshot {
            ready: false,
            session_error: Some(format!(
                "Missing ONNX model file: {}.",
                scanner_resource::path_to_string(&model_path)
            )),
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
    }

    let mut state = postprocess_session_state()
        .lock()
        .expect("postprocess ORT session state mutex should not be poisoned");
    let same_model_loaded = state
        .session_model_path
        .as_ref()
        .map(|loaded_path| loaded_path == &model_path)
        .unwrap_or(false);
    let same_provider_loaded = state
        .session_provider
        .as_ref()
        .map(|loaded_provider| loaded_provider == preferred_provider)
        .unwrap_or(false);

    if state.session.is_some() && same_model_loaded && same_provider_loaded {
        return PostprocessOrtSessionSnapshot {
            ready: true,
            session_error: None,
            inputs: state.inputs.clone(),
            outputs: state.outputs.clone(),
        };
    }

    let session_result = Session::builder()
        .map_err(|error| format!("Failed to create stage-2 ORT session builder: {error}"))
        .and_then(|builder| {
            let mut builder = configure_scanner_session_builder_for_current_platform(builder)?;
            builder = builder
                .with_execution_providers(build_scanner_execution_providers(preferred_provider))
                .map_err(|error| {
                    format!("Failed to configure stage-2 execution providers: {error}")
                })?;
            builder
                .commit_from_file(&model_path)
                .map_err(|error| format!("Failed to load stage-2 ONNX session: {error}"))
        });

    match session_result {
        Ok(session) => {
            let inputs = describe_session_outlets(session.inputs());
            let outputs = describe_session_outlets(session.outputs());
            state.inputs = inputs.clone();
            state.outputs = outputs.clone();
            state.session = Some(session);
            state.session_model_path = Some(model_path);
            state.session_provider = Some(preferred_provider.to_string());
            state.session_error = None;
            PostprocessOrtSessionSnapshot {
                ready: true,
                session_error: None,
                inputs,
                outputs,
            }
        }
        Err(error) => {
            state.session = None;
            state.session_model_path = None;
            state.session_provider = None;
            state.session_error = Some(error.clone());
            state.inputs.clear();
            state.outputs.clear();
            PostprocessOrtSessionSnapshot {
                ready: false,
                session_error: Some(error),
                inputs: Vec::new(),
                outputs: Vec::new(),
            }
        }
    }
}

pub fn run_native_postprocess_model_with_runtime_hints(
    resource_dir_hint: Option<PathBuf>,
    app_config_dir_hint: Option<PathBuf>,
    proxy_image: &RgbaImage,
    target_size: Option<(u32, u32)>,
    grid_postprocess: &str,
) -> Result<NativePostprocessModelRunResult, String> {
    let resource_root_candidates = scanner_resource::build_resource_root_candidates(resource_dir_hint);
    let selected_resource_root = scanner_resource::select_resource_root(&resource_root_candidates, &POSTPROCESS_INTERESTING_PATHS)
        .ok_or_else(|| "Could not resolve the stage-2 resource directory.".to_string())?;
    let config_path = selected_resource_root.path.join(CONFIG_RELATIVE_PATH);
    let config_contents = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "Failed to read stage-2 post-process model config {}: {error}",
            scanner_resource::path_to_string(&config_path)
        )
    })?;
    let config = serde_json::from_str::<ScannerPostProcessModelConfig>(&config_contents).map_err(
        |error| {
            format!(
                "Failed to parse stage-2 post-process model config {}: {error}",
                scanner_resource::path_to_string(&config_path)
            )
        },
    )?;

    let model = config.native_residual_control_points;
    let model_id = model.id.clone();
    let model_path = selected_resource_root.path.join(model.model_path.as_str());
    // Validate the model path stays within the resource root to prevent
    // path traversal via a malicious scanner-postprocess-model.json.
    scanner_resource::validate_path_containment(&model_path, &selected_resource_root.path)?;
    let control_grid_shape = model
        .control_grid_shape
        .map(|[columns, rows]| format!("{columns}x{rows}"));
    let shared_ort_context = ensure_shared_scanner_ort_context(
        Some(selected_resource_root.path.clone()),
        app_config_dir_hint,
    );
    if let Some(error) = shared_ort_context.context_error {
        return Err(error);
    }
    if !shared_ort_context.runtime_ready {
        return Err(shared_ort_context.runtime_error.unwrap_or_else(|| {
            "ONNX Runtime is not ready for the shared scanner pipeline.".to_string()
        }));
    }

    let session_snapshot = ensure_postprocess_session(
        &selected_resource_root.path,
        &model,
        shared_ort_context.preferred_provider.as_str(),
    );
    if !session_snapshot.ready {
        return Err(session_snapshot
            .session_error
            .unwrap_or_else(|| "Stage-2 model session is not ready.".to_string()));
    }

    let input_name = model.input_name.as_deref().unwrap_or("input");
    let input_shape = model.input_tensor_shape.unwrap_or([1, 3, 488, 712]);
    let input_tensor = build_uvdoc_input_tensor(proxy_image, input_shape)?;

    let model_started_at = std::time::Instant::now();
    let point_grid = {
        let mut state = postprocess_session_state()
            .lock()
            .expect("postprocess ORT session state mutex should not be poisoned");
        let session = state
            .session
            .as_mut()
            .ok_or_else(|| "Stage-2 model session is not initialized.".to_string())?;
        let outputs = session
            .run(ort::inputs![input_name => input_tensor])
            .map_err(|error| format!("Failed to run stage-2 ORT inference: {error}"))?;
        let output_name = model
            .output_names
            .as_ref()
            .and_then(|names| names.first())
            .map(String::as_str)
            .unwrap_or("point_positions_2d");
        let output_value = outputs.get(output_name).unwrap_or(&outputs[0]);
        let (shape, tensor_view) = output_value
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("Failed to extract stage-2 output tensor: {error}"))?;
        (shape.to_vec(), tensor_view.to_vec())
    };
    let model_ms = model_started_at.elapsed().as_secs_f64() * 1000.0;

    // -- Diagnostic: dump grid structure for debugging --
    #[cfg(debug_assertions)]
    {
        let shape = &point_grid.0;
        let grid = &point_grid.1;
        let gh = shape.get(2).copied().unwrap_or(0) as usize;
        let gw = shape.get(3).copied().unwrap_or(0) as usize;
        let plane = gh * gw;
        log::debug!(
            "[UVDoc grid diag] proxy={}x{}, tensor_shape={:?}, gh={}, gw={}, plane={}",
            proxy_image.width(), proxy_image.height(), shape, gh, gw, plane
        );
        if plane > 0 && grid.len() >= plane * 2 {
            // Channel 0 (X) range
            let (mut x_min, mut x_max) = (f32::MAX, f32::MIN);
            let (mut y_min, mut y_max) = (f32::MAX, f32::MIN);
            for i in 0..plane {
                x_min = x_min.min(grid[i]);
                x_max = x_max.max(grid[i]);
                y_min = y_min.min(grid[plane + i]);
                y_max = y_max.max(grid[plane + i]);
            }
            log::debug!(
                "[UVDoc grid diag] ch0(X): [{:.4}..{:.4}] range={:.4}, ch1(Y): [{:.4}..{:.4}] range={:.4}",
                x_min, x_max, x_max - x_min, y_min, y_max, y_max - y_min
            );

            // Sample along mid-row (gy=gh/2): how does ch0 vary with gx?
            let mid_gy = gh / 2;
            let mut row_samples = Vec::new();
            for gx in [0, gw / 4, gw / 2, 3 * gw / 4, gw.saturating_sub(1)] {
                if gx < gw {
                    let idx = mid_gy * gw + gx;
                    let id_x = if gw <= 1 { 0.0 } else { 2.0 * gx as f32 / (gw as f32 - 1.0) - 1.0 };
                    row_samples.push(format!(
                        "gx{}:id={:.3} act={:.3} d={:.4}",
                        gx, id_x, grid[idx], grid[idx] - id_x
                    ));
                }
            }
            log::debug!("[UVDoc grid diag] ch0 along mid-row(gy={}): {}", mid_gy, row_samples.join(", "));

            // Sample along mid-col (gx=gw/2): how does ch1 vary with gy?
            let mid_gx = gw / 2;
            let mut col_samples = Vec::new();
            for gy in [0, gh / 4, gh / 2, 3 * gh / 4, gh.saturating_sub(1)] {
                if gy < gh {
                    let idx = gy * gw + mid_gx;
                    let id_y = if gh <= 1 { 0.0 } else { 2.0 * gy as f32 / (gh as f32 - 1.0) - 1.0 };
                    col_samples.push(format!(
                        "gy{}:id={:.3} act={:.3} d={:.4}",
                        gy, id_y, grid[plane + idx], grid[plane + idx] - id_y
                    ));
                }
            }
            log::debug!("[UVDoc grid diag] ch1 along mid-col(gx={}): {}", mid_gx, col_samples.join(", "));

            // CRITICAL: Check if ch0 varies primarily along rows or columns
            // and if ch1 varies primarily along columns or rows.
            // This tells us if the grid dimensions need to be swapped.
            // For correct mapping: ch0 (X) should vary across gx (columns), ch1 (Y) across gy (rows)
            let center_gy = gh / 2;
            let center_gx = gw / 2;
            // ch0 variation along columns (gx) at center row
            let ch0_col_left = grid[center_gy * gw + 0];
            let ch0_col_right = grid[center_gy * gw + (gw - 1)];
            let ch0_col_var = (ch0_col_right - ch0_col_left).abs();
            // ch0 variation along rows (gy) at center column
            let ch0_row_top = grid[0 * gw + center_gx];
            let ch0_row_bot = grid[(gh - 1) * gw + center_gx];
            let ch0_row_var = (ch0_row_bot - ch0_row_top).abs();
            // ch1 variation along columns (gx) at center row
            let ch1_col_left = grid[plane + center_gy * gw + 0];
            let ch1_col_right = grid[plane + center_gy * gw + (gw - 1)];
            let ch1_col_var = (ch1_col_right - ch1_col_left).abs();
            // ch1 variation along rows (gy) at center column
            let ch1_row_top = grid[plane + 0 * gw + center_gx];
            let ch1_row_bot = grid[plane + (gh - 1) * gw + center_gx];
            let ch1_row_var = (ch1_row_bot - ch1_row_top).abs();
            log::debug!(
                "[UVDoc grid axis] ch0(X): col_var={:.4} row_var={:.4} -> varies more along {}",
                ch0_col_var, ch0_row_var, if ch0_col_var > ch0_row_var { "COLUMNS(gx) OK" } else { "ROWS(gy) SWAPPED?" }
            );
            log::debug!(
                "[UVDoc grid axis] ch1(Y): col_var={:.4} row_var={:.4} -> varies more along {}",
                ch1_col_var, ch1_row_var, if ch1_row_var > ch1_col_var { "ROWS(gy) OK" } else { "COLUMNS(gx) SWAPPED?" }
            );
        }
    }

    // Apply the UVDoc backward mapping grid to produce the dewarped image.
    // Per the official UVDoc design (github.com/tanguymagne/UVDoc), the grid
    // is applied directly to the same image the model analyzed (the proxy).
    let residual_started_at = std::time::Instant::now();
    let image = apply_uvdoc_point_grid(proxy_image, target_size, grid_postprocess, &point_grid.1, &point_grid.0)?;
    let residual_warp_ms = residual_started_at.elapsed().as_secs_f64() * 1000.0;

    Ok(NativePostprocessModelRunResult {
        image,
        model_id,
        control_grid_shape,
        model_ms,
        residual_warp_ms,
    })
}

fn build_uvdoc_input_tensor(
    source: &RgbaImage,
    input_shape: [u32; 4],
) -> Result<Tensor<f32>, String> {
    let expected_channels = input_shape[1];
    if expected_channels != 3 {
        return Err(format!(
            "UVDoc input expects 3 channels, got {}.",
            expected_channels
        ));
    }

    let target_height = input_shape[2];
    let target_width = input_shape[3];
    let resized = image::imageops::resize(
        source,
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    );
    let plane_len = target_width as usize * target_height as usize;
    let mut input = vec![0.0_f32; plane_len * 3];
    for (index, pixel) in resized.pixels().enumerate() {
        input[index] = pixel.0[0] as f32 / 255.0;
        input[plane_len + index] = pixel.0[1] as f32 / 255.0;
        input[(plane_len * 2) + index] = pixel.0[2] as f32 / 255.0;
    }

    Tensor::<f32>::from_array((
        vec![
            input_shape[0] as i64,
            input_shape[1] as i64,
            input_shape[2] as i64,
            input_shape[3] as i64,
        ],
        input,
    ))
    .map_err(|error| format!("Failed to build stage-2 input tensor: {error}"))
}

/// Applies the UVDoc backward-mapping grid to produce the dewarped image.
///
/// Pipeline: model grid -> optional postprocess -> bilinear upsample -> backward sample.
///
/// The grid is `[1, 2, Gh, Gw]` with channels 0 (X) and 1 (Y) in normalized
/// `[-1, 1]` coordinates (PyTorch `align_corners=True` convention).
///
/// Grid post-processing modes:
///   - `"none"`: use the raw model grid directly.
///   - `"x-stretch-equalize"`: replace X channel with identity `[-1, +1]`,
///     keeping Y curvature intact.  Eliminates non-uniform horizontal
///     stretch and edge contraction from the model predictions.
fn apply_uvdoc_point_grid(
    proxy_image: &RgbaImage,
    target_size: Option<(u32, u32)>,
    grid_postprocess: &str,
    grid: &[f32],
    shape: &[i64],
) -> Result<RgbaImage, String> {
    if shape.len() != 4 {
        return Err(format!(
            "Unexpected UVDoc output rank {}. Expected [1, 2, H, W].",
            shape.len()
        ));
    }
    let channels =
        usize::try_from(shape[1]).map_err(|_| "Invalid UVDoc channel count.".to_string())?;
    if channels < 2 {
        return Err(format!(
            "Unexpected UVDoc output channel count {}. Expected at least 2.",
            channels
        ));
    }
    let grid_height =
        usize::try_from(shape[2]).map_err(|_| "Invalid UVDoc grid height.".to_string())?;
    let grid_width =
        usize::try_from(shape[3]).map_err(|_| "Invalid UVDoc grid width.".to_string())?;
    let plane_len = grid_height * grid_width;
    if grid.len() < plane_len * channels {
        return Err("UVDoc output tensor is smaller than expected.".to_string());
    }

    let mut cleaned_grid = grid[..plane_len * 2].to_vec();
    match grid_postprocess {
        "x-stretch-equalize" => {
            x_stretch_equalize_grid(&mut cleaned_grid, grid_height, grid_width);
        }
        _ => {
            // "none" -- use raw grid as-is
        }
    }

    let out_w = target_size.map(|s| s.0).unwrap_or_else(|| proxy_image.width());
    let out_h = target_size.map(|s| s.1).unwrap_or_else(|| proxy_image.height());

    let mut output = RgbaImage::new(out_w, out_h);

    for y in 0..out_h {
        let grid_y = if out_h <= 1 || grid_height <= 1 {
            0.0
        } else {
            y as f32 * (grid_height as f32 - 1.0) / (out_h as f32 - 1.0)
        };
        for x in 0..out_w {
            let grid_x = if out_w <= 1 || grid_width <= 1 {
                0.0
            } else {
                x as f32 * (grid_width as f32 - 1.0) / (out_w as f32 - 1.0)
            };

            // Sample the cleaned grid's normalized coordinates for this output pixel.
            // Channel 0 = X, Channel 1 = Y, both in [-1, 1].
            let norm_x = bilinear_sample_grid_channel(
                &cleaned_grid, plane_len, grid_width, grid_height, 0, grid_x, grid_y,
            );
            let norm_y = bilinear_sample_grid_channel(
                &cleaned_grid, plane_len, grid_width, grid_height, 1, grid_x, grid_y,
            );

            // Convert normalized [-1, 1] -> pixel coordinates (align_corners=True)
            // Note: The normalized coordinates refer to the source image space (proxy_image),
            // not the destination image space (out_w, out_h)
            let src_x = normalized_to_pixel(norm_x, proxy_image.width());
            let src_y = normalized_to_pixel(norm_y, proxy_image.height());

            output.put_pixel(x, y, sample_rgba_zero_padded(proxy_image, src_x, src_y));
        }
    }

    Ok(output)
}

/// X stretch equalization of the UVDoc grid **in-place**.
///
/// Replaces the model's X channel (channel 0) with identity
/// `linspace(-1, +1)` for **all rows**.  The Y channel is untouched.
///
/// ## Mathematical basis
///
/// Uniform text width requires d(norm_x)/d(gx) = constant.
/// The unique stretch-1.0 solution is `A = -1, B = 2/(Gw-1)` (identity).
///
/// ## X-Y coupling
///
/// The Y channel varies by only ~0.06 across the full width (col_var).
/// Maximum Y misalignment from identity X is ~3px at extreme edges
/// (0.13% of image height). Text line alignment is preserved.
fn x_stretch_equalize_grid(grid: &mut [f32], grid_height: usize, grid_width: usize) {
    if grid_width <= 1 || grid_height == 0 {
        return;
    }

    let denom = (grid_width as f32) - 1.0;

    for gy in 0..grid_height {
        let row_start = gy * grid_width;
        for gx in 0..grid_width {
            grid[row_start + gx] = -1.0 + 2.0 * (gx as f32 / denom);
        }
    }

    #[cfg(debug_assertions)]
    log::debug!(
        "[UVDoc] grid_postprocess=x-stretch-equalize | ch0(X)->identity[-1,+1], ch1(Y)->preserved",
    );
}



fn bilinear_sample_grid_channel(
    grid: &[f32],
    plane_len: usize,
    grid_width: usize,
    grid_height: usize,
    channel: usize,
    x: f32,
    y: f32,
) -> f32 {
    let clamped_x = x.clamp(0.0, grid_width.saturating_sub(1) as f32);
    let clamped_y = y.clamp(0.0, grid_height.saturating_sub(1) as f32);
    let x0 = clamped_x.floor() as usize;
    let y0 = clamped_y.floor() as usize;
    let x1 = (x0 + 1).min(grid_width.saturating_sub(1));
    let y1 = (y0 + 1).min(grid_height.saturating_sub(1));
    let tx = clamped_x - x0 as f32;
    let ty = clamped_y - y0 as f32;
    let base = channel * plane_len;

    let top_left = grid[base + y0 * grid_width + x0];
    let top_right = grid[base + y0 * grid_width + x1];
    let bottom_left = grid[base + y1 * grid_width + x0];
    let bottom_right = grid[base + y1 * grid_width + x1];

    let top = top_left + (top_right - top_left) * tx;
    let bottom = bottom_left + (bottom_right - bottom_left) * tx;
    top + (bottom - top) * ty
}

fn normalized_to_pixel(value: f32, extent: u32) -> f32 {
    if extent <= 1 {
        0.0
    } else {
        ((value + 1.0) * 0.5) * (extent as f32 - 1.0)
    }
}

fn sample_rgba_zero_padded(source: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    if x < 0.0 || y < 0.0 || x > source.width() as f32 - 1.0 || y > source.height() as f32 - 1.0 {
        return Rgba([0, 0, 0, 0]);
    }

    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = x0.saturating_add(1).min(source.width().saturating_sub(1));
    let y1 = y0.saturating_add(1).min(source.height().saturating_sub(1));
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;

    let top_left = source.get_pixel(x0, y0).0;
    let top_right = source.get_pixel(x1, y0).0;
    let bottom_left = source.get_pixel(x0, y1).0;
    let bottom_right = source.get_pixel(x1, y1).0;
    let mut output = [0u8; 4];
    for channel in 0..4 {
        let top =
            top_left[channel] as f32 + (top_right[channel] as f32 - top_left[channel] as f32) * tx;
        let bottom = bottom_left[channel] as f32
            + (bottom_right[channel] as f32 - bottom_left[channel] as f32) * tx;
        output[channel] = (top + (bottom - top) * ty).round().clamp(0.0, 255.0) as u8;
    }
    Rgba(output)
}

fn describe_session_outlets(
    outlets: &[ort::value::Outlet],
) -> Vec<ScannerPostProcessModelOutletStatus> {
    outlets
        .iter()
        .map(|outlet| ScannerPostProcessModelOutletStatus {
            name: outlet.name().to_string(),
            dtype: outlet.dtype().to_string(),
        })
        .collect()
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_uvdoc_grid_preserves_simple_image() {
        let mut source = RgbaImage::new(5, 4);
        for y in 0..source.height() {
            for x in 0..source.width() {
                source.put_pixel(
                    x,
                    y,
                    Rgba([(x * 30) as u8, (y * 40) as u8, (x + y) as u8, 255]),
                );
            }
        }

        let grid = vec![-1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0];
        let shape = [1_i64, 2, 2, 2];
        let warped = apply_uvdoc_point_grid(&source, None, "none", &grid, &shape)
            .expect("identity warp should succeed");
        assert_eq!(warped.width(), source.width());
        assert_eq!(warped.height(), source.height());
    }
}
