use std::{
    collections::HashSet,
    fs::{self, File},
    future::Future,
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{command, ipc::Channel, AppHandle, Manager};

use crate::{scanner_platform, scanner_resource};

const EXPECTED_SCANNER_ASSET_TAG: &str = "v0.1.0";
const SCANNER_ASSETS_REPO_OWNER: &str = "Nanboom233";
const SCANNER_ASSETS_REPO_NAME: &str = "skid-homework-assets";
const MANIFEST_FILE_NAME: &str = "manifest.json";
const ASSETS_DIR_NAME: &str = "assets";
const CURRENT_DIR_NAME: &str = "current";
const STAGING_DIR_NAME: &str = "staging";
const BACKUP_DIR_NAME: &str = "current.previous";
const DOWNLOAD_CHUNK_SIZE: usize = 64 * 1024;
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(60);
const ERROR_OPERATION_CANCELLED: &str = "assets.operation.cancelled";

const COMMON_REQUIRED_ASSET_PATHS: &[&str] = &[
    "models/docaligner-fastvit_sa24.onnx",
    "models/uvdoc-best-model.onnx",
    "models/LICENSE-docaligner",
    "models/LICENSE-uvdoc",
    "models/README.md",
];

const WINDOWS_REQUIRED_ASSET_PATHS: &[&str] = &[
    "onnxruntime/windows/onnxruntime.dll",
    "onnxruntime/windows/onnxruntime_providers_shared.dll",
    "onnxruntime/windows/DirectML.dll",
    "onnxruntime/windows/README.md",
];

const LINUX_REQUIRED_ASSET_PATHS: &[&str] = &[
    "onnxruntime/linux/libonnxruntime.so",
    "onnxruntime/linux/libonnxruntime.so.1",
    "onnxruntime/linux/libonnxruntime.so.1.24.4",
    "onnxruntime/linux/libonnxruntime_providers_shared.so",
    "onnxruntime/linux/libonnxruntime_providers_cuda.so",
    "onnxruntime/linux/libonnxruntime_providers_tensorrt.so",
    "onnxruntime/linux/README.md",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAssetsError {
    pub code: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl ScannerAssetsError {
    pub(crate) fn new(
        code: impl Into<String>,
        retryable: bool,
        details: impl Into<Option<String>>,
    ) -> Self {
        Self {
            code: code.into(),
            retryable,
            details: details.into(),
        }
    }

    pub(crate) fn with_details(
        code: impl Into<String>,
        retryable: bool,
        details: impl Into<String>,
    ) -> Self {
        Self::new(code, retryable, Some(details.into()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationKind {
    Downloading,
    Importing,
    Clearing,
}

impl OperationKind {
    fn state(self) -> &'static str {
        match self {
            OperationKind::Downloading => "downloading",
            OperationKind::Importing => "importing",
            OperationKind::Clearing => "clearing",
        }
    }
}

#[derive(Debug)]
struct OperationGuard {
    kind: OperationKind,
    operation_id: Option<String>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut state = operation_state()
            .lock()
            .expect("scanner asset operation mutex should not be poisoned");
        if state
            .as_ref()
            .map(|active| active.kind == self.kind && active.operation_id == self.operation_id)
            .unwrap_or(false)
        {
            *state = None;
        }
    }
}

#[derive(Debug)]
struct ActiveOperationState {
    kind: OperationKind,
    operation_id: Option<String>,
    cancel_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAssetsProgress {
    pub phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ScannerAssetsError>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAssetsDownloadRequest {
    pub operation_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAssetsCancelRequest {
    pub operation_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAssetsImportRequest {
    pub archive_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAssetsManifestSummary {
    pub schema_version: u32,
    pub asset_version: String,
    pub platform_target: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAssetsInstallResult {
    pub installed: bool,
    pub asset_version: String,
    pub platform_target: String,
    pub current_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAssetsStatus {
    pub state: String,
    pub platform_target: String,
    pub expected_asset_tag: String,
    pub default_asset_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<ScannerAssetsManifestSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<ScannerAssetsError>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAssetsUpdateCheck {
    pub platform_target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_asset_version: Option<String>,
    pub target_asset_tag: String,
    pub update_available: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScannerAssetManifest {
    schema_version: u32,
    asset_version: String,
    platform_target: String,
    files: Vec<ScannerAssetManifestFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScannerAssetManifestFile {
    path: String,
    size: u64,
    sha256: String,
    source_url: String,
}

#[derive(Debug, Clone)]
pub struct ScannerAssetPaths {
    pub assets_dir: PathBuf,
    pub current_dir: PathBuf,
    staging_dir: PathBuf,
    backup_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct ScannerAssetsDownloadTarget {
    asset_tag: String,
    asset_url: String,
    package_file_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubReleaseAsset {
    name: String,
}

#[command]
pub fn scanner_assets_status(app: AppHandle) -> Result<ScannerAssetsStatus, ScannerAssetsError> {
    let paths = resolve_asset_paths(&app)?;
    let operation = current_operation();
    let current_dir = Some(scanner_resource::path_to_string(&paths.current_dir));
    let mut manifest = None;
    let mut status_error = None;
    let mut state = if paths.current_dir.exists() {
        match inspect_current_install(&paths.current_dir) {
            Ok(summary) => {
                manifest = Some(summary);
                "ready".to_string()
            }
            Err(error) => {
                status_error = Some(error);
                "invalid".to_string()
            }
        }
    } else {
        "missing".to_string()
    };

    if let Some(operation) = operation {
        state = operation.state().to_string();
    }

    Ok(ScannerAssetsStatus {
        state,
        platform_target: scanner_platform::platform_target().to_string(),
        expected_asset_tag: EXPECTED_SCANNER_ASSET_TAG.to_string(),
        default_asset_url: default_asset_url(),
        current_dir,
        manifest,
        last_error: status_error.or_else(last_error),
    })
}

#[command]
pub async fn scanner_assets_download(
    app: AppHandle,
    request: ScannerAssetsDownloadRequest,
    progress_channel: Channel<ScannerAssetsProgress>,
) -> Result<ScannerAssetsInstallResult, ScannerAssetsError> {
    scanner_assets_download_with_target(app, request, progress_channel, || async {
        Ok(expected_download_target())
    })
    .await
}

#[command]
pub fn scanner_assets_cancel(
    _app: AppHandle,
    request: ScannerAssetsCancelRequest,
) -> Result<(), ScannerAssetsError> {
    request_operation_cancel(&request.operation_id);
    Ok(())
}

#[command]
pub async fn scanner_assets_check_update(
    app: AppHandle,
) -> Result<ScannerAssetsUpdateCheck, ScannerAssetsError> {
    let paths = resolve_asset_paths(&app)?;
    let current_asset_version = if paths.current_dir.exists() {
        inspect_current_install(&paths.current_dir)
            .map(|summary| summary.asset_version)
            .ok()
    } else {
        None
    };
    let target = latest_official_release_target().await?;

    Ok(ScannerAssetsUpdateCheck {
        platform_target: scanner_platform::platform_target().to_string(),
        update_available: is_update_available(current_asset_version.as_deref(), &target.asset_tag),
        current_asset_version,
        target_asset_tag: target.asset_tag,
    })
}

#[command]
pub async fn scanner_assets_download_update(
    app: AppHandle,
    request: ScannerAssetsDownloadRequest,
    progress_channel: Channel<ScannerAssetsProgress>,
) -> Result<ScannerAssetsInstallResult, ScannerAssetsError> {
    scanner_assets_download_with_target(app, request, progress_channel, || async {
        latest_official_release_target().await
    })
    .await
}

async fn scanner_assets_download_with_target<F, Fut>(
    app: AppHandle,
    request: ScannerAssetsDownloadRequest,
    progress_channel: Channel<ScannerAssetsProgress>,
    resolve_target: F,
) -> Result<ScannerAssetsInstallResult, ScannerAssetsError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ScannerAssetsDownloadTarget, ScannerAssetsError>>,
{
    let (guard, cancel_requested) = match try_acquire_download_operation(request.operation_id) {
        Ok(operation) => operation,
        Err(error) => {
            send_failed_progress(&progress_channel, error.clone());
            return Err(error);
        }
    };
    let paths = match resolve_asset_paths(&app) {
        Ok(paths) => paths,
        Err(error) => {
            send_failed_progress(&progress_channel, error.clone());
            return Err(error);
        }
    };

    let target = match resolve_target().await {
        Ok(target) => target,
        Err(error) => {
            set_last_error(error.clone());
            send_failed_progress(&progress_channel, error.clone());
            return Err(error);
        }
    };

    let _guard = guard;
    finish_progress_operation(
        &progress_channel,
        download_and_install(paths, target, &progress_channel, cancel_requested).await,
    )
}

#[command]
pub async fn scanner_assets_import(
    app: AppHandle,
    request: ScannerAssetsImportRequest,
    progress_channel: Channel<ScannerAssetsProgress>,
) -> Result<ScannerAssetsInstallResult, ScannerAssetsError> {
    let guard = match try_acquire_operation(OperationKind::Importing) {
        Ok(guard) => guard,
        Err(error) => {
            send_failed_progress(&progress_channel, error.clone());
            return Err(error);
        }
    };
    let paths = match resolve_asset_paths(&app) {
        Ok(paths) => paths,
        Err(error) => {
            send_failed_progress(&progress_channel, error.clone());
            return Err(error);
        }
    };
    let progress_channel_for_task = progress_channel.clone();

    match tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        finish_progress_operation(
            &progress_channel_for_task,
            import_archive(paths, request, &progress_channel_for_task),
        )
    })
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let error = ScannerAssetsError::with_details(
                "assets.operation.taskFailed",
                true,
                format!("Scanner asset import task failed: {error}"),
            );
            set_last_error(error.clone());
            send_failed_progress(&progress_channel, error.clone());
            Err(error)
        }
    }
}

#[command]
pub fn scanner_assets_clear(app: AppHandle) -> Result<(), ScannerAssetsError> {
    let guard = try_acquire_operation(OperationKind::Clearing)?;
    let paths = resolve_asset_paths(&app)?;
    let _guard = guard;

    match clear_installed_assets(&paths) {
        Ok(()) => {
            clear_last_error();
            Ok(())
        }
        Err(error) => {
            set_last_error(error.clone());
            Err(error)
        }
    }
}

pub fn installed_assets_current_dir_from_app(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join(ASSETS_DIR_NAME).join(CURRENT_DIR_NAME))
}

pub fn resolve_asset_paths(app: &AppHandle) -> Result<ScannerAssetPaths, ScannerAssetsError> {
    let app_data_dir = app.path().app_data_dir().map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.status.resolveDataDirFailed",
            true,
            format!("Failed to resolve app data directory: {error}"),
        )
    })?;
    Ok(asset_paths_from_app_data_dir(app_data_dir))
}

fn asset_paths_from_app_data_dir(app_data_dir: PathBuf) -> ScannerAssetPaths {
    let assets_dir = app_data_dir.join(ASSETS_DIR_NAME);
    ScannerAssetPaths {
        current_dir: assets_dir.join(CURRENT_DIR_NAME),
        staging_dir: assets_dir.join(STAGING_DIR_NAME),
        backup_dir: assets_dir.join(BACKUP_DIR_NAME),
        assets_dir,
    }
}

fn platform_archive_extension() -> &'static str {
    match std::env::consts::OS {
        "windows" => "zip",
        "linux" => "tar.gz",
        _ => "unsupported",
    }
}

fn platform_package_file_name() -> String {
    platform_package_file_name_for_tag(EXPECTED_SCANNER_ASSET_TAG)
}

fn platform_package_file_name_for_tag(tag: &str) -> String {
    format!(
        "{}-{}.{}",
        scanner_platform::platform_target(),
        tag,
        platform_archive_extension()
    )
}

fn default_asset_url() -> String {
    official_release_asset_url(EXPECTED_SCANNER_ASSET_TAG, &platform_package_file_name())
}

fn official_release_asset_url(tag: &str, package_file_name: &str) -> String {
    format!(
        "https://github.com/{}/{}/releases/download/{}/{}",
        SCANNER_ASSETS_REPO_OWNER, SCANNER_ASSETS_REPO_NAME, tag, package_file_name
    )
}

fn expected_download_target() -> ScannerAssetsDownloadTarget {
    let package_file_name = platform_package_file_name();
    ScannerAssetsDownloadTarget {
        asset_tag: EXPECTED_SCANNER_ASSET_TAG.to_string(),
        asset_url: official_release_asset_url(EXPECTED_SCANNER_ASSET_TAG, &package_file_name),
        package_file_name,
    }
}

async fn latest_official_release_target() -> Result<ScannerAssetsDownloadTarget, ScannerAssetsError>
{
    let releases = fetch_official_releases().await?;
    select_latest_release_target(&releases).ok_or_else(|| {
        ScannerAssetsError::with_details(
            "assets.update.checkFailed",
            true,
            format!(
                "No scanner asset release with {} and its checksum was found.",
                platform_package_file_name_for_tag("<tag>")
            ),
        )
    })
}

async fn fetch_official_releases() -> Result<Vec<GitHubRelease>, ScannerAssetsError> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases?per_page=100",
        SCANNER_ASSETS_REPO_OWNER, SCANNER_ASSETS_REPO_NAME
    );
    let client = reqwest::Client::builder()
        .user_agent("skid-homework-assets/0.1")
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .read_timeout(DOWNLOAD_READ_TIMEOUT)
        .build()
        .map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.update.checkFailed",
                true,
                format!("Failed to build GitHub release client: {error}"),
            )
        })?;
    let response = client.get(&url).send().await.map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.update.checkFailed",
            true,
            format!("Failed to check scanner asset releases: {error}"),
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(ScannerAssetsError::with_details(
            "assets.update.checkFailed",
            true,
            format!("Scanner asset release check returned HTTP status {status}."),
        ));
    }
    let body = response.text().await.map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.update.checkFailed",
            true,
            format!("Failed to read scanner asset release response: {error}"),
        )
    })?;
    serde_json::from_str(&body).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.update.checkFailed",
            true,
            format!("Failed to parse scanner asset release response: {error}"),
        )
    })
}

fn select_latest_release_target(releases: &[GitHubRelease]) -> Option<ScannerAssetsDownloadTarget> {
    releases.iter().find_map(|release| {
        if release.draft {
            return None;
        }
        // GitHub's releases API returns prerelease entries; keep them
        // eligible when their stable tag filename and checksum contract
        // match this ORT target.
        let _prerelease_is_eligible = release.prerelease;
        parse_stable_semver_tag(&release.tag_name)?;
        let package_file_name = platform_package_file_name_for_tag(&release.tag_name);
        let checksum_file_name = format!("{package_file_name}.sha256");
        if !release_has_asset(release, &package_file_name)
            || !release_has_asset(release, &checksum_file_name)
        {
            return None;
        }
        Some(ScannerAssetsDownloadTarget {
            asset_tag: release.tag_name.clone(),
            asset_url: official_release_asset_url(&release.tag_name, &package_file_name),
            package_file_name,
        })
    })
}

fn release_has_asset(release: &GitHubRelease, asset_name: &str) -> bool {
    release.assets.iter().any(|asset| asset.name == asset_name)
}

fn parse_stable_semver_tag(tag: &str) -> Option<(u64, u64, u64)> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    if version.contains('-') || version.contains('+') {
        return None;
    }
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn is_update_available(current_asset_version: Option<&str>, target_tag: &str) -> bool {
    let Some(current_asset_version) = current_asset_version else {
        return true;
    };
    match (
        parse_stable_semver_tag(current_asset_version),
        parse_stable_semver_tag(target_tag),
    ) {
        (Some(current), Some(target)) => current < target,
        _ => current_asset_version != target_tag,
    }
}

fn operation_state() -> &'static Mutex<Option<ActiveOperationState>> {
    static STATE: OnceLock<Mutex<Option<ActiveOperationState>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

fn current_operation() -> Option<OperationKind> {
    operation_state()
        .lock()
        .expect("scanner asset operation mutex should not be poisoned")
        .as_ref()
        .map(|active| active.kind)
}

fn try_acquire_operation(kind: OperationKind) -> Result<OperationGuard, ScannerAssetsError> {
    let mut state = operation_state()
        .lock()
        .expect("scanner asset operation mutex should not be poisoned");
    if state.is_some() {
        return Err(ScannerAssetsError::with_details(
            "assets.operation.inProgress",
            true,
            "A scanner asset download or import operation is already in progress.",
        ));
    }

    *state = Some(ActiveOperationState {
        kind,
        operation_id: None,
        cancel_requested: Arc::new(AtomicBool::new(false)),
    });
    Ok(OperationGuard {
        kind,
        operation_id: None,
    })
}

fn try_acquire_download_operation(
    operation_id: String,
) -> Result<(OperationGuard, Arc<AtomicBool>), ScannerAssetsError> {
    let mut state = operation_state()
        .lock()
        .expect("scanner asset operation mutex should not be poisoned");
    if state.is_some() {
        return Err(ScannerAssetsError::with_details(
            "assets.operation.inProgress",
            true,
            "A scanner asset download or import operation is already in progress.",
        ));
    }

    let cancel_requested = Arc::new(AtomicBool::new(false));
    *state = Some(ActiveOperationState {
        kind: OperationKind::Downloading,
        operation_id: Some(operation_id.clone()),
        cancel_requested: Arc::clone(&cancel_requested),
    });
    Ok((
        OperationGuard {
            kind: OperationKind::Downloading,
            operation_id: Some(operation_id),
        },
        cancel_requested,
    ))
}

fn request_operation_cancel(operation_id: &str) {
    let state = operation_state()
        .lock()
        .expect("scanner asset operation mutex should not be poisoned");
    if let Some(active) = state.as_ref() {
        if active.kind == OperationKind::Downloading
            && active.operation_id.as_deref() == Some(operation_id)
        {
            active.cancel_requested.store(true, Ordering::SeqCst);
        }
    }
}

fn cancelled_error() -> ScannerAssetsError {
    ScannerAssetsError::with_details(
        ERROR_OPERATION_CANCELLED,
        false,
        "Scanner asset operation was cancelled.",
    )
}

fn check_cancelled(cancel_requested: &AtomicBool) -> Result<(), ScannerAssetsError> {
    if cancel_requested.load(Ordering::SeqCst) {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

fn is_cancelled_error(error: &ScannerAssetsError) -> bool {
    error.code == ERROR_OPERATION_CANCELLED
}

fn last_error_state() -> &'static Mutex<Option<ScannerAssetsError>> {
    static STATE: OnceLock<Mutex<Option<ScannerAssetsError>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

fn set_last_error(error: ScannerAssetsError) {
    *last_error_state()
        .lock()
        .expect("scanner asset last-error mutex should not be poisoned") = Some(error);
}

fn clear_last_error() {
    *last_error_state()
        .lock()
        .expect("scanner asset last-error mutex should not be poisoned") = None;
}

fn last_error() -> Option<ScannerAssetsError> {
    last_error_state()
        .lock()
        .expect("scanner asset last-error mutex should not be poisoned")
        .clone()
}

fn send_progress(channel: &Channel<ScannerAssetsProgress>, progress: ScannerAssetsProgress) {
    let _ = channel.send(progress);
}

fn send_phase_progress(channel: &Channel<ScannerAssetsProgress>, phase: &'static str) {
    send_progress(
        channel,
        ScannerAssetsProgress {
            phase,
            bytes_done: None,
            bytes_total: None,
            error: None,
        },
    );
}

fn send_failed_progress(channel: &Channel<ScannerAssetsProgress>, error: ScannerAssetsError) {
    send_progress(
        channel,
        ScannerAssetsProgress {
            phase: "failed",
            bytes_done: None,
            bytes_total: None,
            error: Some(error),
        },
    );
}

fn finish_progress_operation<T>(
    channel: &Channel<ScannerAssetsProgress>,
    result: Result<T, ScannerAssetsError>,
) -> Result<T, ScannerAssetsError> {
    match result {
        Ok(value) => {
            clear_last_error();
            send_phase_progress(channel, "completed");
            Ok(value)
        }
        Err(error) => {
            if is_cancelled_error(&error) {
                clear_last_error();
            } else {
                set_last_error(error.clone());
            }
            send_failed_progress(channel, error.clone());
            Err(error)
        }
    }
}

async fn download_and_install(
    paths: ScannerAssetPaths,
    target: ScannerAssetsDownloadTarget,
    channel: &Channel<ScannerAssetsProgress>,
    cancel_requested: Arc<AtomicBool>,
) -> Result<ScannerAssetsInstallResult, ScannerAssetsError> {
    fs::create_dir_all(&paths.assets_dir).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.download.fetch.writeFailed",
            true,
            format!(
                "Failed to create asset directory {}: {error}",
                scanner_resource::path_to_string(&paths.assets_dir)
            ),
        )
    })?;

    let archive_path = paths.assets_dir.join(format!(
        ".download-{}",
        target.package_file_name.replace('/', "-")
    ));

    let result = match download_archive_to_path(
        &target.asset_url,
        &archive_path,
        channel,
        &cancel_requested,
    )
    .await
    {
        Ok(()) => match check_cancelled(&cancel_requested) {
            Ok(()) => {
                let paths_for_install = paths.clone();
                let archive_path_for_install = archive_path.clone();
                let channel_for_install = channel.clone();
                let cancel_requested_for_install = Arc::clone(&cancel_requested);
                let target_asset_tag = target.asset_tag.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    install_archive(
                        &paths_for_install,
                        &archive_path_for_install,
                        &channel_for_install,
                        Some(cancel_requested_for_install.as_ref()),
                        ManifestVersionPolicy::RequireTag(target_asset_tag),
                    )
                })
                .await
                .map_err(|error| {
                    ScannerAssetsError::with_details(
                        "assets.operation.taskFailed",
                        true,
                        format!("Scanner asset install task failed: {error}"),
                    )
                })?
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };

    cleanup_failed_download(&paths, &archive_path, result.is_err());
    result
}

async fn download_archive_to_path(
    url: &str,
    archive_path: &Path,
    channel: &Channel<ScannerAssetsProgress>,
    cancel_requested: &AtomicBool,
) -> Result<(), ScannerAssetsError> {
    check_cancelled(cancel_requested)?;
    send_phase_progress(channel, "fetching");
    let client = reqwest::Client::builder()
        .user_agent("skid-homework-assets/0.1")
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .read_timeout(DOWNLOAD_READ_TIMEOUT)
        .build()
        .map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.download.fetch.networkFailed",
                true,
                format!("Failed to build HTTP client: {error}"),
            )
        })?;
    let mut response = client.get(url).send().await.map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.download.fetch.networkFailed",
            true,
            format!("Failed to download scanner asset package from {url}: {error}"),
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(ScannerAssetsError::with_details(
            "assets.download.fetch.httpStatus",
            status.is_server_error() || status.as_u16() == 408 || status.as_u16() == 429,
            format!("Scanner asset package download returned HTTP status {status}."),
        ));
    }

    check_cancelled(cancel_requested)?;
    let bytes_total = response.content_length();
    let file = File::create(archive_path).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.download.fetch.writeFailed",
            true,
            format!(
                "Failed to create download file {}: {error}",
                scanner_resource::path_to_string(archive_path)
            ),
        )
    })?;
    let mut writer = BufWriter::new(file);
    let mut bytes_done = 0_u64;

    loop {
        check_cancelled(cancel_requested)?;
        let Some(chunk) = response.chunk().await.map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.download.fetch.networkFailed",
                true,
                format!("Failed while reading scanner asset package response: {error}"),
            )
        })?
        else {
            break;
        };
        check_cancelled(cancel_requested)?;
        if chunk.is_empty() {
            continue;
        }
        writer.write_all(&chunk).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.download.fetch.writeFailed",
                true,
                format!("Failed while writing scanner asset package: {error}"),
            )
        })?;
        bytes_done += chunk.len() as u64;
        send_progress(
            channel,
            ScannerAssetsProgress {
                phase: "fetching",
                bytes_done: Some(bytes_done),
                bytes_total,
                error: None,
            },
        );
    }
    check_cancelled(cancel_requested)?;
    writer.flush().map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.download.fetch.writeFailed",
            true,
            format!("Failed to flush scanner asset package: {error}"),
        )
    })?;

    Ok(())
}

fn cleanup_failed_download(paths: &ScannerAssetPaths, archive_path: &Path, failed: bool) {
    let _ = fs::remove_file(archive_path);
    if failed {
        let _ = fs::remove_dir_all(&paths.staging_dir);
    }
}

fn clear_installed_assets(paths: &ScannerAssetPaths) -> Result<(), ScannerAssetsError> {
    let _ort_worker_guard = begin_ort_asset_mutation()?;
    remove_path_if_exists(&paths.current_dir).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.clear.removeFailed",
            true,
            format!(
                "Failed to remove installed scanner assets at {}: {error}",
                scanner_resource::path_to_string(&paths.current_dir)
            ),
        )
    })?;

    cleanup_transient_asset_paths(paths);
    clear_last_error();
    Ok(())
}

fn cleanup_transient_asset_paths(paths: &ScannerAssetPaths) {
    let _ = remove_path_if_exists(&paths.staging_dir);
    let _ = remove_path_if_exists(&paths.backup_dir);

    let Ok(entries) = fs::read_dir(&paths.assets_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name.starts_with(".download-") || name.starts_with(&format!("{BACKUP_DIR_NAME}.")) {
            let _ = remove_path_if_exists(&entry.path());
        }
    }
}

fn remove_path_if_exists(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)
        }
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn import_archive(
    paths: ScannerAssetPaths,
    request: ScannerAssetsImportRequest,
    channel: &Channel<ScannerAssetsProgress>,
) -> Result<ScannerAssetsInstallResult, ScannerAssetsError> {
    let result = (|| {
        let archive_path = PathBuf::from(request.archive_path.trim());
        if archive_path.as_os_str().is_empty() {
            return Err(ScannerAssetsError::with_details(
                "assets.import.archive.openFailed",
                false,
                "Archive path is required.",
            ));
        }
        validate_archive_format(&archive_path)?;
        if !archive_path.is_file() {
            return Err(ScannerAssetsError::with_details(
                "assets.import.archive.openFailed",
                false,
                format!(
                    "Scanner asset archive is not a file: {}",
                    scanner_resource::path_to_string(&archive_path)
                ),
            ));
        }

        install_archive(
            &paths,
            &archive_path,
            channel,
            None,
            ManifestVersionPolicy::AllowAny,
        )
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&paths.staging_dir);
    }
    result
}

fn validate_archive_format(archive_path: &Path) -> Result<(), ScannerAssetsError> {
    let path_text = archive_path.to_string_lossy().to_ascii_lowercase();
    let valid = match std::env::consts::OS {
        "windows" => path_text.ends_with(".zip"),
        "linux" => path_text.ends_with(".tar.gz"),
        _ => false,
    };

    if valid {
        Ok(())
    } else {
        Err(ScannerAssetsError::with_details(
            "assets.import.archive.formatMismatch",
            false,
            format!(
                "Expected a {} scanner asset package for platform target {}.",
                platform_archive_extension(),
                scanner_platform::platform_target()
            ),
        ))
    }
}

fn install_archive(
    paths: &ScannerAssetPaths,
    archive_path: &Path,
    channel: &Channel<ScannerAssetsProgress>,
    cancel_requested: Option<&AtomicBool>,
    version_policy: ManifestVersionPolicy,
) -> Result<ScannerAssetsInstallResult, ScannerAssetsError> {
    prepare_staging(paths)?;

    let result = (|| {
        if let Some(cancel_requested) = cancel_requested {
            check_cancelled(cancel_requested)?;
        }
        send_phase_progress(channel, "unpacking");
        extract_archive_for_current_platform(archive_path, &paths.staging_dir)?;

        if let Some(cancel_requested) = cancel_requested {
            check_cancelled(cancel_requested)?;
        }
        send_phase_progress(channel, "verifying");
        let manifest = load_and_verify_install_root(&paths.staging_dir, version_policy)?;
        let summary = manifest_summary(&manifest);

        if let Some(cancel_requested) = cancel_requested {
            check_cancelled(cancel_requested)?;
        }
        send_phase_progress(channel, "activating");
        let _ort_worker_guard = begin_ort_asset_mutation()?;
        activate_staging(paths)?;

        Ok(ScannerAssetsInstallResult {
            installed: true,
            asset_version: summary.asset_version,
            platform_target: summary.platform_target,
            current_dir: scanner_resource::path_to_string(&paths.current_dir),
        })
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&paths.staging_dir);
    }

    result
}

fn prepare_staging(paths: &ScannerAssetPaths) -> Result<(), ScannerAssetsError> {
    fs::create_dir_all(&paths.assets_dir).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.install.activate.replaceFailed",
            true,
            format!(
                "Failed to create scanner assets directory {}: {error}",
                scanner_resource::path_to_string(&paths.assets_dir)
            ),
        )
    })?;

    if paths.staging_dir.exists() {
        fs::remove_dir_all(&paths.staging_dir).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.install.activate.replaceFailed",
                true,
                format!(
                    "Failed to clear staging directory {}: {error}",
                    scanner_resource::path_to_string(&paths.staging_dir)
                ),
            )
        })?;
    }
    fs::create_dir_all(&paths.staging_dir).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.install.activate.replaceFailed",
            true,
            format!(
                "Failed to create staging directory {}: {error}",
                scanner_resource::path_to_string(&paths.staging_dir)
            ),
        )
    })
}

fn extract_archive_for_current_platform(
    archive_path: &Path,
    staging_dir: &Path,
) -> Result<(), ScannerAssetsError> {
    match std::env::consts::OS {
        "windows" => extract_zip_archive(archive_path, staging_dir),
        "linux" => extract_tar_gz_archive(archive_path, staging_dir),
        _ => Err(ScannerAssetsError::with_details(
            "assets.import.archive.formatMismatch",
            false,
            "Scanner assets are only supported on Windows and Linux desktop targets.",
        )),
    }
}

fn extract_zip_archive(archive_path: &Path, staging_dir: &Path) -> Result<(), ScannerAssetsError> {
    let file = File::open(archive_path).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.import.archive.openFailed",
            false,
            format!(
                "Failed to open zip archive {}: {error}",
                scanner_resource::path_to_string(archive_path)
            ),
        )
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.import.archive.openFailed",
            false,
            format!("Failed to read zip archive: {error}"),
        )
    })?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.import.archive.openFailed",
                false,
                format!("Failed to read zip archive entry {index}: {error}"),
            )
        })?;

        if entry.is_symlink() {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.unsafePath",
                false,
                format!(
                    "Zip archive contains unsupported symlink entry: {}",
                    entry.name()
                ),
            ));
        }
        if entry.name().contains('\\') {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.unsafePath",
                false,
                format!(
                    "Zip archive contains unsafe path separator: {}",
                    entry.name()
                ),
            ));
        }

        let Some(relative_path) = entry.enclosed_name() else {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.unsafePath",
                false,
                format!("Zip archive contains unsafe path: {}", entry.name()),
            ));
        };
        let relative_text = relative_path.to_string_lossy();
        let safe_relative_path = validate_manifest_relative_path(&relative_text)?;
        let output_path = staging_dir.join(safe_relative_path);

        if entry.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| {
                ScannerAssetsError::with_details(
                    "assets.import.archive.openFailed",
                    false,
                    format!(
                        "Failed to create directory {}: {error}",
                        scanner_resource::path_to_string(&output_path)
                    ),
                )
            })?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                ScannerAssetsError::with_details(
                    "assets.import.archive.openFailed",
                    false,
                    format!(
                        "Failed to create directory {}: {error}",
                        scanner_resource::path_to_string(parent)
                    ),
                )
            })?;
        }

        let mut output = File::create(&output_path).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.import.archive.openFailed",
                false,
                format!(
                    "Failed to create extracted file {}: {error}",
                    scanner_resource::path_to_string(&output_path)
                ),
            )
        })?;
        io::copy(&mut entry, &mut output).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.import.archive.openFailed",
                false,
                format!(
                    "Failed to extract file {}: {error}",
                    scanner_resource::path_to_string(&output_path)
                ),
            )
        })?;
    }

    Ok(())
}

fn extract_tar_gz_archive(
    archive_path: &Path,
    staging_dir: &Path,
) -> Result<(), ScannerAssetsError> {
    let file = File::open(archive_path).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.import.archive.openFailed",
            false,
            format!(
                "Failed to open tar.gz archive {}: {error}",
                scanner_resource::path_to_string(archive_path)
            ),
        )
    })?;
    let decoder = GzDecoder::new(BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.import.archive.openFailed",
            false,
            format!("Failed to read tar.gz archive entries: {error}"),
        )
    })?;

    for entry in entries {
        let mut entry = entry.map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.import.archive.openFailed",
                false,
                format!("Failed to read tar.gz archive entry: {error}"),
            )
        })?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.unsafePath",
                false,
                "tar.gz archive contains unsupported link entry.",
            ));
        }
        if entry.path_bytes().iter().any(|byte| *byte == b'\\') {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.unsafePath",
                false,
                "tar.gz archive contains unsafe path separator.",
            ));
        }

        let relative_path = entry.path().map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.install.verify.unsafePath",
                false,
                format!("tar.gz archive contains invalid path: {error}"),
            )
        })?;
        let Some(relative_text) = relative_path.to_str() else {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.unsafePath",
                false,
                "tar.gz archive contains non-UTF-8 path.",
            ));
        };
        let safe_relative_path = validate_manifest_relative_path(relative_text)?;
        let output_path = staging_dir.join(safe_relative_path);

        if entry_type.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| {
                ScannerAssetsError::with_details(
                    "assets.import.archive.openFailed",
                    false,
                    format!(
                        "Failed to create directory {}: {error}",
                        scanner_resource::path_to_string(&output_path)
                    ),
                )
            })?;
            continue;
        }

        if !entry_type.is_file() {
            continue;
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                ScannerAssetsError::with_details(
                    "assets.import.archive.openFailed",
                    false,
                    format!(
                        "Failed to create directory {}: {error}",
                        scanner_resource::path_to_string(parent)
                    ),
                )
            })?;
        }

        let mut output = File::create(&output_path).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.import.archive.openFailed",
                false,
                format!(
                    "Failed to create extracted file {}: {error}",
                    scanner_resource::path_to_string(&output_path)
                ),
            )
        })?;
        io::copy(&mut entry, &mut output).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.import.archive.openFailed",
                false,
                format!(
                    "Failed to extract file {}: {error}",
                    scanner_resource::path_to_string(&output_path)
                ),
            )
        })?;
    }

    Ok(())
}

#[derive(Debug, Clone)]
enum ManifestVersionPolicy {
    RequireTag(String),
    AllowAny,
}

fn load_and_verify_install_root(
    root: &Path,
    version_policy: ManifestVersionPolicy,
) -> Result<ScannerAssetManifest, ScannerAssetsError> {
    let manifest_path = root.join(MANIFEST_FILE_NAME);
    if !manifest_path.is_file() {
        return Err(ScannerAssetsError::with_details(
            "assets.install.manifest.missing",
            false,
            format!(
                "Scanner asset manifest is missing at {}.",
                scanner_resource::path_to_string(&manifest_path)
            ),
        ));
    }

    let manifest = parse_manifest_file(&manifest_path)?;
    verify_manifest_metadata(&manifest, version_policy)?;
    verify_declared_files(root, &manifest)?;
    verify_required_file_declarations(&manifest)?;
    Ok(manifest)
}

fn inspect_current_install(
    root: &Path,
) -> Result<ScannerAssetsManifestSummary, ScannerAssetsError> {
    let manifest_path = root.join(MANIFEST_FILE_NAME);
    if !manifest_path.is_file() {
        return Err(ScannerAssetsError::with_details(
            "assets.install.manifest.missing",
            false,
            format!(
                "Scanner asset manifest is missing at {}.",
                scanner_resource::path_to_string(&manifest_path)
            ),
        ));
    }

    let manifest = parse_manifest_file(&manifest_path)?;
    verify_manifest_metadata(&manifest, ManifestVersionPolicy::AllowAny)?;
    verify_declared_files(root, &manifest)?;
    verify_required_file_declarations(&manifest)?;
    Ok(manifest_summary(&manifest))
}

fn parse_manifest_file(path: &Path) -> Result<ScannerAssetManifest, ScannerAssetsError> {
    let file = File::open(path).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.install.manifest.invalid",
            false,
            format!(
                "Failed to open scanner asset manifest {}: {error}",
                scanner_resource::path_to_string(path)
            ),
        )
    })?;
    serde_json::from_reader(BufReader::new(file)).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.install.manifest.invalid",
            false,
            format!("Failed to parse scanner asset manifest: {error}"),
        )
    })
}

fn verify_manifest_metadata(
    manifest: &ScannerAssetManifest,
    version_policy: ManifestVersionPolicy,
) -> Result<(), ScannerAssetsError> {
    if manifest.schema_version != 1 {
        return Err(ScannerAssetsError::with_details(
            "assets.install.manifest.invalid",
            false,
            format!(
                "Unsupported scanner asset manifest schema version {}.",
                manifest.schema_version
            ),
        ));
    }
    if let ManifestVersionPolicy::RequireTag(expected_tag) = version_policy {
        if manifest.asset_version != expected_tag {
            return Err(ScannerAssetsError::with_details(
                "assets.install.manifest.versionMismatch",
                false,
                format!(
                    "Scanner asset package version {} does not match expected tag {}.",
                    manifest.asset_version, expected_tag
                ),
            ));
        }
    }
    if manifest.platform_target != scanner_platform::platform_target() {
        return Err(ScannerAssetsError::with_details(
            "assets.install.manifest.platformMismatch",
            false,
            format!(
                "Scanner asset package target {} does not match current target {}.",
                manifest.platform_target,
                scanner_platform::platform_target()
            ),
        ));
    }
    if manifest.files.is_empty() {
        return Err(ScannerAssetsError::with_details(
            "assets.install.manifest.invalid",
            false,
            "Scanner asset manifest files list is empty.",
        ));
    }

    Ok(())
}

fn verify_declared_files(
    root: &Path,
    manifest: &ScannerAssetManifest,
) -> Result<(), ScannerAssetsError> {
    let canonical_root = root.canonicalize().map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.install.verify.fileMissing",
            false,
            format!(
                "Failed to canonicalize scanner asset root {}: {error}",
                scanner_resource::path_to_string(root)
            ),
        )
    })?;

    for file in &manifest.files {
        if file.source_url.trim().is_empty() {
            return Err(ScannerAssetsError::with_details(
                "assets.install.manifest.invalid",
                false,
                format!("Manifest entry {} is missing sourceUrl.", file.path),
            ));
        }
        let safe_relative_path = validate_manifest_relative_path(&file.path)?;
        let resolved_path = root.join(&safe_relative_path);
        if !resolved_path.is_file() {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.fileMissing",
                false,
                format!(
                    "Manifest entry {} is missing at {}.",
                    file.path,
                    scanner_resource::path_to_string(&resolved_path)
                ),
            ));
        }

        let canonical_file = resolved_path.canonicalize().map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.install.verify.fileMissing",
                false,
                format!(
                    "Failed to canonicalize manifest entry {}: {error}",
                    scanner_resource::path_to_string(&resolved_path)
                ),
            )
        })?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.unsafePath",
                false,
                format!("Manifest entry {} resolves outside asset root.", file.path),
            ));
        }

        let actual_size = fs::metadata(&resolved_path)
            .map_err(|error| {
                ScannerAssetsError::with_details(
                    "assets.install.verify.fileMissing",
                    false,
                    format!("Failed to read metadata for {}: {error}", file.path),
                )
            })?
            .len();
        if actual_size != file.size {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.sizeMismatch",
                false,
                format!(
                    "{} size mismatch: manifest={} actual={}.",
                    file.path, file.size, actual_size
                ),
            ));
        }

        let expected_sha256 = normalize_sha256(&file.sha256)?;
        let actual_sha256 = sha256_file_hex(&resolved_path)?;
        if actual_sha256 != expected_sha256 {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.checksumMismatch",
                false,
                format!("{} sha256 mismatch.", file.path),
            ));
        }
    }

    Ok(())
}

fn verify_required_file_declarations(
    manifest: &ScannerAssetManifest,
) -> Result<(), ScannerAssetsError> {
    let declared = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();

    for required_path in required_asset_paths_for_current_platform() {
        if !declared.contains(required_path) {
            return Err(ScannerAssetsError::with_details(
                "assets.install.verify.fileMissing",
                false,
                format!("Required scanner asset is not declared: {required_path}."),
            ));
        }
    }

    Ok(())
}

fn required_asset_paths_for_current_platform() -> Vec<&'static str> {
    let mut paths = COMMON_REQUIRED_ASSET_PATHS.to_vec();
    match std::env::consts::OS {
        "windows" => paths.extend_from_slice(WINDOWS_REQUIRED_ASSET_PATHS),
        "linux" => paths.extend_from_slice(LINUX_REQUIRED_ASSET_PATHS),
        _ => {}
    }
    paths
}

fn validate_manifest_relative_path(relative_path: &str) -> Result<PathBuf, ScannerAssetsError> {
    let trimmed = relative_path.trim();
    if trimmed.is_empty()
        || trimmed.contains('\\')
        || trimmed.starts_with('/')
        || trimmed.starts_with("//")
        || looks_like_windows_drive_path(trimmed)
    {
        return Err(ScannerAssetsError::with_details(
            "assets.install.verify.unsafePath",
            false,
            format!("Unsafe scanner asset path: {relative_path}."),
        ));
    }

    let path = Path::new(trimmed);
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            _ => {
                return Err(ScannerAssetsError::with_details(
                    "assets.install.verify.unsafePath",
                    false,
                    format!("Unsafe scanner asset path: {relative_path}."),
                ))
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(ScannerAssetsError::with_details(
            "assets.install.verify.unsafePath",
            false,
            format!("Unsafe scanner asset path: {relative_path}."),
        ));
    }

    Ok(normalized)
}

fn looks_like_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn normalize_sha256(value: &str) -> Result<String, ScannerAssetsError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(ScannerAssetsError::with_details(
            "assets.install.manifest.invalid",
            false,
            "Manifest entry has invalid sha256 value.",
        ));
    }
    Ok(normalized)
}

fn sha256_file_hex(path: &Path) -> Result<String, ScannerAssetsError> {
    let mut file = File::open(path).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.install.verify.fileMissing",
            false,
            format!(
                "Failed to open file for checksum {}: {error}",
                scanner_resource::path_to_string(path)
            ),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_SIZE];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.install.verify.checksumMismatch",
                false,
                format!(
                    "Failed to read file for checksum {}: {error}",
                    scanner_resource::path_to_string(path)
                ),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn activate_staging(paths: &ScannerAssetPaths) -> Result<(), ScannerAssetsError> {
    cleanup_stale_backup_dir(&paths.backup_dir);

    let had_current = paths.current_dir.exists();
    let backup_dir = if had_current {
        Some(select_backup_dir(paths)?)
    } else {
        None
    };

    if let Some(backup_dir) = backup_dir.as_ref() {
        fs::rename(&paths.current_dir, backup_dir).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.install.activate.replaceFailed",
                true,
                format!("Failed to move current scanner assets to backup: {error}"),
            )
        })?;
    }

    if let Err(error) = fs::rename(&paths.staging_dir, &paths.current_dir) {
        if let Some(backup_dir) = backup_dir.as_ref() {
            let _ = fs::rename(backup_dir, &paths.current_dir);
        }
        return Err(ScannerAssetsError::with_details(
            "assets.install.activate.replaceFailed",
            true,
            format!("Failed to activate scanner assets: {error}"),
        ));
    }

    if let Some(backup_dir) = backup_dir.as_ref() {
        let _ = fs::remove_dir_all(backup_dir);
    }

    if backup_dir
        .as_ref()
        .map(|backup_dir| backup_dir != &paths.backup_dir)
        .unwrap_or(true)
    {
        cleanup_stale_backup_dir(&paths.backup_dir);
    }

    Ok(())
}

fn cleanup_stale_backup_dir(backup_dir: &Path) {
    if backup_dir.exists() {
        let _ = fs::remove_dir_all(backup_dir);
    }
}

fn select_backup_dir(paths: &ScannerAssetPaths) -> Result<PathBuf, ScannerAssetsError> {
    if !paths.backup_dir.exists() {
        return Ok(paths.backup_dir.clone());
    }

    for suffix in 1..=1000 {
        let candidate = paths.assets_dir.join(format!("{BACKUP_DIR_NAME}.{suffix}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(ScannerAssetsError::with_details(
        "assets.install.activate.replaceFailed",
        true,
        format!(
            "Failed to select a backup directory under {}.",
            scanner_resource::path_to_string(&paths.assets_dir)
        ),
    ))
}

fn manifest_summary(manifest: &ScannerAssetManifest) -> ScannerAssetsManifestSummary {
    ScannerAssetsManifestSummary {
        schema_version: manifest.schema_version,
        asset_version: manifest.asset_version.clone(),
        platform_target: manifest.platform_target.clone(),
    }
}

#[cfg(not(test))]
fn begin_ort_asset_mutation(
) -> Result<crate::scanner_ort::ScannerOrtAssetMutationGuard, ScannerAssetsError> {
    crate::scanner_ort::begin_ort_asset_mutation()
}

#[cfg(test)]
struct OrtAssetMutationGuard;

#[cfg(test)]
thread_local! {
    static TEST_ORT_ASSET_MUTATION_CALLS: std::cell::Cell<usize> = std::cell::Cell::new(0);
    static TEST_ORT_ASSET_MUTATION_FAIL: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

#[cfg(test)]
fn begin_ort_asset_mutation() -> Result<OrtAssetMutationGuard, ScannerAssetsError> {
    TEST_ORT_ASSET_MUTATION_CALLS.with(|calls| calls.set(calls.get() + 1));
    let should_fail = TEST_ORT_ASSET_MUTATION_FAIL.with(|fail| fail.get());
    if should_fail {
        return Err(ScannerAssetsError::with_details(
            "runtime.worker.stopFailed",
            true,
            "Failed to stop scanner ORT worker before asset mutation.",
        ));
    }
    Ok(OrtAssetMutationGuard)
}
