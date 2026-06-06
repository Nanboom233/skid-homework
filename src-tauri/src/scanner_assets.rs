use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
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
    fn new(code: impl Into<String>, retryable: bool, details: impl Into<Option<String>>) -> Self {
        Self {
            code: code.into(),
            retryable,
            details: details.into(),
        }
    }

    fn with_details(code: impl Into<String>, retryable: bool, details: impl Into<String>) -> Self {
        Self::new(code, retryable, Some(details.into()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationKind {
    Downloading,
    Importing,
}

impl OperationKind {
    fn state(self) -> &'static str {
        match self {
            OperationKind::Downloading => "downloading",
            OperationKind::Importing => "importing",
        }
    }
}

#[derive(Debug)]
struct OperationGuard {
    kind: OperationKind,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut state = operation_state()
            .lock()
            .expect("scanner asset operation mutex should not be poisoned");
        if *state == Some(self.kind) {
            *state = None;
        }
    }
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
    progress_channel: Channel<ScannerAssetsProgress>,
) -> Result<ScannerAssetsInstallResult, ScannerAssetsError> {
    let guard = match try_acquire_operation(OperationKind::Downloading) {
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

    let _guard = guard;
    finish_progress_operation(
        &progress_channel,
        download_and_install(paths, &progress_channel).await,
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
    format!(
        "{}-{}.{}",
        scanner_platform::platform_target(),
        EXPECTED_SCANNER_ASSET_TAG,
        platform_archive_extension()
    )
}

fn default_asset_url() -> String {
    format!(
        "https://github.com/{}/{}/releases/download/{}/{}",
        SCANNER_ASSETS_REPO_OWNER,
        SCANNER_ASSETS_REPO_NAME,
        EXPECTED_SCANNER_ASSET_TAG,
        platform_package_file_name()
    )
}

fn operation_state() -> &'static Mutex<Option<OperationKind>> {
    static STATE: OnceLock<Mutex<Option<OperationKind>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

fn current_operation() -> Option<OperationKind> {
    *operation_state()
        .lock()
        .expect("scanner asset operation mutex should not be poisoned")
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

    *state = Some(kind);
    Ok(OperationGuard { kind })
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
            set_last_error(error.clone());
            send_failed_progress(channel, error.clone());
            Err(error)
        }
    }
}

async fn download_and_install(
    paths: ScannerAssetPaths,
    channel: &Channel<ScannerAssetsProgress>,
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

    let url = default_asset_url();
    let archive_path = paths.assets_dir.join(format!(
        ".download-{}",
        platform_package_file_name().replace('/', "-")
    ));

    let result = match download_archive_to_path(&url, &archive_path, channel).await {
        Ok(()) => {
            let paths_for_install = paths.clone();
            let archive_path_for_install = archive_path.clone();
            let channel_for_install = channel.clone();
            tauri::async_runtime::spawn_blocking(move || {
                install_archive(
                    &paths_for_install,
                    &archive_path_for_install,
                    &channel_for_install,
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
    };

    let _ = fs::remove_file(&archive_path);
    if result.is_err() {
        let _ = fs::remove_dir_all(&paths.staging_dir);
    }
    result
}

async fn download_archive_to_path(
    url: &str,
    archive_path: &Path,
    channel: &Channel<ScannerAssetsProgress>,
) -> Result<(), ScannerAssetsError> {
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

    while let Some(chunk) = response.chunk().await.map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.download.fetch.networkFailed",
            true,
            format!("Failed while reading scanner asset package response: {error}"),
        )
    })? {
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
    writer.flush().map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.download.fetch.writeFailed",
            true,
            format!("Failed to flush scanner asset package: {error}"),
        )
    })?;

    Ok(())
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

        install_archive(&paths, &archive_path, channel)
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
) -> Result<ScannerAssetsInstallResult, ScannerAssetsError> {
    prepare_staging(paths)?;

    let result = (|| {
        send_phase_progress(channel, "unpacking");
        extract_archive_for_current_platform(archive_path, &paths.staging_dir)?;

        send_phase_progress(channel, "verifying");
        let manifest = load_and_verify_install_root(&paths.staging_dir)?;
        let summary = manifest_summary(&manifest);

        send_phase_progress(channel, "activating");
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

fn load_and_verify_install_root(root: &Path) -> Result<ScannerAssetManifest, ScannerAssetsError> {
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
    verify_manifest_metadata(&manifest)?;
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
    verify_manifest_metadata(&manifest)?;
    verify_required_file_declarations(&manifest)?;
    verify_required_files_exist(root, &manifest)?;
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

#[cfg(test)]
fn parse_manifest_bytes(bytes: &[u8]) -> Result<ScannerAssetManifest, ScannerAssetsError> {
    serde_json::from_slice(bytes).map_err(|error| {
        ScannerAssetsError::with_details(
            "assets.install.manifest.invalid",
            false,
            format!("Failed to parse scanner asset manifest: {error}"),
        )
    })
}

fn verify_manifest_metadata(manifest: &ScannerAssetManifest) -> Result<(), ScannerAssetsError> {
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
    if manifest.asset_version != EXPECTED_SCANNER_ASSET_TAG {
        return Err(ScannerAssetsError::with_details(
            "assets.install.manifest.versionMismatch",
            false,
            format!(
                "Scanner asset package version {} does not match expected tag {}.",
                manifest.asset_version, EXPECTED_SCANNER_ASSET_TAG
            ),
        ));
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

fn verify_required_files_exist(
    root: &Path,
    manifest: &ScannerAssetManifest,
) -> Result<(), ScannerAssetsError> {
    for file in &manifest.files {
        let safe_relative_path = validate_manifest_relative_path(&file.path)?;
        let resolved_path = root.join(safe_relative_path);
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
    if paths.backup_dir.exists() {
        fs::remove_dir_all(&paths.backup_dir).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.install.activate.replaceFailed",
                true,
                format!(
                    "Failed to clear backup directory {}: {error}",
                    scanner_resource::path_to_string(&paths.backup_dir)
                ),
            )
        })?;
    }

    let had_current = paths.current_dir.exists();
    if had_current {
        fs::rename(&paths.current_dir, &paths.backup_dir).map_err(|error| {
            ScannerAssetsError::with_details(
                "assets.install.activate.replaceFailed",
                true,
                format!("Failed to move current scanner assets to backup: {error}"),
            )
        })?;
    }

    if let Err(error) = fs::rename(&paths.staging_dir, &paths.current_dir) {
        if had_current {
            let _ = fs::rename(&paths.backup_dir, &paths.current_dir);
        }
        return Err(ScannerAssetsError::with_details(
            "assets.install.activate.replaceFailed",
            true,
            format!("Failed to activate scanner assets: {error}"),
        ));
    }

    if had_current {
        let _ = fs::remove_dir_all(&paths.backup_dir);
    }

    Ok(())
}

fn manifest_summary(manifest: &ScannerAssetManifest) -> ScannerAssetsManifestSummary {
    ScannerAssetsManifestSummary {
        schema_version: manifest.schema_version,
        asset_version: manifest.asset_version.clone(),
        platform_target: manifest.platform_target.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with_files(files: Vec<ScannerAssetManifestFile>) -> ScannerAssetManifest {
        ScannerAssetManifest {
            schema_version: 1,
            asset_version: EXPECTED_SCANNER_ASSET_TAG.to_string(),
            platform_target: scanner_platform::platform_target().to_string(),
            files,
        }
    }

    fn manifest_file(path: &str, bytes: &[u8]) -> ScannerAssetManifestFile {
        ScannerAssetManifestFile {
            path: path.to_string(),
            size: bytes.len() as u64,
            sha256: hex_lower(&Sha256::digest(bytes)),
            source_url: format!("https://official.example/{path}"),
        }
    }

    #[test]
    fn package_file_name_and_default_url_use_expected_tag_without_prefix() {
        let file_name = platform_package_file_name();
        let expected_release_prefix = format!(
            "https://github.com/{SCANNER_ASSETS_REPO_OWNER}/{SCANNER_ASSETS_REPO_NAME}/releases/download/{EXPECTED_SCANNER_ASSET_TAG}/"
        );
        assert!(file_name.contains(EXPECTED_SCANNER_ASSET_TAG));
        assert!(!file_name.starts_with("scanner-assets-"));
        assert!(default_asset_url().ends_with(&file_name));
        assert!(default_asset_url().starts_with(&expected_release_prefix));
        assert!(!default_asset_url().contains("/latest/"));
    }

    #[test]
    fn download_timeouts_are_configured() {
        assert_eq!(DOWNLOAD_CONNECT_TIMEOUT, Duration::from_secs(15));
        assert_eq!(DOWNLOAD_READ_TIMEOUT, Duration::from_secs(60));
    }

    #[test]
    fn manifest_rejects_unknown_fields() {
        let manifest = br#"{
            "schemaVersion": 1,
            "assetVersion": "v0.1.0",
            "platformTarget": "windows-directml",
            "archiveFormat": "zip",
            "files": []
        }"#;

        let error = parse_manifest_bytes(manifest).expect_err("unknown fields must fail");
        assert_eq!(error.code, "assets.install.manifest.invalid");
    }

    #[test]
    fn manifest_rejects_unsupported_schema_version() {
        let manifest = ScannerAssetManifest {
            schema_version: 2,
            asset_version: EXPECTED_SCANNER_ASSET_TAG.to_string(),
            platform_target: scanner_platform::platform_target().to_string(),
            files: vec![manifest_file("models/uvdoc-best-model.onnx", b"model")],
        };

        let error = verify_manifest_metadata(&manifest).expect_err("schema version must fail");
        assert_eq!(error.code, "assets.install.manifest.invalid");
    }

    #[test]
    fn unsafe_manifest_paths_are_rejected() {
        for path in [
            "",
            "../model.onnx",
            "models/../model.onnx",
            "/models/model.onnx",
            "//server/share/model.onnx",
            r"models\model.onnx",
            "C:/models/model.onnx",
            "./models/model.onnx",
        ] {
            let error = validate_manifest_relative_path(path).expect_err(path);
            assert_eq!(error.code, "assets.install.verify.unsafePath");
        }

        assert_eq!(
            validate_manifest_relative_path("models/uvdoc-best-model.onnx").unwrap(),
            PathBuf::from("models").join("uvdoc-best-model.onnx")
        );
    }

    #[test]
    fn platform_mismatch_is_rejected() {
        let manifest = ScannerAssetManifest {
            schema_version: 1,
            asset_version: EXPECTED_SCANNER_ASSET_TAG.to_string(),
            platform_target: "wrong-platform".to_string(),
            files: vec![manifest_file("models/uvdoc-best-model.onnx", b"model")],
        };

        let error = verify_manifest_metadata(&manifest).expect_err("platform mismatch must fail");
        assert_eq!(error.code, "assets.install.manifest.platformMismatch");
    }

    #[test]
    fn missing_required_declaration_is_rejected() {
        let manifest = manifest_with_files(vec![manifest_file(
            "models/uvdoc-best-model.onnx",
            b"model",
        )]);

        let error =
            verify_required_file_declarations(&manifest).expect_err("missing required must fail");
        assert_eq!(error.code, "assets.install.verify.fileMissing");
    }

    #[test]
    fn declared_missing_file_is_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manifest = manifest_with_files(vec![manifest_file("models/missing.onnx", b"model")]);

        let error =
            verify_declared_files(temp_dir.path(), &manifest).expect_err("missing file must fail");
        assert_eq!(error.code, "assets.install.verify.fileMissing");
    }

    #[test]
    fn declared_size_mismatch_is_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("models").join("model.onnx");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(&file_path, b"actual").unwrap();
        let mut entry = manifest_file("models/model.onnx", b"actual");
        entry.size += 1;
        let manifest = manifest_with_files(vec![entry]);

        let error =
            verify_declared_files(temp_dir.path(), &manifest).expect_err("size mismatch must fail");
        assert_eq!(error.code, "assets.install.verify.sizeMismatch");
    }

    #[test]
    fn declared_checksum_mismatch_is_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("models").join("model.onnx");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(&file_path, b"actual").unwrap();
        let entry = manifest_file("models/model.onnx", b"other!");
        let manifest = manifest_with_files(vec![entry]);

        let error = verify_declared_files(temp_dir.path(), &manifest)
            .expect_err("checksum mismatch must fail");
        assert_eq!(error.code, "assets.install.verify.checksumMismatch");
    }

    #[test]
    fn operation_mutex_rejects_concurrent_operations() {
        let guard = try_acquire_operation(OperationKind::Importing).unwrap();
        let error = try_acquire_operation(OperationKind::Downloading)
            .expect_err("second operation must fail");
        assert_eq!(error.code, "assets.operation.inProgress");
        drop(guard);

        let guard = try_acquire_operation(OperationKind::Downloading).unwrap();
        drop(guard);
    }

    #[test]
    fn cleanup_staging_preserves_current_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
        fs::create_dir_all(&paths.current_dir).unwrap();
        fs::write(paths.current_dir.join("marker"), b"current").unwrap();
        fs::create_dir_all(&paths.staging_dir).unwrap();
        fs::write(paths.staging_dir.join("marker"), b"staging").unwrap();

        fs::remove_dir_all(&paths.staging_dir).unwrap();

        assert_eq!(
            fs::read(paths.current_dir.join("marker")).unwrap(),
            b"current"
        );
    }
}
