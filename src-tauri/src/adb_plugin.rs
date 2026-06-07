/// Native ADB bridge for Tauri desktop builds.
/// This replaces WebUSB/WebADB from the browser environment with direct
/// host-level access to the local `adb` executable.
use std::{
    env,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Mutex as StdMutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use serde::{Deserialize, Serialize};
use tauri::command;
use tauri::ipc::{Channel, InvokeResponseBody};

static ADB_EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();

const CONNECT_READY_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const ADB_SERVER_RECOVERY_RETRY_DELAY: Duration = Duration::from_millis(200);

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdbPairRequest {
    pub address: String,
    pub pairing_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdbConnectRequest {
    pub address: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AdbDeviceInfo {
    pub serial: String,
    pub name: String,
    pub state: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdbConnectResponse {
    pub serial: String,
    pub message: String,
}

fn adb_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "adb.exe"
    } else {
        "adb"
    }
}

fn candidate_from_sdk_root(root: &Path) -> PathBuf {
    root.join("platform-tools").join(adb_binary_name())
}

fn push_env_candidate(candidates: &mut Vec<PathBuf>, env_name: &str) {
    if let Ok(value) = env::var(env_name) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            candidates.push(candidate_from_sdk_root(Path::new(trimmed)));
        }
    }
}

fn discover_adb_executable() -> PathBuf {
    if let Ok(value) = env::var("ADB_PATH") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let mut candidates = Vec::new();
    push_env_candidate(&mut candidates, "ANDROID_HOME");
    push_env_candidate(&mut candidates, "ANDROID_SDK_ROOT");

    if cfg!(target_os = "windows") {
        if let Ok(local_app_data) = env::var("LOCALAPPDATA") {
            candidates.push(candidate_from_sdk_root(
                &PathBuf::from(local_app_data).join("Android").join("Sdk"),
            ));
        }
    }

    if cfg!(target_os = "macos") {
        if let Ok(home) = env::var("HOME") {
            candidates.push(candidate_from_sdk_root(
                &PathBuf::from(home)
                    .join("Library")
                    .join("Android")
                    .join("sdk"),
            ));
        }
    }

    if cfg!(target_os = "linux") {
        if let Ok(home) = env::var("HOME") {
            candidates.push(candidate_from_sdk_root(
                &PathBuf::from(home).join("Android").join("Sdk"),
            ));
        }
    }

    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(adb_binary_name()))
}

fn resolve_adb_executable() -> PathBuf {
    ADB_EXECUTABLE.get_or_init(discover_adb_executable).clone()
}

fn configure_adb_command(_command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        // Prevent adb.exe from flashing a console window for background desktop operations.
        _command.creation_flags(CREATE_NO_WINDOW);
    }
}

pub(crate) fn normalize_text_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .trim()
        .to_string()
}

pub(crate) fn combine_command_output(output: &Output) -> String {
    let stdout = normalize_text_output(&output.stdout);
    let stderr = normalize_text_output(&output.stderr);

    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => String::new(),
    }
}

pub(crate) fn ensure_non_empty(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required."));
    }

    Ok(trimmed.to_string())
}

fn validate_adb_remote_address(value: &str, field_name: &str) -> Result<String, String> {
    let address = ensure_non_empty(value, field_name)?;

    if address.len() > 255 {
        return Err(format!("{field_name} must be 255 characters or less."));
    }

    if address.chars().any(char::is_whitespace) {
        return Err(format!("{field_name} must not contain whitespace."));
    }

    if address.contains("://") {
        return Err(format!(
            "{field_name} must be a host:port value, not a URL."
        ));
    }

    if address.contains('/') || address.contains('\\') {
        return Err(format!("{field_name} must not contain path separators."));
    }

    let (host, port_text) = address
        .rsplit_once(':')
        .ok_or_else(|| format!("{field_name} must use the host:port format."))?;

    if host.is_empty() || port_text.is_empty() {
        return Err(format!("{field_name} must use the host:port format."));
    }

    let port = port_text
        .parse::<u16>()
        .map_err(|_| format!("{field_name} port must be a number from 1 to 65535."))?;
    if port == 0 {
        return Err(format!(
            "{field_name} port must be a number from 1 to 65535."
        ));
    }

    if !host
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':' | '[' | ']'))
    {
        return Err(format!(
            "{field_name} host contains unsupported characters."
        ));
    }

    Ok(address)
}

fn validate_pairing_code(value: &str) -> Result<String, String> {
    let pairing_code = ensure_non_empty(value, "Pairing code")?;
    if pairing_code.len() != 6 || !pairing_code.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("Pairing code must be exactly 6 digits.".to_string());
    }
    Ok(pairing_code)
}

pub(crate) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn build_app_process_shell_command(
    classpath: &str,
    main_class: &str,
    main_args: &[String],
) -> String {
    let mut command_parts = vec![
        format!("CLASSPATH={}", shell_single_quote(classpath)),
        "app_process".to_string(),
        "/".to_string(),
        shell_single_quote(main_class),
    ];
    command_parts.extend(main_args.iter().map(|value| shell_single_quote(value)));

    command_parts.join(" ")
}

pub(crate) fn wrap_shell_c_script(script: &str) -> String {
    shell_single_quote(script)
}

pub(crate) fn run_adb_command(args: &[String]) -> Result<Output, String> {
    let executable = resolve_adb_executable();
    let mut command = Command::new(&executable);
    configure_adb_command(&mut command);

    command
        .args(args)
        .output()
        .map_err(|error| match error.kind() {
            ErrorKind::NotFound => format!(
                "ADB executable was not found. Install Android Platform Tools or set the ADB_PATH environment variable. Looked for `{}`.",
                executable.display()
            ),
            _ => format!("Failed to launch `{}`: {error}", executable.display()),
        })
}

fn build_failed_action_error(action: &str, output: &Output) -> String {
    let details = combine_command_output(output);
    if details.is_empty() {
        format!("{action} failed with status {}.", output.status)
    } else {
        format!("{action} failed: {details}")
    }
}

fn is_adb_server_recoverable_failure(details: &str) -> bool {
    let normalized = details.to_ascii_lowercase();
    [
        "daemon not running",
        "failed to start daemon",
        "cannot connect to daemon",
        "could not read ok from adb server",
        "failed to check server version",
    ]
    .iter()
    .any(|pattern| normalized.contains(pattern))
}

fn is_adb_server_management_command(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some("start-server") | Some("kill-server")
    )
}

fn run_adb_management_command(args: &[&str], action: &str) -> Result<Output, String> {
    let owned_args = args
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<String>>();
    let output = run_adb_command(&owned_args)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(build_failed_action_error(action, &output))
    }
}

fn recover_adb_server() -> Result<(), String> {
    let first_start_error = match run_adb_management_command(&["start-server"], "adb start-server")
    {
        Ok(_) => return Ok(()),
        Err(error) => error,
    };

    let mut recovery_messages = vec![format!(
        "Initial adb start-server attempt failed: {first_start_error}"
    )];

    match run_adb_command(&["kill-server".to_string()]) {
        Ok(output) if !output.status.success() => {
            let details = combine_command_output(&output);
            if !details.is_empty() {
                recovery_messages.push(format!("adb kill-server reported: {details}"));
            }
        }
        Err(error) => {
            recovery_messages.push(format!("Failed to launch adb kill-server: {error}"));
        }
        Ok(_) => {}
    }

    thread::sleep(ADB_SERVER_RECOVERY_RETRY_DELAY);

    match run_adb_management_command(&["start-server"], "adb start-server") {
        Ok(_) => Ok(()),
        Err(error) => {
            recovery_messages.push(format!("Retry adb start-server attempt failed: {error}"));
            Err(recovery_messages.join("\n"))
        }
    }
}

pub(crate) fn run_adb_checked(args: &[String], action: &str) -> Result<Output, String> {
    let output = run_adb_command(args)?;

    if output.status.success() {
        return Ok(output);
    }

    let details = combine_command_output(&output);
    if !details.is_empty()
        && !is_adb_server_management_command(args)
        && is_adb_server_recoverable_failure(&details)
    {
        if let Err(recovery_error) = recover_adb_server() {
            return Err(format!(
                "{action} failed: {details}\nADB server auto-recovery failed: {recovery_error}"
            ));
        }

        let retried_output = run_adb_command(args)?;
        if retried_output.status.success() {
            return Ok(retried_output);
        }

        return Err(build_failed_action_error(
            &format!("{action} after ADB server auto-recovery"),
            &retried_output,
        ));
    }

    Err(build_failed_action_error(action, &output))
}

fn find_device_attribute(attributes: &[&str], key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    attributes
        .iter()
        .find_map(|attribute| attribute.strip_prefix(&prefix))
        .map(|value| value.replace('_', " "))
}

fn list_devices_inner() -> Result<Vec<AdbDeviceInfo>, String> {
    let args = vec!["devices".to_string(), "-l".to_string()];
    let output = run_adb_checked(&args, "adb devices")?;
    let stdout = normalize_text_output(&output.stdout);

    let mut devices = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('*') || trimmed == "List of devices attached" {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        let serial = parts[0].to_string();
        let state = parts[1].to_string();
        let name = find_device_attribute(&parts[2..], "model")
            .or_else(|| find_device_attribute(&parts[2..], "device"))
            .or_else(|| find_device_attribute(&parts[2..], "product"))
            .unwrap_or_else(|| serial.clone());

        devices.push(AdbDeviceInfo {
            serial,
            name,
            state,
        });
    }

    Ok(devices)
}

fn address_host(address: &str) -> Option<String> {
    address
        .rsplit_once(':')
        .map(|(host, _)| host.trim_matches(&['[', ']'][..]).to_string())
}

fn find_ready_device<'a>(devices: &'a [AdbDeviceInfo], address: &str) -> Option<&'a AdbDeviceInfo> {
    if let Some(device) = devices
        .iter()
        .find(|device| device.state == "device" && device.serial == address)
    {
        return Some(device);
    }

    let target_host = address_host(address)?;

    devices.iter().find(|device| {
        device.state == "device"
            && address_host(&device.serial)
                .map(|serial_host| serial_host == target_host)
                .unwrap_or(false)
    })
}

fn extract_connected_serial(message: &str) -> Option<String> {
    for line in message.lines() {
        let trimmed = line.trim();
        if let Some(serial) = trimmed.strip_prefix("connected to ") {
            return Some(serial.trim().to_string());
        }

        if let Some(serial) = trimmed.strip_prefix("already connected to ") {
            return Some(serial.trim().to_string());
        }
    }

    None
}

fn wait_for_ready_device(
    address: &str,
    serial_hint: Option<&str>,
) -> Result<AdbDeviceInfo, String> {
    let deadline = Instant::now() + CONNECT_READY_TIMEOUT;

    loop {
        let devices = list_devices_inner()?;
        if let Some(serial_hint) = serial_hint {
            if let Some(device) = find_ready_device(&devices, serial_hint) {
                return Ok(device.clone());
            }
        }

        if let Some(device) = find_ready_device(&devices, address) {
            return Ok(device.clone());
        }

        if Instant::now() >= deadline {
            let visible_devices = devices
                .iter()
                .map(|device| format!("{} ({})", device.serial, device.state))
                .collect::<Vec<String>>()
                .join(", ");

            if visible_devices.is_empty() {
                return Err(format!(
                    "Connected to {address}, but no ready ADB device appeared before the timeout."
                ));
            }

            return Err(format!(
                "Connected to {address}, but the device was not ready before the timeout. Visible devices: {visible_devices}"
            ));
        }

        thread::sleep(CONNECT_READY_POLL_INTERVAL);
    }
}

#[command]
pub async fn tauri_adb_list_devices() -> Result<Vec<AdbDeviceInfo>, String> {
    tauri::async_runtime::spawn_blocking(list_devices_inner)
        .await
        .map_err(|error| format!("ADB device listing task failed: {error}"))?
}

/// Pair with a remote Android device over wireless debugging.
#[command]
pub async fn tauri_adb_pair(request: AdbPairRequest) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let address = validate_adb_remote_address(&request.address, "Pairing address")?;
        let pairing_code = validate_pairing_code(&request.pairing_code)?;

        let args = vec!["pair".to_string(), address.clone(), pairing_code];
        let output = run_adb_checked(&args, &format!("adb pair {address}"))?;
        let message = combine_command_output(&output);

        if message.is_empty() {
            Ok(format!("Successfully paired to {address}."))
        } else {
            Ok(message)
        }
    })
    .await
    .map_err(|error| format!("ADB pairing task failed: {error}"))?
}

/// Connect to a remote ADB endpoint and wait until the device is ready.
#[command]
pub async fn tauri_adb_connect(request: AdbConnectRequest) -> Result<AdbConnectResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let address = validate_adb_remote_address(&request.address, "Remote ADB address")?;

        let args = vec!["connect".to_string(), address.clone()];
        let output = run_adb_checked(&args, &format!("adb connect {address}"))?;
        let message = combine_command_output(&output);
        let serial_hint = extract_connected_serial(&message);
        let device = wait_for_ready_device(&address, serial_hint.as_deref())?;

        Ok(AdbConnectResponse {
            serial: device.serial.clone(),
            message: if message.is_empty() {
                format!("Connected to {}.", device.serial)
            } else {
                message
            },
        })
    })
    .await
    .map_err(|error| format!("ADB connect task failed: {error}"))?
}

/// Push a local file to the device filesystem.
#[command]
pub async fn tauri_adb_push(
    serial: String,
    local_path: String,
    remote_path: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let serial = ensure_non_empty(&serial, "ADB serial")?;
        let local_path = ensure_non_empty(&local_path, "Local file path")?;
        let remote_path = ensure_non_empty(&remote_path, "Remote file path")?;

        let args = vec![
            "-s".to_string(),
            serial.clone(),
            "push".to_string(),
            local_path,
            remote_path.clone(),
        ];
        let output = run_adb_checked(&args, &format!("adb -s {serial} push -> {remote_path}"))?;

        Ok(combine_command_output(&output))
    })
    .await
    .map_err(|error| format!("ADB push task failed: {error}"))?
}

/// Set up TCP port forwarding to a device-side abstract socket.
#[command]
pub async fn tauri_adb_forward(
    serial: String,
    local_port: u16,
    remote_socket_name: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let serial = ensure_non_empty(&serial, "ADB serial")?;
        let remote_socket_name = ensure_non_empty(&remote_socket_name, "Remote socket name")?;

        let args = vec![
            "-s".to_string(),
            serial.clone(),
            "forward".to_string(),
            format!("tcp:{local_port}"),
            format!("localabstract:{remote_socket_name}"),
        ];
        let output = run_adb_checked(
            &args,
            &format!("adb -s {serial} forward tcp:{local_port} localabstract:{remote_socket_name}"),
        )?;

        Ok(combine_command_output(&output))
    })
    .await
    .map_err(|error| format!("ADB forward task failed: {error}"))?
}

/// Remove a previously established TCP port forward.
#[command]
pub async fn tauri_adb_remove_forward(serial: String, local_port: u16) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let serial = ensure_non_empty(&serial, "ADB serial")?;

        let args = vec![
            "-s".to_string(),
            serial.clone(),
            "forward".to_string(),
            "--remove".to_string(),
            format!("tcp:{local_port}"),
        ];
        let output = run_adb_checked(
            &args,
            &format!("adb -s {serial} forward --remove tcp:{local_port}"),
        )?;

        Ok(combine_command_output(&output))
    })
    .await
    .map_err(|error| format!("ADB remove-forward task failed: {error}"))?
}

// ---------------------------------------------------------------------------
// Scanner camera-server lifecycle
// ---------------------------------------------------------------------------

const SCANNER_SERVER_MAIN_CLASS: &str = "com.skidhomework.server.Server";
const SCANNER_SERVER_PID_PATH: &str = "/data/local/tmp/skid-scanner-server.pid";
const SCANNER_SERVER_LOG_PATH: &str = "/data/local/tmp/skid-scanner-server.log";
const SCANNER_SERVER_READY_SENTINEL: &str = "SCANNER_SERVER_READY";

fn build_scanner_server_kill_script(main_class: &str) -> String {
    format!(
        "killed=0; \
for pid in $(ps -A -o PID,ARGS 2>/dev/null | grep {main_class} | grep -v grep | awk '{{print $1}}'); do \
  kill \"$pid\" >/dev/null 2>&1 && killed=1; \
done; \
sleep 1; \
for pid in $(ps -A -o PID,ARGS 2>/dev/null | grep {main_class} | grep -v grep | awk '{{print $1}}'); do \
  kill -9 \"$pid\" >/dev/null 2>&1 && killed=1; \
done; \
if [ \"$killed\" -eq 1 ]; then \
  sleep 1; \
fi",
        main_class = shell_single_quote(main_class),
    )
}

fn build_scanner_server_start_script(
    classpath: &str,
    main_class: &str,
    server_args: &[String],
) -> String {
    let kill_command = build_scanner_server_kill_script(main_class);
    let app_process_command = build_app_process_shell_command(classpath, main_class, server_args);

    format!(
        "{kill_command}; \
rm -f {pidfile}; \
: >{logfile}; \
{app_process_command} </dev/null >>{logfile} 2>&1 & echo $! > {pidfile}",
        pidfile = shell_single_quote(SCANNER_SERVER_PID_PATH),
        logfile = shell_single_quote(SCANNER_SERVER_LOG_PATH),
    )
}

fn build_scanner_server_stop_script(main_class: &str) -> String {
    let kill_command = build_scanner_server_kill_script(main_class);

    format!(
        "pidfile={pidfile}; \
stopped=0; \
if [ -f \"$pidfile\" ]; then \
  pid=$(cat \"$pidfile\"); \
  if [ -n \"$pid\" ]; then \
    kill \"$pid\" >/dev/null 2>&1 && stopped=1; \
  fi; \
  rm -f \"$pidfile\"; \
fi; \
{kill_command}; \
if [ \"$stopped\" -eq 1 ]; then \
  echo \"Camera server stopped.\"; \
fi",
        pidfile = shell_single_quote(SCANNER_SERVER_PID_PATH),
    )
}

/// Start the Android camera server on the selected device.
#[command]
pub async fn tauri_adb_start_server(
    serial: String,
    classpath: String,
    main_class: String,
    server_args: Vec<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let serial = ensure_non_empty(&serial, "ADB serial")?;
        let classpath = ensure_non_empty(&classpath, "Server classpath")?;
        let main_class = ensure_non_empty(&main_class, "Server main class")?;
        let shell_command =
            build_scanner_server_start_script(&classpath, &main_class, &server_args);
        let args = vec![
            "-s".to_string(),
            serial.clone(),
            "shell".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            wrap_shell_c_script(&shell_command),
        ];
        let output = run_adb_checked(
            &args,
            &format!("adb -s {serial} shell sh -c <start scanner camera server>"),
        )?;
        let message = combine_command_output(&output);

        if message.is_empty() {
            Ok(format!("Camera server started on {serial}."))
        } else {
            Ok(message)
        }
    })
    .await
    .map_err(|error| format!("ADB start-server task failed: {error}"))?
}

/// Stop the Android camera server on the selected device.
#[command]
pub async fn tauri_adb_stop_server(serial: String, classpath: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let serial = ensure_non_empty(&serial, "ADB serial")?;
        let _classpath = ensure_non_empty(&classpath, "Server classpath")?;
        let shell_command = build_scanner_server_stop_script(SCANNER_SERVER_MAIN_CLASS);
        let args = vec![
            "-s".to_string(),
            serial.clone(),
            "shell".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            wrap_shell_c_script(&shell_command),
        ];
        let output = run_adb_checked(
            &args,
            &format!("adb -s {serial} shell sh -c <stop scanner camera server>"),
        )?;
        let message = combine_command_output(&output);

        if message.is_empty() {
            Ok(format!("Camera server stopped on {serial}."))
        } else {
            Ok(message)
        }
    })
    .await
    .map_err(|error| format!("ADB stop-server task failed: {error}"))?
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
static LOG_TAILER_HANDLE: OnceLock<StdMutex<Option<tauri::async_runtime::JoinHandle<()>>>> =
    OnceLock::new();

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn log_tailer_handle() -> &'static StdMutex<Option<tauri::async_runtime::JoinHandle<()>>> {
    LOG_TAILER_HANDLE.get_or_init(|| StdMutex::new(None))
}

/// Start tailing the Android camera-server log file into the Tauri log stream.
#[cfg(any(target_os = "windows", target_os = "linux"))]
#[command]
pub async fn tauri_adb_start_log_tailer(serial: String) -> Result<(), String> {
    let serial = ensure_non_empty(&serial, "ADB serial")?;
    let mut handle_guard = log_tailer_handle()
        .lock()
        .map_err(|_| "Log tailer handle lock was poisoned.".to_string())?;

    if handle_guard.is_some() {
        return Ok(());
    }

    let handle = tauri::async_runtime::spawn(async move {
        if let Err(error) = run_log_tailer(&serial).await {
            log::warn!("[ScannerLogTailer] Exited: {error}");
        }
        if let Ok(mut handle_guard) = log_tailer_handle().lock() {
            handle_guard.take();
        }
    });

    *handle_guard = Some(handle);
    Ok(())
}

/// Stop the running Android camera-server log tailer.
#[cfg(any(target_os = "windows", target_os = "linux"))]
#[command]
pub async fn tauri_adb_stop_log_tailer() -> Result<(), String> {
    let handle = log_tailer_handle()
        .lock()
        .map_err(|_| "Log tailer handle lock was poisoned.".to_string())?
        .take();

    if let Some(handle) = handle {
        handle.abort();
        let _ = tokio::time::timeout(Duration::from_secs(3), handle).await;
    }

    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
async fn run_log_tailer(serial: &str) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let executable = resolve_adb_executable();
    let mut command = tokio::process::Command::new(&executable);
    command
        .args(["-s", serial, "shell", "tail", "-f", SCANNER_SERVER_LOG_PATH])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);

    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to start scanner server log tailer: {error}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture scanner server log tailer stdout.".to_string())?;
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => log::info!("[AndroidCameraServer] {line}"),
            Ok(None) => break,
            Err(error) => {
                log::warn!("[ScannerLogTailer] Read error: {error}");
                break;
            }
        }
    }

    let _ = child.kill().await;
    Ok(())
}

/// Wait for the server readiness sentinel in the device log file.
#[cfg(any(target_os = "windows", target_os = "linux"))]
#[command]
pub async fn tauri_adb_await_server_ready(serial: String, timeout_ms: u64) -> Result<(), String> {
    let serial = ensure_non_empty(&serial, "ADB serial")?;
    let poll_interval = tokio::time::Duration::from_millis(200);
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "Server did not become ready within {timeout_ms}ms; sentinel `{}` was not found in {}.",
                SCANNER_SERVER_READY_SENTINEL,
                SCANNER_SERVER_LOG_PATH
            ));
        }

        let check_result = tauri::async_runtime::spawn_blocking({
            let serial = serial.clone();
            move || {
                let shell_command = format!(
                    "grep -Fq {} {}",
                    shell_single_quote(SCANNER_SERVER_READY_SENTINEL),
                    shell_single_quote(SCANNER_SERVER_LOG_PATH)
                );
                let args = vec!["-s".to_string(), serial, "shell".to_string(), shell_command];
                run_adb_command(&args)
            }
        })
        .await
        .map_err(|error| format!("Server ready check task failed: {error}"))?;

        if matches!(check_result, Ok(output) if output.status.success()) {
            log::info!(
                "[ScannerServerReady] Sentinel `{}` found in {}.",
                SCANNER_SERVER_READY_SENTINEL,
                SCANNER_SERVER_LOG_PATH
            );
            return Ok(());
        }

        tokio::time::sleep(poll_interval).await;
    }
}

// ---------------------------------------------------------------------------
// Screenshot command
// ---------------------------------------------------------------------------

fn send_raw_payload(
    channel: &Channel<InvokeResponseBody>,
    bytes: Vec<u8>,
    context: &str,
) -> Result<(), String> {
    channel
        .send(InvokeResponseBody::Raw(bytes))
        .map_err(|error| format!("Failed to deliver {context} to the frontend: {error}"))
}

/// Capture a device screenshot via `adb exec-out screencap -p`.
#[command]
pub async fn tauri_adb_screenshot(
    serial: String,
    payload_channel: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let png_bytes: Result<Vec<u8>, String> = tauri::async_runtime::spawn_blocking(move || {
        let serial = ensure_non_empty(&serial, "ADB serial")?;
        let args = vec![
            "-s".to_string(),
            serial.clone(),
            "exec-out".to_string(),
            "screencap".to_string(),
            "-p".to_string(),
        ];
        let output = run_adb_checked(&args, &format!("adb -s {serial} exec-out screencap -p"))?;
        Ok(output.stdout)
    })
    .await
    .map_err(|error| format!("ADB screenshot task failed: {error}"))?;

    send_raw_payload(&payload_channel, png_bytes?, "ADB screenshot")
}
