/// Scanner-specific ADB transport commands.
///
/// These Tauri commands handle camera server lifecycle and still image
/// capture. They are separated from the generic ADB primitives in
/// `adb_plugin.rs` for clearer responsibility boundaries.
use std::{
    io::{BufReader, Read},
    net::{SocketAddr, TcpStream as StdTcpStream},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tauri::{
    command,
    ipc::{Channel, InvokeResponseBody},
};

use crate::adb_plugin::{
    build_app_process_shell_command, combine_command_output, ensure_non_empty,
    normalize_text_output, run_adb_checked, run_adb_command, shell_single_quote,
    wrap_shell_c_script,
};

const DEVICE_TMP_DIR: &str = "/data/local/tmp";
const SCANNER_SERVER_PID_PATH: &str = "/data/local/tmp/skid-scanner-server.pid";
const SCANNER_SERVER_LOG_PATH: &str = "/data/local/tmp/skid-scanner-server.log";
const STILL_CAPTURE_MAIN_CLASS: &str = "com.skidhomework.server.StillCapture";
const STILL_STREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const STILL_STREAM_READ_TIMEOUT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// Helper functions (scanner-specific)
// ---------------------------------------------------------------------------

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
    let app_process_command = build_app_process_shell_command(classpath, main_class, server_args);
    let kill_command = build_scanner_server_kill_script(main_class);

    format!(
        "{kill_command}; \
rm -f {pidfile}; \
: >{logfile}; \
{} </dev/null >>{logfile} 2>&1 & echo $! > {pidfile}",
        app_process_command,
        kill_command = kill_command,
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
        kill_command = kill_command,
    )
}

fn build_still_capture_script(
    classpath: &str,
    socket_name: &str,
    output_path: Option<&str>,
) -> String {
    let mut capture_args = vec!["--socket".to_string(), socket_name.to_string()];
    if let Some(output_path) = output_path {
        capture_args.push("--output".to_string());
        capture_args.push(output_path.to_string());
    }
    let app_process_command =
        build_app_process_shell_command(classpath, STILL_CAPTURE_MAIN_CLASS, &capture_args);

    if let Some(output_path) = output_path {
        return format!(
            "mkdir -p {tmp_dir}; \
 rm -f {output_path}; \
 {app_process_command} >/dev/null",
            tmp_dir = shell_single_quote(DEVICE_TMP_DIR),
            output_path = shell_single_quote(output_path),
        );
    }

    app_process_command
}

fn build_remote_still_capture_path(serial: &str) -> String {
    let sanitized_serial = serial
        .chars()
        .map(|value| match value {
            'a'..='z' | 'A'..='Z' | '0'..='9' => value,
            _ => '_',
        })
        .collect::<String>();
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    format!("{DEVICE_TMP_DIR}/skid-scanner-still-{sanitized_serial}-{unique_suffix}.jpg")
}

fn describe_hex_window(bytes: &[u8], count: usize, from_end: bool) -> String {
    if bytes.is_empty() {
        return "∅".to_string();
    }

    let safe_count = count.max(1).min(bytes.len());
    let slice = if from_end {
        &bytes[bytes.len() - safe_count..]
    } else {
        &bytes[..safe_count]
    };

    slice
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect::<Vec<String>>()
        .join(" ")
}

fn find_marker_offset(
    bytes: &[u8],
    marker_high: u8,
    marker_low: u8,
    from_end: bool,
) -> Option<usize> {
    if bytes.len() < 2 {
        return None;
    }

    if from_end {
        for index in (0..=(bytes.len() - 2)).rev() {
            if bytes[index] == marker_high && bytes[index + 1] == marker_low {
                return Some(index);
            }
        }
        return None;
    }

    for index in 0..=(bytes.len() - 2) {
        if bytes[index] == marker_high && bytes[index + 1] == marker_low {
            return Some(index);
        }
    }

    None
}

fn describe_binary_payload(bytes: &[u8]) -> String {
    format!(
        "len={} head=[{}] tail=[{}] first_soi={:?} last_eoi={:?}",
        bytes.len(),
        describe_hex_window(bytes, 16, false),
        describe_hex_window(bytes, 16, true),
        find_marker_offset(bytes, 0xff, 0xd8, false),
        find_marker_offset(bytes, 0xff, 0xd9, true),
    )
}

fn build_remove_file_shell_script(path: &str) -> String {
    format!("rm -f {}", shell_single_quote(path))
}

fn validate_still_capture_payload(
    serial: &str,
    payload: Vec<u8>,
    failure_context: &str,
) -> Result<Vec<u8>, String> {
    if payload.is_empty() {
        return Err(format!("{failure_context} returned no data for {serial}."));
    }

    if payload.len() >= 2 && payload[0] == 0xff && payload[1] == 0xd8 {
        return Ok(payload);
    }

    let text_probe = normalize_text_output(&payload);
    if !text_probe.is_empty() {
        return Err(format!(
            "{failure_context} was not a JPEG for {serial}: {text_probe}"
        ));
    }

    Err(format!(
        "{failure_context} was not a JPEG for {serial}: {}",
        describe_binary_payload(&payload)
    ))
}

fn send_raw_payload(
    channel: &Channel<InvokeResponseBody>,
    bytes: Vec<u8>,
    context: &str,
) -> Result<(), String> {
    channel
        .send(InvokeResponseBody::Raw(bytes))
        .map_err(|error| format!("Failed to deliver {context} to the frontend: {error}"))
}

fn capture_still_via_device_file(
    serial: &str,
    classpath: &str,
    socket_name: &str,
) -> Result<Vec<u8>, String> {
    let remote_output_path = build_remote_still_capture_path(serial);
    let capture_script =
        build_still_capture_script(classpath, socket_name, Some(&remote_output_path));

    let capture_args = vec![
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
        "sh".to_string(),
        "-c".to_string(),
        wrap_shell_c_script(&capture_script),
    ];

    let capture_output = run_adb_checked(
        &capture_args,
        &format!("adb -s {serial} shell sh -c <capture still to file>"),
    )?;
    let capture_details = combine_command_output(&capture_output);
    if !capture_details.is_empty() {
        log::info!(
            "[Scanner][StillDiag] Device capture command output for {serial}: {capture_details}"
        );
    }
    log::info!(
        "[Scanner][StillDiag] Device still capture command completed for {serial}; remote path={remote_output_path}"
    );

    let fetch_result = (|| {
        let fetch_args = vec![
            "-s".to_string(),
            serial.to_string(),
            "exec-out".to_string(),
            "cat".to_string(),
            remote_output_path.clone(),
        ];
        let output = run_adb_checked(
            &fetch_args,
            &format!("adb -s {serial} exec-out cat {remote_output_path}"),
        )?;
        let payload = validate_still_capture_payload(
            serial,
            output.stdout,
            "Device-file still capture fetch",
        )?;

        log::info!(
            "[Scanner][StillDiag] Host fetched still payload for {serial}: {}",
            describe_binary_payload(&payload)
        );

        Ok(payload)
    })();

    let cleanup_script = build_remove_file_shell_script(&remote_output_path);
    let cleanup_args = vec![
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
        "sh".to_string(),
        "-c".to_string(),
        wrap_shell_c_script(&cleanup_script),
    ];
    let _ = run_adb_command(&cleanup_args);

    fetch_result
}

fn capture_still_via_forwarded_socket(port: u16) -> Result<Vec<u8>, String> {
    let address: SocketAddr = format!("127.0.0.1:{port}").parse().map_err(|error| {
        format!("Invalid still-stream forward address for port {port}: {error}")
    })?;
    let started_at = Instant::now();
    let stream =
        StdTcpStream::connect_timeout(&address, STILL_STREAM_CONNECT_TIMEOUT).map_err(|error| {
            format!("Failed to connect to forwarded still stream at {address}: {error}")
        })?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(STILL_STREAM_READ_TIMEOUT));

    let mut payload = Vec::with_capacity(2 * 1024 * 1024);
    let mut reader = BufReader::with_capacity(256 * 1024, stream);
    reader.read_to_end(&mut payload).map_err(|error| {
        format!("Failed to read forwarded still stream from {address}: {error}")
    })?;

    let address_label = format!("tcp://{address}");
    let payload =
        validate_still_capture_payload(&address_label, payload, "Forwarded still stream payload")?;
    if payload.len() < 2 || payload[payload.len() - 2] != 0xff || payload[payload.len() - 1] != 0xd9
    {
        return Err(format!(
            "Forwarded still stream payload was truncated for {address_label}: {}",
            describe_binary_payload(&payload)
        ));
    }
    log::info!(
        "[Scanner][StillDiag] Forwarded still payload from {address}: {}",
        describe_binary_payload(&payload)
    );
    log::info!(
        "[Scanner][StillPerf] Forwarded still stream completed from {address} in {:.1}ms.",
        started_at.elapsed().as_secs_f64() * 1000.0,
    );
    Ok(payload)
}

// ---------------------------------------------------------------------------
// Tauri Commands (scanner-specific transport)
// ---------------------------------------------------------------------------

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

/// Capture a full-resolution still image via device-file transfer.
#[command]
pub async fn tauri_adb_capture_still(
    serial: String,
    classpath: String,
    socket_name: String,
    payload_channel: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let jpeg_bytes: Result<Vec<u8>, String> = tauri::async_runtime::spawn_blocking(move || {
        let serial = ensure_non_empty(&serial, "ADB serial")?;
        let classpath = ensure_non_empty(&classpath, "Server classpath")?;
        let socket_name = ensure_non_empty(&socket_name, "Still capture socket name")?;
        let start = Instant::now();

        let payload = capture_still_via_device_file(&serial, &classpath, &socket_name)?;
        log::info!(
            "[Scanner][StillPerf] Still capture via device-file completed for {serial} in {:.1}ms.",
            start.elapsed().as_secs_f64() * 1000.0,
        );
        Ok(payload)
    })
    .await
    .map_err(|error| format!("ADB still-capture task failed: {error}"))?;

    send_raw_payload(&payload_channel, jpeg_bytes?, "ADB still capture")
}

/// Capture a full-resolution still image over a persistent forwarded still-stream socket.
#[command]
pub async fn tauri_adb_capture_still_stream(
    port: u16,
    payload_channel: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let jpeg_bytes: Result<Vec<u8>, String> =
        tauri::async_runtime::spawn_blocking(move || capture_still_via_forwarded_socket(port))
            .await
            .map_err(|error| format!("ADB forwarded still-stream task failed: {error}"))?;

    send_raw_payload(
        &payload_channel,
        jpeg_bytes?,
        "ADB forwarded still-stream capture",
    )
}

/// Start the Android Camera Server on the device.
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
            &format!("adb -s {serial} shell sh -c <start scanner server>"),
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

/// Stop the Android Camera Server running on the device.
#[command]
pub async fn tauri_adb_stop_server(serial: String, classpath: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let serial = ensure_non_empty(&serial, "ADB serial")?;
        let _classpath = ensure_non_empty(&classpath, "Server classpath")?;
        let kill_command = build_scanner_server_stop_script("com.skidhomework.server.Server");
        let args = vec![
            "-s".to_string(),
            serial.clone(),
            "shell".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            wrap_shell_c_script(&kill_command),
        ];

        let output = run_adb_command(&args)?;
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
