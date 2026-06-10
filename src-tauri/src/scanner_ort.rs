use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};
#[cfg(not(test))]
use std::thread;
#[cfg(not(test))]
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{command, AppHandle, Manager};

use crate::scanner_assets::{self, ScannerAssetsError};
use crate::scanner_ort_protocol::{
    read_framed_json, write_framed_json, ScannerOrtModelStatus, ScannerOrtWorkerModelRequest,
    ScannerOrtWorkerOperation, ScannerOrtWorkerProbeResult, ScannerOrtWorkerRequest,
    ScannerOrtWorkerRequestPayload, ScannerOrtWorkerResourceRequest, ScannerOrtWorkerResponse,
    ScannerOrtWorkerResponsePayload, ScannerOrtWorkerRuntimeStatus, LINUX_CUDA_RELATIVE_PATH,
    LINUX_ORT_RELATIVE_PATH, LINUX_ORT_SONAME_RELATIVE_PATH, LINUX_ORT_VERSIONED_RELATIVE_PATH,
    LINUX_SHARED_RELATIVE_PATH, LINUX_TENSORRT_RELATIVE_PATH, SCANNER_ORT_MODEL_MANIFEST, STAGE,
    WINDOWS_DIRECTML_RELATIVE_PATH, WINDOWS_ORT_RELATIVE_PATH, WINDOWS_ORT_SHARED_RELATIVE_PATH,
};
use crate::{scanner_platform, scanner_resource};

#[cfg(not(test))]
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(not(test))]
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtResourceStatus {
    pub relative_path: String,
    pub resolved_path: Option<String>,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtResourceTreeEntry {
    pub relative_path: String,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub size_bytes: Option<u64>,
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
    pub resource_tree: Vec<ScannerOrtResourceTreeEntry>,
    pub models: Vec<ScannerOrtModelStatus>,
    pub message: String,
}

struct OrtWorkerState {
    child: Option<OrtWorkerChild>,
    next_request_id: u64,
    asset_mutation_active: bool,
    last_exit: Option<String>,
    last_protocol_error: Option<String>,
}

impl Default for OrtWorkerState {
    fn default() -> Self {
        Self {
            child: None,
            next_request_id: 1,
            asset_mutation_active: false,
            last_exit: None,
            last_protocol_error: None,
        }
    }
}

struct OrtWorkerChild {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    launch_label: String,
}

enum OrtWorkerLaunchCommand {
    Direct(PathBuf),
    CargoRun { manifest_path: PathBuf },
}

impl OrtWorkerLaunchCommand {
    fn label(&self) -> String {
        match self {
            OrtWorkerLaunchCommand::Direct(path) => scanner_resource::path_to_string(path),
            OrtWorkerLaunchCommand::CargoRun { manifest_path } => {
                format!(
                    "cargo run --manifest-path {}",
                    scanner_resource::path_to_string(manifest_path)
                )
            }
        }
    }
}

#[cfg(not(test))]
pub(crate) struct ScannerOrtAssetMutationGuard {
    active: bool,
}

#[cfg(not(test))]
impl Drop for ScannerOrtAssetMutationGuard {
    fn drop(&mut self) {
        if self.active {
            worker_state()
                .lock()
                .expect("scanner ORT worker mutex should not be poisoned")
                .asset_mutation_active = false;
        }
    }
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

#[cfg(not(test))]
pub(crate) fn begin_ort_asset_mutation() -> Result<ScannerOrtAssetMutationGuard, ScannerAssetsError>
{
    let mut state = worker_state()
        .lock()
        .expect("scanner ORT worker mutex should not be poisoned");
    if state.asset_mutation_active {
        return Err(ScannerAssetsError::with_details(
            "runtime.worker.assetMutationInProgress",
            true,
            "Scanner ORT worker is already blocked for an asset operation.",
        ));
    }

    state.asset_mutation_active = true;
    if let Err(error) = stop_worker_locked(&mut state) {
        state.asset_mutation_active = false;
        return Err(ScannerAssetsError::with_details(
            "runtime.worker.stopFailed",
            true,
            format!("Failed to stop scanner ORT worker before asset mutation: {error}"),
        ));
    }

    Ok(ScannerOrtAssetMutationGuard { active: true })
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
    let resource_tree = build_current_resource_tree(installed_assets_current_dir_hint.as_deref());
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
    let worker_probe = probe_ort_worker(resource_base_dir.as_deref());
    let runtime_status = worker_probe.runtime;
    let preferred_provider_ready = scanner_platform::is_provider_available(
        &preferred_provider,
        &runtime_status.available_providers,
    );

    let models = worker_probe.models;
    let model_load_ready = runtime_status.ready && models.iter().all(|model| model.session_ready);
    let message = build_probe_message(&runtime_status, &models, model_load_ready);
    let (selected_runtime_library_path, loaded_runtime_library_path, runtime_library_path) =
        runtime_library_path_status_texts(&runtime_status);

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
        runtime_path_mismatch: runtime_status.runtime_path_mismatch,
        runtime_ready: runtime_status.ready,
        model_load_ready,
        preferred_provider,
        preferred_provider_ready,
        provider_candidates,
        available_providers: runtime_status.available_providers,
        ort_build_info: runtime_status.ort_build_info,
        runtime_error: runtime_status.runtime_error,
        resources,
        resource_tree,
        models,
        message,
    }
}

fn probe_ort_worker(resource_base_dir: Option<&Path>) -> ScannerOrtWorkerProbeResult {
    let Some(resource_base_dir) = resource_base_dir else {
        return worker_error_probe(
            None,
            "Could not resolve the scanner resource directory.".to_string(),
        );
    };

    match send_worker_probe(resource_base_dir) {
        Ok(probe) => probe,
        Err(error) => worker_error_probe(Some(resource_base_dir), error),
    }
}

fn worker_error_probe(
    resource_base_dir: Option<&Path>,
    runtime_error: String,
) -> ScannerOrtWorkerProbeResult {
    let selected_runtime_library_path = resource_base_dir
        .and_then(runtime_library_path_for_current_platform)
        .map(|path| scanner_resource::path_to_string(&path));
    let models = build_model_statuses_without_worker(
        resource_base_dir,
        "Scanner ORT worker is not ready, so the model session cannot be created.",
    );

    ScannerOrtWorkerProbeResult {
        runtime: ScannerOrtWorkerRuntimeStatus {
            ready: false,
            runtime_error: Some(runtime_error),
            ort_build_info: None,
            available_providers: Vec::new(),
            selected_runtime_library_path,
            loaded_runtime_library_path: None,
            runtime_path_mismatch: false,
        },
        models,
    }
}

fn build_model_statuses_without_worker(
    resource_base_dir: Option<&Path>,
    session_error: &str,
) -> Vec<ScannerOrtModelStatus> {
    SCANNER_ORT_MODEL_MANIFEST
        .iter()
        .map(|model| {
            let resolved_path =
                resource_base_dir.map(|base_dir| base_dir.join(model.relative_path));
            let path_error = match (resource_base_dir, resolved_path.as_deref()) {
                (Some(base_dir), Some(path)) => {
                    scanner_resource::validate_path_containment(path, base_dir).err()
                }
                _ => None,
            };
            let file_exists = resolved_path
                .as_ref()
                .map(|path| path.exists())
                .unwrap_or(false);
            ScannerOrtModelStatus {
                id: model.id.to_string(),
                role: model.role.to_string(),
                relative_path: model.relative_path.to_string(),
                resolved_path: resolved_path
                    .as_deref()
                    .map(scanner_resource::path_to_string),
                file_exists,
                session_ready: false,
                session_error: Some(path_error.unwrap_or_else(|| session_error.to_string())),
                inputs: Vec::new(),
                outputs: Vec::new(),
            }
        })
        .collect()
}

fn send_worker_probe(resource_base_dir: &Path) -> Result<ScannerOrtWorkerProbeResult, String> {
    let mut state = worker_state()
        .lock()
        .expect("scanner ORT worker mutex should not be poisoned");
    let response = send_worker_request_locked(
        &mut state,
        ScannerOrtWorkerRequestPayload::Probe(build_worker_resource_request(resource_base_dir)),
    )?;

    match response.payload {
        ScannerOrtWorkerResponsePayload::Ok {
            operation: ScannerOrtWorkerOperation::Probe,
            probe: Some(probe),
        } => Ok(probe),
        ScannerOrtWorkerResponsePayload::Ok { operation, .. } => Err(format!(
            "Scanner ORT worker returned unexpected operation {:?}.",
            operation
        )),
        ScannerOrtWorkerResponsePayload::Error { code, message } => {
            Err(format!("{code}: {message}"))
        }
        ScannerOrtWorkerResponsePayload::Event { event, message } => Err(format!(
            "Unexpected scanner ORT worker event {event}: {message}"
        )),
    }
}

fn build_worker_resource_request(resource_base_dir: &Path) -> ScannerOrtWorkerResourceRequest {
    ScannerOrtWorkerResourceRequest {
        resource_base_dir: scanner_resource::path_to_string(resource_base_dir),
        runtime_library_path: runtime_library_path_for_current_platform(resource_base_dir)
            .as_deref()
            .map(scanner_resource::path_to_string),
        models: SCANNER_ORT_MODEL_MANIFEST
            .iter()
            .map(|model| ScannerOrtWorkerModelRequest {
                id: model.id.to_string(),
                role: model.role.to_string(),
                relative_path: model.relative_path.to_string(),
                resolved_path: scanner_resource::path_to_string(
                    &resource_base_dir.join(model.relative_path),
                ),
            })
            .collect(),
    }
}

fn worker_state() -> &'static Mutex<OrtWorkerState> {
    static STATE: OnceLock<Mutex<OrtWorkerState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(OrtWorkerState::default()))
}

fn send_worker_request_locked(
    state: &mut OrtWorkerState,
    payload: ScannerOrtWorkerRequestPayload,
) -> Result<ScannerOrtWorkerResponse, String> {
    if state.asset_mutation_active {
        return Err(
            "Scanner ORT worker is blocked while ORT assets are being updated.".to_string(),
        );
    }

    if worker_has_exited(state) {
        state.child = None;
    }
    if state.child.is_none() {
        state.child = Some(spawn_worker()?);
    }

    let request = ScannerOrtWorkerRequest {
        id: next_request_id(state),
        payload,
    };

    let exchange_result = {
        let worker = state
            .child
            .as_mut()
            .expect("scanner ORT worker should be present after spawn");
        exchange_worker_request(worker, &request)
    };

    match exchange_result {
        Ok(response) if response.id == request.id => Ok(response),
        Ok(response) => {
            let error = format!(
                "Scanner ORT worker response id {} did not match request id {}.",
                response.id, request.id
            );
            terminate_failed_worker(state, &error);
            Err(error)
        }
        Err(error) => {
            terminate_failed_worker(state, &error);
            Err(error)
        }
    }
}

fn next_request_id(state: &mut OrtWorkerState) -> u64 {
    let id = state.next_request_id;
    state.next_request_id = state.next_request_id.saturating_add(1).max(1);
    id
}

fn exchange_worker_request(
    worker: &mut OrtWorkerChild,
    request: &ScannerOrtWorkerRequest,
) -> Result<ScannerOrtWorkerResponse, String> {
    write_framed_json(&mut worker.stdin, request).map_err(|error| {
        format!(
            "Failed to write request to scanner ORT worker {}: {error}",
            worker.launch_label
        )
    })?;

    read_framed_json::<_, ScannerOrtWorkerResponse>(&mut worker.stdout)
        .map_err(|error| {
            format!(
                "Failed to read response from scanner ORT worker {}: {error}",
                worker.launch_label
            )
        })?
        .ok_or_else(|| {
            format!(
                "Scanner ORT worker {} exited without a response.",
                worker.launch_label
            )
        })
}

fn terminate_failed_worker(state: &mut OrtWorkerState, error: &str) {
    state.last_protocol_error = Some(error.to_string());
    if let Some(mut worker) = state.child.take() {
        let _ = worker.child.kill();
        let _ = worker.child.wait();
    }
}

fn worker_has_exited(state: &mut OrtWorkerState) -> bool {
    let Some(worker) = state.child.as_mut() else {
        return false;
    };
    match worker.child.try_wait() {
        Ok(Some(status)) => {
            state.last_exit = Some(status.to_string());
            true
        }
        Ok(None) => false,
        Err(error) => {
            state.last_protocol_error = Some(format!("Failed to poll scanner ORT worker: {error}"));
            true
        }
    }
}

#[cfg(not(test))]
fn stop_worker_locked(state: &mut OrtWorkerState) -> Result<(), String> {
    let Some(mut worker) = state.child.take() else {
        return Ok(());
    };

    let request = ScannerOrtWorkerRequest {
        id: next_request_id(state),
        payload: ScannerOrtWorkerRequestPayload::Shutdown,
    };

    let exchange_result = exchange_worker_request(&mut worker, &request);
    match exchange_result {
        Ok(ScannerOrtWorkerResponse {
            id,
            payload:
                ScannerOrtWorkerResponsePayload::Ok {
                    operation: ScannerOrtWorkerOperation::Shutdown,
                    ..
                },
        }) if id == request.id => {}
        Ok(response) => {
            state.child = Some(worker);
            return Err(format!(
                "Scanner ORT worker returned unexpected shutdown response: {:?}.",
                response.payload
            ));
        }
        Err(error) => match worker.child.try_wait() {
            Ok(Some(status)) => {
                state.last_exit = Some(status.to_string());
                return Ok(());
            }
            _ => {
                state.child = Some(worker);
                state.last_protocol_error = Some(error.clone());
                return Err(error);
            }
        },
    }

    let deadline = Instant::now() + WORKER_SHUTDOWN_TIMEOUT;
    loop {
        match worker.child.try_wait() {
            Ok(Some(status)) => {
                state.last_exit = Some(status.to_string());
                return Ok(());
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(WORKER_POLL_INTERVAL);
            }
            Ok(None) => {
                state.child = Some(worker);
                return Err(format!(
                    "Scanner ORT worker did not exit within {} seconds.",
                    WORKER_SHUTDOWN_TIMEOUT.as_secs()
                ));
            }
            Err(error) => {
                state.child = Some(worker);
                return Err(format!(
                    "Failed to wait for scanner ORT worker exit: {error}"
                ));
            }
        }
    }
}

fn spawn_worker() -> Result<OrtWorkerChild, String> {
    let launch = resolve_worker_launch_command();
    let label = launch.label();
    let mut command = match &launch {
        OrtWorkerLaunchCommand::Direct(path) => Command::new(path),
        OrtWorkerLaunchCommand::CargoRun { manifest_path } => {
            let mut command = Command::new("cargo");
            command.args([
                "run",
                "--quiet",
                "--manifest-path",
                &scanner_resource::path_to_string(manifest_path),
                "--bin",
                "scanner-ort-worker",
                "--",
            ]);
            command
        }
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to start scanner ORT worker {label}: {error}"))?;
    let stdin = child.stdin.take().ok_or_else(|| {
        format!("Scanner ORT worker {label} did not expose a stdin protocol pipe.")
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        format!("Scanner ORT worker {label} did not expose a stdout protocol pipe.")
    })?;

    Ok(OrtWorkerChild {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        launch_label: label,
    })
}

fn resolve_worker_launch_command() -> OrtWorkerLaunchCommand {
    if let Ok(path) = std::env::var("SKID_SCANNER_ORT_WORKER") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return OrtWorkerLaunchCommand::Direct(PathBuf::from(trimmed));
        }
    }

    let worker_name = scanner_ort_worker_binary_name();
    if let Ok(current_exe) = std::env::current_exe() {
        let sibling = current_exe.with_file_name(worker_name);
        if sibling.is_file() {
            return OrtWorkerLaunchCommand::Direct(sibling);
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_binary = manifest_dir
        .join("target")
        .join(if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        })
        .join(worker_name);
    if target_binary.is_file() {
        return OrtWorkerLaunchCommand::Direct(target_binary);
    }

    OrtWorkerLaunchCommand::CargoRun {
        manifest_path: manifest_dir.join("Cargo.toml"),
    }
}

fn scanner_ort_worker_binary_name() -> &'static str {
    if cfg!(windows) {
        "scanner-ort-worker.exe"
    } else {
        "scanner-ort-worker"
    }
}

fn runtime_library_path_status_texts(
    runtime_status: &ScannerOrtWorkerRuntimeStatus,
) -> (Option<String>, Option<String>, Option<String>) {
    let selected_runtime_library_path = runtime_status.selected_runtime_library_path.clone();
    let loaded_runtime_library_path = runtime_status.loaded_runtime_library_path.clone();
    let runtime_library_path = loaded_runtime_library_path.clone();

    (
        selected_runtime_library_path,
        loaded_runtime_library_path,
        runtime_library_path,
    )
}

fn build_probe_message(
    runtime_status: &ScannerOrtWorkerRuntimeStatus,
    models: &[ScannerOrtModelStatus],
    model_load_ready: bool,
) -> String {
    if !runtime_status.ready {
        return runtime_status
            .runtime_error
            .clone()
            .unwrap_or_else(|| "Scanner ORT runtime is not ready.".to_string());
    }

    if runtime_status.runtime_path_mismatch {
        return runtime_status.runtime_error.clone().unwrap_or_else(|| {
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

#[cfg(test)]
#[allow(dead_code)]
fn runtime_path_mismatch_error(selected: Option<&Path>, loaded: Option<&Path>) -> Option<String> {
    let selected = selected?;
    let loaded = loaded?;
    if selected == loaded {
        return None;
    }

    Some(
        crate::scanner_ort_protocol::build_runtime_path_mismatch_error(
            &scanner_resource::path_to_string(selected),
            &scanner_resource::path_to_string(loaded),
        ),
    )
}

fn runtime_library_path_for_current_platform(resource_base_dir: &Path) -> Option<PathBuf> {
    match std::env::consts::OS {
        "windows" => Some(resource_base_dir.join(WINDOWS_ORT_RELATIVE_PATH)),
        "linux" => Some(resource_base_dir.join(LINUX_ORT_RELATIVE_PATH)),
        _ => None,
    }
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
            let metadata = resolved_path
                .as_ref()
                .and_then(|path| fs::metadata(path).ok());
            ScannerOrtResourceStatus {
                relative_path: (*relative_path).to_string(),
                resolved_path: resolved_path
                    .as_deref()
                    .map(scanner_resource::path_to_string),
                exists: metadata.is_some(),
                size_bytes: metadata
                    .as_ref()
                    .filter(|metadata| metadata.is_file())
                    .map(|metadata| metadata.len()),
                required: true,
            }
        })
        .collect()
}

fn build_current_resource_tree(current_dir: Option<&Path>) -> Vec<ScannerOrtResourceTreeEntry> {
    let Some(current_dir) = current_dir else {
        return Vec::new();
    };

    if !current_dir.is_dir() {
        return Vec::new();
    }

    let mut entries = Vec::new();
    collect_resource_tree_entries(current_dir, current_dir, 0, &mut entries);
    entries
}

struct ResourceTreeChild {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

fn collect_resource_tree_entries(
    root: &Path,
    dir: &Path,
    depth: usize,
    entries: &mut Vec<ScannerOrtResourceTreeEntry>,
) -> u64 {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return 0;
    };

    let mut children = read_dir
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            Some(ResourceTreeChild {
                path: entry.path(),
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: file_type.is_dir(),
            })
        })
        .collect::<Vec<_>>();

    children.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut total_size = 0;
    for child in children {
        let Some(relative_path) = resource_tree_relative_path(root, &child.path) else {
            continue;
        };

        if child.is_dir {
            let entry_index = entries.len();
            entries.push(ScannerOrtResourceTreeEntry {
                relative_path,
                name: child.name,
                depth,
                is_dir: true,
                size_bytes: Some(0),
            });

            let child_size = collect_resource_tree_entries(root, &child.path, depth + 1, entries);
            entries[entry_index].size_bytes = Some(child_size);
            total_size += child_size;
            continue;
        }

        let size_bytes = fs::symlink_metadata(&child.path)
            .ok()
            .map(|metadata| metadata.len());
        total_size += size_bytes.unwrap_or(0);
        entries.push(ScannerOrtResourceTreeEntry {
            relative_path,
            name: child.name,
            depth,
            is_dir: false,
            size_bytes,
        });
    }

    total_size
}

fn resource_tree_relative_path(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root).ok().map(|relative_path| {
        relative_path
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    })
}
