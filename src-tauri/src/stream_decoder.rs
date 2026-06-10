/// Native H.264 stream decoder for the ADB camera scanner.
///
/// Connects to a forwarded TCP port, reads length-prefixed H.264 NAL units
/// from the Android Camera Server, decodes them with `openh264`, extracts a
/// downscaled I420 preview frame, and pushes the newest frame packet to the
/// frontend over a Tauri IPC channel.
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use serde::Serialize;
use tauri::{
    command,
    ipc::{Channel, InvokeResponseBody},
};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::task::spawn_blocking;
use tokio::time::{sleep, timeout};

use crate::scanner_frame_protocol::{
    self, FRAME_CODEC_I420_TELEMETRY, FRAME_PACKET_HEADER_SIZE, FRAME_PACKET_TELEMETRY_SIZE,
};

/// Shared flag to signal the decode loop to stop.
static STREAMING: AtomicBool = AtomicBool::new(false);
/// Monotonic generation used to invalidate an older decode loop before its task exits.
static STREAM_SESSION_ID: AtomicU64 = AtomicU64::new(0);

/// Frame counter for periodic perf logging.
static FRAME_SEQ: AtomicU64 = AtomicU64::new(0);
/// The most recent preview frame packet, retained for Rust-side live preview consumers.
static LATEST_PREVIEW_FRAME_PACKET: OnceLock<Mutex<Option<Arc<Vec<u8>>>>> = OnceLock::new();

/// Dynamic preview size limits, set from frontend settings at stream start.
static MAX_PREVIEW_W: AtomicUsize = AtomicUsize::new(DEFAULT_MAX_PREVIEW_WIDTH);
static MAX_PREVIEW_H: AtomicUsize = AtomicUsize::new(DEFAULT_MAX_PREVIEW_HEIGHT);

/// Emit the aggregate throughput log every N seconds.
const OVERALL_LOG_INTERVAL_SECS: u64 = 5;
/// Startup transport budget for TCP connect + one-byte server handshake.
const STARTUP_CONNECT_TIMEOUT_MS: u64 = 8_000;
/// Retry a dropped preview socket before surfacing a fatal stop.
const STREAM_RECONNECT_MAX_ATTEMPTS: usize = 12;
/// Base reconnect delay.
const RECONNECT_RETRY_BASE_DELAY_MS: u64 = 100;
/// Upper bound for reconnect backoff.
const RECONNECT_RETRY_MAX_DELAY_MS: u64 = 800;

/// Default preview size cap; overridden by frontend settings at stream start.
const DEFAULT_MAX_PREVIEW_WIDTH: usize = 640;
const DEFAULT_MAX_PREVIEW_HEIGHT: usize = 360;

/// Decoded preview frame plus timing metadata.
struct PreviewFrame {
    packet: Vec<u8>,
    payload_len: usize,
    width: u32,
    height: u32,
    decode_ms: f64,
    preview_pack_ms: f64,
}

fn latest_preview_frame_packet_state() -> &'static Mutex<Option<Arc<Vec<u8>>>> {
    LATEST_PREVIEW_FRAME_PACKET.get_or_init(|| Mutex::new(None))
}

pub(crate) fn replace_latest_preview_frame_packet(packet: Option<Arc<Vec<u8>>>) {
    let mut state = latest_preview_frame_packet_state()
        .lock()
        .expect("latest preview frame packet mutex should not be poisoned");
    *state = packet;
}

pub(crate) fn get_latest_preview_frame_packet() -> Option<Arc<Vec<u8>>> {
    latest_preview_frame_packet_state()
        .lock()
        .expect("latest preview frame packet mutex should not be poisoned")
        .clone()
}

/// Structured decoder lifecycle event for diagnostics and future UI hooks.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DecoderLifecycleEvent {
    state: String,
    detail: String,
    recoverable: bool,
    reconnect_attempt: usize,
}

/// Final exit classification for the decode loop.
#[derive(Clone, Debug, PartialEq, Eq)]
enum DecodeLoopExit {
    ManualStop,
    RemoteClosed { detail: String },
}

/// Start receiving and decoding the H.264 video stream from the forwarded port.
#[command]
pub async fn tauri_scanner_start_stream(
    port: u16,
    frame_channel: Channel<InvokeResponseBody>,
    status_channel: Channel<DecoderLifecycleEvent>,
    max_preview_width: Option<u32>,
    max_preview_height: Option<u32>,
) -> Result<(), String> {
    if STREAMING.swap(true, Ordering::SeqCst) {
        return Err("Stream decoder is already running.".to_string());
    }
    let session_id = STREAM_SESSION_ID.fetch_add(1, Ordering::SeqCst) + 1;

    FRAME_SEQ.store(0, Ordering::Relaxed);
    replace_latest_preview_frame_packet(None);
    MAX_PREVIEW_W.store(
        max_preview_width.unwrap_or(DEFAULT_MAX_PREVIEW_WIDTH as u32) as usize,
        Ordering::Relaxed,
    );
    MAX_PREVIEW_H.store(
        max_preview_height.unwrap_or(DEFAULT_MAX_PREVIEW_HEIGHT as u32) as usize,
        Ordering::Relaxed,
    );
    send_decoder_status(
        &status_channel,
        "starting",
        format!("Starting decoder for tcp://127.0.0.1:{port}."),
        true,
        0,
    );

    tauri::async_runtime::spawn(async move {
        let result =
            decode_stream_loop(port, frame_channel, status_channel.clone(), session_id).await;
        let is_current_session = STREAM_SESSION_ID.load(Ordering::SeqCst) == session_id;

        if is_current_session {
            match &result {
                Ok(DecodeLoopExit::ManualStop) => {
                    send_decoder_status(
                        &status_channel,
                        "stopped",
                        "Stream decoder stopped by request.".to_string(),
                        false,
                        0,
                    );
                }
                Ok(DecodeLoopExit::RemoteClosed { detail }) => {
                    send_decoder_status(&status_channel, "stopped", detail.clone(), false, 0);
                }
                Err(error) => {
                    log::error!("Stream decoder error: {error}");
                    send_decoder_status(&status_channel, "error", error.clone(), false, 0);
                }
            }
        }

        if is_current_session {
            STREAMING.store(false, Ordering::SeqCst);
            replace_latest_preview_frame_packet(None);
        }
    });

    Ok(())
}

/// Stop the currently running stream decoder.
#[command]
pub async fn tauri_scanner_stop_stream() -> Result<(), String> {
    if !STREAMING.swap(false, Ordering::SeqCst) {
        return Err("No stream decoder is currently running.".to_string());
    }
    STREAM_SESSION_ID.fetch_add(1, Ordering::SeqCst);
    replace_latest_preview_frame_packet(None);
    Ok(())
}

/// Internal decode loop that reads NAL units from TCP and decodes them.

/// Format context-specific messages for a recoverable stream reconnect.
fn format_reconnect_messages(
    error: &std::io::Error,
    has_received_frame: bool,
    phase: &str,
) -> (String, String) {
    let detail = if has_received_frame {
        format!("Preview {phase} interrupted ({error}). Reconnecting decoder transport.")
    } else {
        format!(
            "Preview stream {phase} closed before the first frame ({error}). Waiting for the scanner socket to become ready."
        )
    };
    let exhaust_msg = if has_received_frame {
        format!(
            "Preview {phase} kept failing after {STREAM_RECONNECT_MAX_ATTEMPTS} reconnect attempts: {error}"
        )
    } else {
        format!(
            "Preview stream {phase} closed before the first frame after {STREAM_RECONNECT_MAX_ATTEMPTS} reconnect attempts: {error}"
        )
    };
    (detail, exhaust_msg)
}

async fn decode_stream_loop(
    port: u16,
    frame_channel: Channel<InvokeResponseBody>,
    status_channel: Channel<DecoderLifecycleEvent>,
    session_id: u64,
) -> Result<DecodeLoopExit, String> {
    let address = format!("127.0.0.1:{port}");
    let mut reconnect_attempt = 0usize;
    let (mut stream, mut decoder) = connect_decoder_stream(
        &address,
        ConnectRetry::Timeout(Duration::from_millis(STARTUP_CONNECT_TIMEOUT_MS)),
        reconnect_attempt,
        "Waiting for preview stream to become available.",
        &status_channel,
        session_id,
    )
    .await?;
    let mut has_received_frame = false;

    let mut length_buf = [0u8; 4];
    let mut nal_buf: Vec<u8> = Vec::with_capacity(256 * 1024);
    let loop_start = Instant::now();
    let mut last_overall_log_sec = 0;

    loop {
        if !is_stream_session_current(session_id) {
            return Ok(DecodeLoopExit::ManualStop);
        }

        let iter_start = Instant::now();

        if let Err(error) = stream.read_exact(&mut length_buf).await {
            if !is_stream_session_current(session_id) {
                return Ok(DecodeLoopExit::ManualStop);
            }

            if is_recoverable_stream_error(&error) {
                reconnect_attempt += 1;
                let (detail, exhaust_msg) =
                    format_reconnect_messages(&error, has_received_frame, "socket read");
                if reconnect_attempt > STREAM_RECONNECT_MAX_ATTEMPTS {
                    return Err(exhaust_msg);
                }

                log::warn!("{detail}");
                send_decoder_status(
                    &status_channel,
                    "reconnecting",
                    detail,
                    true,
                    reconnect_attempt,
                );
                sleep(Duration::from_millis(reconnect_delay_ms(reconnect_attempt))).await;
                let (next_stream, next_decoder) = connect_decoder_stream(
                    &address,
                    ConnectRetry::Attempts(STREAM_RECONNECT_MAX_ATTEMPTS),
                    reconnect_attempt,
                    "Reconnecting preview stream after socket interruption.",
                    &status_channel,
                    session_id,
                )
                .await?;
                stream = next_stream;
                decoder = next_decoder;
                continue;
            }

            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                return Ok(DecodeLoopExit::RemoteClosed {
                    detail: "Preview stream ended after the upstream socket closed.".to_string(),
                });
            }
            return Err(format!("Failed to read NAL length: {error}"));
        }

        let nal_length = u32::from_be_bytes(length_buf) as usize;

        if nal_length == 0 || nal_length > 10 * 1024 * 1024 {
            continue;
        }

        nal_buf.resize(nal_length, 0);
        if let Err(error) = stream.read_exact(&mut nal_buf[..nal_length]).await {
            if !is_stream_session_current(session_id) {
                return Ok(DecodeLoopExit::ManualStop);
            }

            if is_recoverable_stream_error(&error) {
                reconnect_attempt += 1;
                let (detail, exhaust_msg) =
                    format_reconnect_messages(&error, has_received_frame, "payload read");
                if reconnect_attempt > STREAM_RECONNECT_MAX_ATTEMPTS {
                    return Err(exhaust_msg);
                }

                log::warn!("{detail}");
                send_decoder_status(
                    &status_channel,
                    "reconnecting",
                    detail,
                    true,
                    reconnect_attempt,
                );
                sleep(Duration::from_millis(reconnect_delay_ms(reconnect_attempt))).await;
                let (next_stream, next_decoder) = connect_decoder_stream(
                    &address,
                    ConnectRetry::Attempts(STREAM_RECONNECT_MAX_ATTEMPTS),
                    reconnect_attempt,
                    "Reconnecting preview stream after payload interruption.",
                    &status_channel,
                    session_id,
                )
                .await?;
                stream = next_stream;
                decoder = next_decoder;
                continue;
            }

            return Err(format!(
                "Failed to read NAL data ({nal_length} bytes): {error}"
            ));
        }

        let tcp_read_ms = iter_start.elapsed().as_secs_f64() * 1000.0;
        // Copy NAL data for the blocking decode task.  The copy reuses the
        // nal_buf allocation across frames (only the copy is allocated fresh
        // if the blocking thread hasn't returned the previous one yet, which
        // is the steady-state since decode is CPU-bound).
        let nal_snapshot = nal_buf[..nal_length].to_vec();
        let decoder_clone = decoder.clone();
        let decode_result =
            spawn_blocking(move || decode_nal_to_preview(decoder_clone, nal_snapshot))
                .await
                .map_err(|error| format!("Decode task panicked: {error}"))?;

        match decode_result {
            Ok(Some(frame)) => {
                let first_frame_ready = !has_received_frame;
                has_received_frame = true;
                reconnect_attempt = 0;
                let seq = FRAME_SEQ.fetch_add(1, Ordering::Relaxed);
                let payload_kb = frame.payload_len as f64 / 1024.0;
                let nal_kb = nal_length as f64 / 1024.0;
                let mut preview_packet = frame.packet;
                let sent_at_epoch_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|error| format!("System clock drifted before unix epoch: {error}"))?
                    .as_millis() as u64;
                scanner_frame_protocol::write_frame_telemetry(
                    &mut preview_packet,
                    sent_at_epoch_ms,
                    seq as u32,
                )?;
                // Wrap the packet in Arc so detection consumers get a cheap
                // ref-count bump instead of a full clone.  After storing the
                // Arc for detection, try_unwrap recovers the original Vec
                // for the IPC send without copying in the common case where
                // detection has already released its reference.
                let shared_packet = Arc::new(preview_packet);
                replace_latest_preview_frame_packet(Some(Arc::clone(&shared_packet)));

                if seq % 15 == 0 {
                    log::info!(
                        "[perf] frame#{seq} {}x{} | tcp_read={:.1}ms  h264_decode={:.1}ms  \
                         preview_pack={:.1}ms | NAL={:.1}KB  I420={:.1}KB",
                        frame.width,
                        frame.height,
                        tcp_read_ms,
                        frame.decode_ms,
                        frame.preview_pack_ms,
                        nal_kb,
                        payload_kb
                    );
                }

                let ipc_packet =
                    Arc::try_unwrap(shared_packet).unwrap_or_else(|arc| (*arc).clone());
                frame_channel
                    .send(InvokeResponseBody::Raw(ipc_packet))
                    .map_err(|error| {
                        format!("Failed to deliver the preview frame to the frontend: {error}")
                    })?;

                if first_frame_ready {
                    send_decoder_status(
                        &status_channel,
                        "ready",
                        format!("Decoder published the first preview frame from tcp://{address}."),
                        false,
                        0,
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                return Err(error);
            }
        }

        let elapsed = loop_start.elapsed().as_secs();
        if elapsed > 0 && elapsed >= last_overall_log_sec + OVERALL_LOG_INTERVAL_SECS {
            last_overall_log_sec = elapsed;
            let total = FRAME_SEQ.load(Ordering::Relaxed);
            let fps = total as f64 / loop_start.elapsed().as_secs_f64();
            log::info!("[perf] overall: {total} frames in {elapsed}s = {fps:.1} fps");
        }
    }
}

/// Decode a single H.264 NAL unit to a downscaled contiguous I420 preview frame.
fn decode_nal_to_preview(
    decoder: Arc<Mutex<Decoder>>,
    nal_data: Vec<u8>,
) -> Result<Option<PreviewFrame>, String> {
    // Ensure Annex-B start code prefix for openh264.
    // With KEY_PREPEND_SPS_PPS_TO_IDR_FRAMES=1, the vast majority of NALs
    // already carry the start code and take the zero-alloc fast path.
    let data = if nal_data.starts_with(&[0, 0, 0, 1]) || nal_data.starts_with(&[0, 0, 1]) {
        nal_data
    } else {
        // Rare path: prepend start code.  Reuse the original allocation
        // when capacity permits to avoid a fresh heap allocation.
        let mut prefixed = Vec::with_capacity(4 + nal_data.len());
        prefixed.extend_from_slice(&[0, 0, 0, 1]);
        prefixed.extend_from_slice(&nal_data);
        prefixed
    };

    let mut decoder_guard = decoder
        .lock()
        .map_err(|error| format!("Decoder mutex poisoned: {error}"))?;

    let decode_start = Instant::now();
    match decoder_guard.decode(&data) {
        Ok(Some(decoded_yuv)) => {
            let decode_ms = decode_start.elapsed().as_secs_f64() * 1000.0;
            let (source_width, source_height) = decoded_yuv.dimensions();
            let (y_stride, u_stride, v_stride) = decoded_yuv.strides();
            let (preview_width, preview_height, factor) =
                select_preview_dimensions(source_width, source_height);

            let pack_start = Instant::now();
            let payload_len =
                scanner_frame_protocol::compute_i420_payload_len(preview_width, preview_height);
            let packet = pack_i420_preview_packet(
                decoded_yuv.y(),
                decoded_yuv.u(),
                decoded_yuv.v(),
                y_stride,
                u_stride,
                v_stride,
                preview_width,
                preview_height,
                factor,
            );
            let preview_pack_ms = pack_start.elapsed().as_secs_f64() * 1000.0;

            Ok(Some(PreviewFrame {
                packet,
                payload_len,
                width: preview_width as u32,
                height: preview_height as u32,
                decode_ms,
                preview_pack_ms,
            }))
        }
        Ok(None) => Ok(None),
        Err(error) => {
            log::warn!("H.264 decode error (non-fatal): {error}");
            Ok(None)
        }
    }
}

/// Pick a preview size that limits IPC cost while keeping aspect ratio and I420 alignment.
fn select_preview_dimensions(width: usize, height: usize) -> (usize, usize, usize) {
    let max_w = MAX_PREVIEW_W.load(Ordering::Relaxed);
    let max_h = MAX_PREVIEW_H.load(Ordering::Relaxed);
    scanner_frame_protocol::select_preview_dimensions(width, height, max_w, max_h)
}

/// Pack a preview I420 frame into a binary frame packet.
#[allow(clippy::too_many_arguments)]
fn pack_i420_preview_packet(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    y_stride: usize,
    u_stride: usize,
    v_stride: usize,
    preview_width: usize,
    preview_height: usize,
    factor: usize,
) -> Vec<u8> {
    debug_assert_eq!(preview_width % 2, 0);
    debug_assert_eq!(preview_height % 2, 0);

    let expected_payload_len =
        scanner_frame_protocol::compute_i420_payload_len(preview_width, preview_height);
    let preview_chroma_width = preview_width / 2;
    let preview_chroma_height = preview_height / 2;
    let mut packet = Vec::with_capacity(
        FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE + expected_payload_len,
    );
    packet.push(FRAME_CODEC_I420_TELEMETRY);
    packet.extend_from_slice(&(preview_width as u32).to_be_bytes());
    packet.extend_from_slice(&(preview_height as u32).to_be_bytes());
    packet.resize(FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE, 0);

    if factor == 1 {
        scanner_frame_protocol::append_plane_contiguous(
            &mut packet,
            y_plane,
            preview_width,
            preview_height,
            y_stride,
        );
        scanner_frame_protocol::append_plane_contiguous(
            &mut packet,
            u_plane,
            preview_chroma_width,
            preview_chroma_height,
            u_stride,
        );
        scanner_frame_protocol::append_plane_contiguous(
            &mut packet,
            v_plane,
            preview_chroma_width,
            preview_chroma_height,
            v_stride,
        );
        debug_assert_eq!(
            packet.len(),
            FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE + expected_payload_len
        );
        return packet;
    }

    let mut row_buf = vec![0u8; preview_width];
    scanner_frame_protocol::append_downsampled_plane_by_factor(
        &mut packet,
        y_plane,
        preview_width,
        preview_height,
        y_stride,
        factor,
        &mut row_buf,
    );
    scanner_frame_protocol::append_downsampled_plane_by_factor(
        &mut packet,
        u_plane,
        preview_chroma_width,
        preview_chroma_height,
        u_stride,
        factor.max(1),
        &mut row_buf,
    );
    scanner_frame_protocol::append_downsampled_plane_by_factor(
        &mut packet,
        v_plane,
        preview_chroma_width,
        preview_chroma_height,
        v_stride,
        factor.max(1),
        &mut row_buf,
    );

    debug_assert_eq!(
        packet.len(),
        FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE + expected_payload_len
    );
    packet
}

/// Copy a strided image plane into a tightly packed buffer.
/// Emit a structured decoder lifecycle event over the scanner status channel.
fn send_decoder_status(
    channel: &Channel<DecoderLifecycleEvent>,
    state: &str,
    detail: String,
    recoverable: bool,
    reconnect_attempt: usize,
) {
    let _ = channel.send(DecoderLifecycleEvent {
        state: state.to_string(),
        detail,
        recoverable,
        reconnect_attempt,
    });
}

/// Build a fresh H.264 decoder instance for a new preview stream session.
fn create_decoder() -> Result<Arc<Mutex<Decoder>>, String> {
    Ok(Arc::new(Mutex::new(Decoder::new().map_err(|error| {
        format!("Failed to create H.264 decoder: {error}")
    })?)))
}

/// Determine whether a TCP stream error is transient enough to warrant reconnecting.
fn is_recoverable_stream_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::Interrupted
    )
}

/// Compute a bounded reconnect delay in milliseconds.
fn reconnect_delay_ms(attempt: usize) -> u64 {
    let backoff = RECONNECT_RETRY_BASE_DELAY_MS.saturating_mul(attempt.max(1) as u64);
    backoff.min(RECONNECT_RETRY_MAX_DELAY_MS)
}

fn is_stream_session_current(session_id: u64) -> bool {
    STREAMING.load(Ordering::SeqCst) && STREAM_SESSION_ID.load(Ordering::SeqCst) == session_id
}

#[derive(Clone, Copy)]
enum ConnectRetry {
    Timeout(Duration),
    Attempts(usize),
}

impl ConnectRetry {
    fn remaining(self, started_at: Instant) -> Option<Duration> {
        match self {
            ConnectRetry::Timeout(timeout_budget) => {
                timeout_budget.checked_sub(started_at.elapsed())
            }
            ConnectRetry::Attempts(_) => None,
        }
    }

    fn exhausted(self, attempts: usize, started_at: Instant) -> bool {
        match self {
            ConnectRetry::Timeout(timeout_budget) => started_at.elapsed() >= timeout_budget,
            ConnectRetry::Attempts(max_attempts) => attempts >= max_attempts,
        }
    }

    fn progress(self, attempts: usize, started_at: Instant) -> String {
        match self {
            ConnectRetry::Timeout(timeout_budget) => format!(
                "attempt {attempts}, elapsed {}ms/{}ms",
                started_at.elapsed().as_millis(),
                timeout_budget.as_millis()
            ),
            ConnectRetry::Attempts(max_attempts) => {
                format!("attempt {attempts}/{max_attempts}")
            }
        }
    }

    fn failure_context(self, attempts: usize, started_at: Instant) -> String {
        match self {
            ConnectRetry::Timeout(timeout_budget) => format!(
                "after {attempts} attempt(s) over {}ms/{}ms",
                started_at.elapsed().as_millis(),
                timeout_budget.as_millis()
            ),
            ConnectRetry::Attempts(max_attempts) => {
                format!("after {attempts}/{max_attempts} attempts")
            }
        }
    }
}

/// Connect to the local forwarded preview socket and read the one-byte server handshake.
async fn connect_decoder_stream(
    address: &str,
    retry: ConnectRetry,
    reconnect_attempt: usize,
    detail_prefix: &str,
    status_channel: &Channel<DecoderLifecycleEvent>,
    session_id: u64,
) -> Result<(BufReader<TcpStream>, Arc<Mutex<Decoder>>), String> {
    let mut attempts = 0usize;
    let started_at = Instant::now();

    loop {
        if !is_stream_session_current(session_id) {
            return Err("Stream decoder stopped before TCP connect completed.".to_string());
        }

        let connect_result = match retry.remaining(started_at) {
            Some(remaining) if remaining.is_zero() => {
                return Err(format!(
                    "{detail_prefix} Timed out {} while connecting to {address}.",
                    retry.failure_context(attempts, started_at)
                ));
            }
            Some(remaining) => match timeout(remaining, TcpStream::connect(address)).await {
                Ok(result) => result,
                Err(_) => {
                    attempts += 1;
                    return Err(format!(
                        "{detail_prefix} Timed out {} while connecting to {address}.",
                        retry.failure_context(attempts, started_at)
                    ));
                }
            },
            None => TcpStream::connect(address).await,
        };

        match connect_result {
            Ok(mut stream) => {
                // Read and verify the server's 1-byte transport handshake (0x00).
                let mut handshake = [0u8; 1];
                let handshake_result = match retry.remaining(started_at) {
                    Some(remaining) if remaining.is_zero() => {
                        attempts += 1;
                        return Err(format!(
                            "{detail_prefix} Timed out {} while waiting for the server handshake.",
                            retry.failure_context(attempts, started_at)
                        ));
                    }
                    Some(remaining) => {
                        match timeout(remaining, stream.read_exact(&mut handshake)).await {
                            Ok(result) => result,
                            Err(_) => {
                                attempts += 1;
                                return Err(format!(
                                "{detail_prefix} Timed out {} while waiting for the server handshake.",
                                retry.failure_context(attempts, started_at)
                            ));
                            }
                        }
                    }
                    None => stream.read_exact(&mut handshake).await,
                };

                if let Err(e) = handshake_result {
                    attempts += 1;
                    if retry.exhausted(attempts, started_at) {
                        return Err(format!(
                            "{detail_prefix} Handshake failed {}: {e}",
                            retry.failure_context(attempts, started_at)
                        ));
                    }
                    let delay_ms = reconnect_delay_ms(attempts);
                    let detail = format!(
                        "{detail_prefix} Handshake {} failed: {e}. Retrying in {delay_ms}ms.",
                        retry.progress(attempts, started_at)
                    );
                    log::warn!("{detail}");
                    send_decoder_status(
                        status_channel,
                        "connecting",
                        detail,
                        true,
                        reconnect_attempt.max(attempts),
                    );
                    sleep(Duration::from_millis(delay_ms)).await;
                    continue;
                }

                if handshake[0] != 0x00 {
                    attempts += 1;
                    if retry.exhausted(attempts, started_at) {
                        return Err(format!(
                            "{detail_prefix} Invalid server handshake byte 0x{:02x} {}.",
                            handshake[0],
                            retry.failure_context(attempts, started_at)
                        ));
                    }
                    let delay_ms = reconnect_delay_ms(attempts);
                    let detail = format!(
                        "{detail_prefix} Invalid server handshake byte 0x{:02x} on {}. Retrying in {delay_ms}ms.",
                        handshake[0],
                        retry.progress(attempts, started_at)
                    );
                    log::warn!("{detail}");
                    send_decoder_status(
                        status_channel,
                        "connecting",
                        detail,
                        true,
                        reconnect_attempt.max(attempts),
                    );
                    sleep(Duration::from_millis(delay_ms)).await;
                    continue;
                }

                if reconnect_attempt > 0 || attempts > 0 {
                    send_decoder_status(
                        status_channel,
                        "connected",
                        format!(
                            "Decoder connected to {address} after {} attempt(s).",
                            reconnect_attempt.max(attempts)
                        ),
                        true,
                        reconnect_attempt.max(attempts),
                    );
                }
                return Ok((
                    BufReader::with_capacity(64 * 1024, stream),
                    create_decoder()?,
                ));
            }
            Err(error) => {
                attempts += 1;
                if retry.exhausted(attempts, started_at) {
                    return Err(format!(
                        "{detail_prefix} Failed to connect to stream at {address} {}: {error}",
                        retry.failure_context(attempts, started_at)
                    ));
                }

                let delay_ms = reconnect_delay_ms(attempts);
                let detail = format!(
                    "{detail_prefix} Connect {} failed: {error}. Retrying in {delay_ms}ms.",
                    retry.progress(attempts, started_at)
                );
                log::warn!("{detail}");
                send_decoder_status(
                    status_channel,
                    "connecting",
                    detail,
                    true,
                    reconnect_attempt.max(attempts),
                );
                sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }
}
