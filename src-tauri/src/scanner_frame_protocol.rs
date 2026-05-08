/// Shared frame packet protocol constants and parsing utilities.
///
/// This module is the single source of truth for the binary preview frame
/// packet format used across the decoder, detection, and benchmark subsystems.

/// Packet header layout: 1-byte codec + 4-byte width (BE) + 4-byte height (BE).
pub const FRAME_PACKET_HEADER_SIZE: usize = 9;

/// Optional telemetry block: 8-byte sent_at_epoch_ms (BE) + 4-byte sequence (BE).
pub const FRAME_PACKET_TELEMETRY_SIZE: usize = 12;

/// Tightly packed I420 frame without telemetry.
pub const FRAME_CODEC_I420: u8 = 3;

/// Tightly packed I420 frame with a telemetry block after the header.
pub const FRAME_CODEC_I420_TELEMETRY: u8 = 4;

/// Parse the header of a codec-tagged frame packet.
///
/// Returns `(width, height, payload_slice)` where `payload_slice` points at the
/// I420 plane data after the header (and optional telemetry) bytes.
pub fn parse_frame_packet_header(packet: &[u8]) -> Result<(u32, u32, &[u8]), String> {
    if packet.len() < FRAME_PACKET_HEADER_SIZE {
        return Err("Frame packet is shorter than the protocol header.".to_string());
    }

    let codec = packet[0];
    if codec != FRAME_CODEC_I420 && codec != FRAME_CODEC_I420_TELEMETRY {
        return Err(format!(
            "Expected an I420 frame packet, got codec {codec}."
        ));
    }

    let width = u32::from_be_bytes([packet[1], packet[2], packet[3], packet[4]]);
    let height = u32::from_be_bytes([packet[5], packet[6], packet[7], packet[8]]);
    let payload_offset = if codec == FRAME_CODEC_I420_TELEMETRY {
        FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE
    } else {
        FRAME_PACKET_HEADER_SIZE
    };

    if packet.len() < payload_offset {
        return Err("Frame packet telemetry header is truncated.".to_string());
    }

    Ok((width, height, &packet[payload_offset..]))
}

/// Decode a tightly packed I420 payload into an `RgbImage`.
///
/// The payload must contain exactly `width * height * 3 / 2` bytes
/// (luma + two quarter-size chroma planes).  Dimensions must be even.
pub fn decode_i420_payload_to_rgb_image(
    payload: &[u8],
    width: u32,
    height: u32,
) -> Result<image::RgbImage, String> {
    if width == 0 || height == 0 {
        return Err("Preview frame dimensions must be greater than zero.".to_string());
    }

    if width % 2 != 0 || height % 2 != 0 {
        return Err(format!(
            "I420 preview frames require even dimensions, got {}x{}.",
            width, height
        ));
    }

    let width_usize = width as usize;
    let height_usize = height as usize;

    const MAX_FRAME_DIMENSION: u32 = 4096;
    if width > MAX_FRAME_DIMENSION || height > MAX_FRAME_DIMENSION {
        return Err(format!(
            "Frame dimensions {}x{} exceed maximum allowed {}.",
            width, height, MAX_FRAME_DIMENSION,
        ));
    }

    let luma_len = width_usize.checked_mul(height_usize)
        .ok_or_else(|| format!("I420 dimension overflow: {}x{}", width, height))?;
    let chroma_width = width_usize / 2;
    let chroma_height = height_usize / 2;
    let chroma_len = chroma_width.checked_mul(chroma_height)
        .ok_or_else(|| format!("I420 chroma dimension overflow: {}x{}", chroma_width, chroma_height))?;
    let expected_len = luma_len.checked_add(2 * chroma_len)
        .ok_or_else(|| "I420 total size overflow".to_string())?;

    if payload.len() != expected_len {
        return Err(format!(
            "Invalid I420 preview payload size: expected {expected_len}, got {}.",
            payload.len()
        ));
    }

    let y_plane = &payload[..luma_len];
    let u_plane = &payload[luma_len..luma_len + chroma_len];
    let v_plane = &payload[luma_len + chroma_len..];
    // Allocate the output buffer without zero-filling — every pixel is
    // unconditionally written by the 2×2 block loop below, so the
    // zero-fill from `RgbImage::new()` (~691 KB for 640×360) is pure waste.
    let pixel_count = width_usize.checked_mul(height_usize)
        .and_then(|v| v.checked_mul(3))
        .ok_or_else(|| format!("RGB buffer size overflow: {}x{}x3", width, height))?;
    let mut raw_buf = Vec::with_capacity(pixel_count);
    // SAFETY: the loop below writes exactly width×height×3 bytes (all
    // pixels in 2×2 blocks covering the full even-dimensioned image).
    // All dimensions are validated above with checked arithmetic and MAX cap.
    unsafe { raw_buf.set_len(pixel_count); }
    let output = &mut raw_buf[..];

    // Process 2×2 luma blocks sharing one chroma pair.
    // This halves chroma lookups vs. per-pixel and amortises the UV→RGB
    // contribution computation across four pixels.
    for chroma_row in 0..chroma_height {
        let row0 = chroma_row * 2;
        let row1 = row0 + 1;
        let y_row0 = row0 * width_usize;
        let y_row1 = row1 * width_usize;
        let uv_row = chroma_row * chroma_width;

        for chroma_col in 0..chroma_width {
            let col0 = chroma_col * 2;
            let col1 = col0 + 1;
            let u = u_plane[uv_row + chroma_col] as i32;
            let v = v_plane[uv_row + chroma_col] as i32;
            let d = u - 128;
            let e = v - 128;
            let r_add = 409 * e + 128;
            let g_add = -100 * d - 208 * e + 128;
            let b_add = 516 * d + 128;

            // Pixel (row0, col0)
            let c0 = (y_plane[y_row0 + col0] as i32 - 16).max(0);
            let base0 = 298 * c0;
            let off0 = (y_row0 + col0) * 3;
            output[off0]     = ((base0 + r_add) >> 8).clamp(0, 255) as u8;
            output[off0 + 1] = ((base0 + g_add) >> 8).clamp(0, 255) as u8;
            output[off0 + 2] = ((base0 + b_add) >> 8).clamp(0, 255) as u8;

            // Pixel (row0, col1)
            let c1 = (y_plane[y_row0 + col1] as i32 - 16).max(0);
            let base1 = 298 * c1;
            let off1 = (y_row0 + col1) * 3;
            output[off1]     = ((base1 + r_add) >> 8).clamp(0, 255) as u8;
            output[off1 + 1] = ((base1 + g_add) >> 8).clamp(0, 255) as u8;
            output[off1 + 2] = ((base1 + b_add) >> 8).clamp(0, 255) as u8;

            // Pixel (row1, col0)
            let c2 = (y_plane[y_row1 + col0] as i32 - 16).max(0);
            let base2 = 298 * c2;
            let off2 = (y_row1 + col0) * 3;
            output[off2]     = ((base2 + r_add) >> 8).clamp(0, 255) as u8;
            output[off2 + 1] = ((base2 + g_add) >> 8).clamp(0, 255) as u8;
            output[off2 + 2] = ((base2 + b_add) >> 8).clamp(0, 255) as u8;

            // Pixel (row1, col1)
            let c3 = (y_plane[y_row1 + col1] as i32 - 16).max(0);
            let base3 = 298 * c3;
            let off3 = (y_row1 + col1) * 3;
            output[off3]     = ((base3 + r_add) >> 8).clamp(0, 255) as u8;
            output[off3 + 1] = ((base3 + g_add) >> 8).clamp(0, 255) as u8;
            output[off3 + 2] = ((base3 + b_add) >> 8).clamp(0, 255) as u8;
        }
    }

    image::RgbImage::from_raw(width, height, raw_buf)
        .ok_or_else(|| "Failed to construct RgbImage from decoded I420 buffer.".to_string())
}

/// Build a `DynamicImage` from a complete preview frame packet (header + I420 payload).
pub fn build_dynamic_image_from_preview_frame_packet(packet: &[u8]) -> Result<image::DynamicImage, String> {
    let (width, height, payload) = parse_frame_packet_header(packet)?;
    let rgb = decode_i420_payload_to_rgb_image(payload, width, height)?;
    Ok(image::DynamicImage::ImageRgb8(rgb))
}

// ---------------------------------------------------------------------------
// I420 preview packing utilities
// ---------------------------------------------------------------------------

/// Clamp a dimension to a valid even I420 size (minimum 2).
pub fn clamp_even_dimension(value: usize) -> usize {
    if value <= 2 {
        return 2;
    }
    value & !1
}

/// Pick a preview size that fits within `max_w × max_h` while preserving
/// aspect ratio and I420 even-dimension alignment.
///
/// Returns `(preview_width, preview_height, downsample_factor)`.
pub fn select_preview_dimensions(
    width: usize,
    height: usize,
    max_w: usize,
    max_h: usize,
) -> (usize, usize, usize) {
    let max_w = max_w.max(2);
    let max_h = max_h.max(2);
    let mut factor = ((width + max_w - 1) / max_w)
        .max((height + max_h - 1) / max_h)
        .max(1);
    let mut preview_width = clamp_even_dimension(width / factor);
    let mut preview_height = clamp_even_dimension(height / factor);

    while preview_width > max_w || preview_height > max_h {
        factor += 1;
        preview_width = clamp_even_dimension(width / factor);
        preview_height = clamp_even_dimension(height / factor);
    }

    (preview_width, preview_height, factor.max(1))
}

/// Compute the payload length for a tightly packed I420 frame.
pub fn compute_i420_payload_len(width: usize, height: usize) -> usize {
    let chroma_w = width / 2;
    let chroma_h = height / 2;
    width * height + 2 * (chroma_w * chroma_h)
}

/// Append a strided image plane to a tightly packed destination buffer.
pub fn append_plane_contiguous(
    destination: &mut Vec<u8>,
    plane: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) {
    if stride == width {
        destination.extend_from_slice(&plane[..width * height]);
        return;
    }
    for row in 0..height {
        let row_start = row * stride;
        destination.extend_from_slice(&plane[row_start..row_start + width]);
    }
}

/// Append a downscaled image plane by sampling every `factor`th pixel.
///
/// Uses a caller-provided row buffer to batch `extend_from_slice` per row
/// instead of `push` per pixel, reducing capacity-check overhead from
/// O(width×height) to O(height) calls.  The same buffer is reused across
/// Y/U/V plane calls to avoid repeated allocations.
pub fn append_downsampled_plane_by_factor(
    destination: &mut Vec<u8>,
    plane: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    factor: usize,
    row_buf: &mut Vec<u8>,
) {
    if factor <= 1 {
        append_plane_contiguous(destination, plane, width, height, stride);
        return;
    }
    row_buf.resize(width, 0);
    for row in 0..height {
        let row_start = row * factor * stride;
        let source_row = &plane[row_start..row_start + (width * factor)];
        for (dst, &src) in row_buf.iter_mut().zip(source_row.iter().step_by(factor)) {
            *dst = src;
        }
        destination.extend_from_slice(&row_buf[..width]);
    }
}

/// Write telemetry data (timestamp + sequence) into a frame packet's
/// telemetry slot (bytes 9..21).
pub fn write_frame_telemetry(
    packet: &mut [u8],
    sent_at_epoch_ms: u64,
    sequence: u32,
) -> Result<(), String> {
    let telemetry_end = FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE;
    if packet.len() < telemetry_end {
        return Err(format!(
            "Preview frame packet is too short to store telemetry: {} bytes.",
            packet.len()
        ));
    }
    packet[FRAME_PACKET_HEADER_SIZE..FRAME_PACKET_HEADER_SIZE + 8]
        .copy_from_slice(&sent_at_epoch_ms.to_be_bytes());
    packet[FRAME_PACKET_HEADER_SIZE + 8..telemetry_end]
        .copy_from_slice(&sequence.to_be_bytes());
    Ok(())
}
