use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageReader, RgbaImage};

use crate::scanner_detect::{ScannerDetectDocumentRequest, ScannerPoint};
use crate::scanner_frame_protocol;
use crate::stream_decoder::get_latest_preview_frame_packet;

const MAX_DETECT_IMAGE_BYTES: usize = 50 * 1024 * 1024;
const MAX_DETECT_IMAGE_PIXELS: u64 = 24_000_000; // 24 megapixels

#[derive(Debug, Clone)]
pub(crate) struct PreparedInferenceImage {
    pub(crate) original_width: u32,
    pub(crate) original_height: u32,
    pub(crate) working_image: DynamicImage,
}

impl PreparedInferenceImage {
    pub(crate) fn working_dimensions(&self) -> (u32, u32) {
        self.working_image.dimensions()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedDetectInput {
    pub(crate) prepared_image: Option<PreparedInferenceImage>,
    pub(crate) input_transport: &'static str,
}

pub(crate) fn decode_detect_image_with_limits(
    bytes: &[u8],
    context: &str,
) -> Result<DynamicImage, String> {
    if bytes.len() > MAX_DETECT_IMAGE_BYTES {
        return Err(format!(
            "{context}: input size {} bytes exceeds limit of {} bytes.",
            bytes.len(),
            MAX_DETECT_IMAGE_BYTES,
        ));
    }

    // Header-first check: read dimensions WITHOUT full decode to block compressed bombs.
    if let Ok(reader) = ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format() {
        if let Ok((w, h)) = reader.into_dimensions() {
            let header_pixels = u64::from(w) * u64::from(h);
            if header_pixels > MAX_DETECT_IMAGE_PIXELS {
                return Err(format!(
                    "{context}: header dimensions {w}x{h} ({header_pixels} pixels) exceed limit of {MAX_DETECT_IMAGE_PIXELS}.",
                ));
            }
        }
    }

    let decoded = image::load_from_memory(bytes)
        .map_err(|error| format!("Failed to decode {context}: {error}"))?;
    let pixel_count = u64::from(decoded.width()) * u64::from(decoded.height());

    if pixel_count > MAX_DETECT_IMAGE_PIXELS {
        return Err(format!(
            "{context}: decoded image {}x{} ({} pixels) exceeds limit of {}.",
            decoded.width(),
            decoded.height(),
            pixel_count,
            MAX_DETECT_IMAGE_PIXELS,
        ));
    }

    Ok(decoded)
}

/// Resolve the input image for native ORT detection.
///
/// Takes `&mut` to move large pixel buffers out of the request (via `std::mem::take`)
/// instead of cloning them.
pub(crate) fn resolve_detect_input(
    request: &mut ScannerDetectDocumentRequest,
) -> Result<ResolvedDetectInput, String> {
    let (decoded_image, input_transport) = if request.use_latest_preview_frame {
        (resolve_detect_from_preview_cache()?, "latest-preview-cache")
    } else if !request.rgba_bytes.is_empty() {
        (Some(resolve_detect_from_rgba_owned(request)?), "rgba-ipc")
    } else if !request.source_bytes.is_empty() {
        (
            Some(resolve_detect_from_encoded_owned(request)?),
            "source-bytes",
        )
    } else {
        (None, "none")
    };

    Ok(ResolvedDetectInput {
        // Skip the max_width/max_height pre-shrink for the Native ORT path.
        // The model function (`run_docaligner_fastvit_sa24`) will resize the
        // image to the model's input_size (e.g. 256x256) in a single step.
        // Applying the frontend's processing bounds here would create a wasteful
        // double-resize chain (for example, 640x360 -> 320x180 -> 256x256) that degrades
        // the heatmap quality through accumulated interpolation blur.
        prepared_image: decoded_image.map(|image| prepare_inference_image(image, None, None)),
        input_transport,
    })
}

pub(crate) fn build_dynamic_image_from_preview_frame_packet(
    packet: &[u8],
) -> Result<DynamicImage, String> {
    scanner_frame_protocol::build_dynamic_image_from_preview_frame_packet(packet)
}

pub(crate) fn scale_points_between_dimensions(
    points: Vec<ScannerPoint>,
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> Vec<ScannerPoint> {
    if source_width == target_width && source_height == target_height {
        return points;
    }

    points
        .into_iter()
        .map(|point| ScannerPoint {
            x: scale_coordinate(point.x, source_width, target_width),
            y: scale_coordinate(point.y, source_height, target_height),
        })
        .collect()
}

pub(crate) fn scale_coordinate(value: f32, source_size: u32, target_size: u32) -> f32 {
    if source_size <= 1 || target_size <= 1 {
        return 0.0;
    }

    (((value + 0.5) / source_size as f32) * target_size as f32 - 0.5)
        .clamp(0.0, target_size.saturating_sub(1) as f32)
}

/// Resolve detection input from the latest preview frame cache.
fn resolve_detect_from_preview_cache() -> Result<Option<DynamicImage>, String> {
    let cached_preview_packet = get_latest_preview_frame_packet();
    cached_preview_packet
        .as_deref()
        .map(|packet| build_dynamic_image_from_preview_frame_packet(packet))
        .transpose()
}

/// Resolve detection input from raw RGBA bytes in the request (takes ownership).
fn resolve_detect_from_rgba_owned(
    request: &mut ScannerDetectDocumentRequest,
) -> Result<DynamicImage, String> {
    let width = request
        .rgba_width
        .ok_or_else(|| "RGBA native scanner request is missing rgbaWidth.".to_string())?;
    let height = request
        .rgba_height
        .ok_or_else(|| "RGBA native scanner request is missing rgbaHeight.".to_string())?;

    if width == 0 || height == 0 {
        return Err("RGBA native scanner dimensions must be greater than zero.".to_string());
    }

    let pixel_count = u64::from(width) * u64::from(height);
    if pixel_count > MAX_DETECT_IMAGE_PIXELS {
        return Err(format!(
            "RGBA native scanner dimensions {width}x{height} ({} pixels) exceed limit of {}.",
            pixel_count, MAX_DETECT_IMAGE_PIXELS,
        ));
    }

    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "RGBA native scanner dimensions overflowed.".to_string())?;

    if request.rgba_bytes.len() != expected_len {
        return Err(format!(
            "RGBA native scanner payload length mismatch: expected {expected_len} bytes for {width}x{height}, got {}.",
            request.rgba_bytes.len()
        ));
    }

    // Take ownership to avoid cloning the large pixel buffer.
    let rgba_bytes = std::mem::take(&mut request.rgba_bytes);
    let image = RgbaImage::from_raw(width, height, rgba_bytes).ok_or_else(|| {
        "Failed to materialize RGBA source frame for native scanner inference.".to_string()
    })?;
    Ok(DynamicImage::ImageRgba8(image))
}

/// Resolve detection input from encoded image bytes (PNG/JPEG) in the request (takes ownership).
fn resolve_detect_from_encoded_owned(
    request: &mut ScannerDetectDocumentRequest,
) -> Result<DynamicImage, String> {
    let source_bytes = std::mem::take(&mut request.source_bytes);
    decode_detect_image_with_limits(&source_bytes, "source image for native scanner inference")
}

fn prepare_inference_image(
    image: DynamicImage,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> PreparedInferenceImage {
    let (original_width, original_height) = image.dimensions();
    let (working_width, working_height) =
        bounded_dimensions(original_width, original_height, max_width, max_height);

    let working_image = if working_width == original_width && working_height == original_height {
        image
    } else {
        // Resize in RGB to avoid an unnecessary RGBA round trip; the downstream
        // inference path (`run_docaligner_fastvit_sa24`) converts to RGB anyway.
        DynamicImage::ImageRgb8(image::imageops::resize(
            &image.to_rgb8(),
            working_width,
            working_height,
            FilterType::Triangle,
        ))
    };

    PreparedInferenceImage {
        original_width,
        original_height,
        working_image,
    }
}

fn bounded_dimensions(
    width: u32,
    height: u32,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> (u32, u32) {
    let max_width = max_width.filter(|value| *value > 0).unwrap_or(width);
    let max_height = max_height.filter(|value| *value > 0).unwrap_or(height);
    let scale = f64::min(
        1.0,
        f64::min(
            max_width as f64 / f64::from(width.max(1)),
            max_height as f64 / f64::from(height.max(1)),
        ),
    );

    (
        u32::max(1, (f64::from(width) * scale).round() as u32),
        u32::max(1, (f64::from(height) * scale).round() as u32),
    )
}
