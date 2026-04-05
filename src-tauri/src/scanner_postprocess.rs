use std::collections::VecDeque;
use std::io::Cursor;
use std::time::Instant;

use image::codecs::png::PngEncoder;
use image::imageops::{rotate180, rotate270, rotate90};
use image::{ColorType, DynamicImage, GrayImage, ImageEncoder, Luma, Rgba, RgbaImage};
use imageproc::contrast::otsu_level;
use imageproc::filter::gaussian_blur_f32;
use imageproc::geometric_transformations::{warp_into, Interpolation, Projection};
use serde::{Deserialize, Serialize};
use tauri::{
    command,
    ipc::{Channel, InvokeResponseBody},
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ScannerPoint {
    x: f32,
    y: f32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPostProcessRequest {
    source_bytes: Vec<u8>,
    document_points: Option<Vec<ScannerPoint>>,
    output_rotation: u16,
    image_enhancement: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerPostProcessResponse {
    processing_ms: f64,
    decode_ms: f64,
    refine_ms: Option<f64>,
    perspective_ms: Option<f64>,
    flatten_ms: Option<f64>,
    crop_ms: Option<f64>,
    enhance_ms: Option<f64>,
    rotate_ms: Option<f64>,
    encode_ms: f64,
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    encoded_mime_type: &'static str,
    effective_document_points: Option<Vec<ScannerPoint>>,
    refinement_applied: bool,
    local_flattening_applied: bool,
    paper_crop_applied: bool,
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

#[command]
pub async fn tauri_scanner_postprocess_image(
    request: ScannerPostProcessRequest,
    payload_channel: Channel<InvokeResponseBody>,
) -> Result<ScannerPostProcessResponse, String> {
    let (response, encoded_png) =
        tauri::async_runtime::spawn_blocking(move || process_image_request(request))
            .await
            .map_err(|error| format!("Native scanner post-process task failed: {error}"))??;

    send_raw_payload(
        &payload_channel,
        encoded_png,
        "native scanner post-process result",
    )?;
    Ok(response)
}

pub fn postprocess_image_bytes(
    source_bytes: Vec<u8>,
    document_points: Option<&[crate::scanner_detect::ScannerPoint]>,
    output_rotation: u16,
    image_enhancement: bool,
) -> Result<(ScannerPostProcessResponse, Vec<u8>), String> {
    let request = ScannerPostProcessRequest {
        source_bytes,
        document_points: document_points.map(|points| {
            points
                .iter()
                .map(|point| ScannerPoint {
                    x: point.x(),
                    y: point.y(),
                })
                .collect()
        }),
        output_rotation,
        image_enhancement,
    };
    process_image_request(request)
}

fn process_image_request(
    request: ScannerPostProcessRequest,
) -> Result<(ScannerPostProcessResponse, Vec<u8>), String> {
    let started_at = Instant::now();
    let decode_started_at = Instant::now();
    let decoded = image::load_from_memory(&request.source_bytes)
        .map_err(|error| format!("Failed to decode source image: {error}"))?;
    let decode_ms = decode_started_at.elapsed().as_secs_f64() * 1000.0;

    let mut current = decoded.into_rgba8();
    let input_width = current.width();
    let input_height = current.height();
    let mut refine_ms = None;
    let mut perspective_ms = None;
    let mut flatten_ms = None;
    let mut crop_ms = None;
    let mut enhance_ms = None;
    let mut rotate_ms = None;
    let mut refinement_applied = false;
    let mut local_flattening_applied = false;
    let mut paper_crop_applied = false;
    let effective_document_points =
        if let Some(points) = normalize_document_points(request.document_points.as_deref())? {
            let refine_started_at = Instant::now();
            let refined_points = refine_document_points(&current, &points);
            refine_ms = Some(refine_started_at.elapsed().as_secs_f64() * 1000.0);
            refinement_applied = refinement_applied_between(&points, &refined_points);
            Some(refined_points)
        } else {
            None
        };

    if let Some(points) = effective_document_points.as_ref() {
        validate_quad_geometry(points, input_width, input_height)?;
        let perspective_started_at = Instant::now();
        current = warp_document_to_rect(&current, points)?;
        perspective_ms = Some(perspective_started_at.elapsed().as_secs_f64() * 1000.0);
    }

    if request.output_rotation != 0 {
        let rotate_started_at = Instant::now();
        current = rotate_image(current, request.output_rotation)?;
        rotate_ms = Some(rotate_started_at.elapsed().as_secs_f64() * 1000.0);
    }

    if effective_document_points.is_some() {
        let flatten_started_at = Instant::now();
        let flatten_result = apply_local_spine_flattening(&current);
        flatten_ms = Some(flatten_started_at.elapsed().as_secs_f64() * 1000.0);
        current = flatten_result.image;
        local_flattening_applied = flatten_result.applied;
    }

    if effective_document_points.is_some() && !local_flattening_applied {
        let crop_started_at = Instant::now();
        let crop_result = crop_to_paper_region(&current);
        crop_ms = Some(crop_started_at.elapsed().as_secs_f64() * 1000.0);
        current = crop_result.image;
        paper_crop_applied = crop_result.applied;
    }

    if request.image_enhancement {
        let enhance_started_at = Instant::now();
        current = enhance_document_image(&current, local_flattening_applied);
        enhance_ms = Some(enhance_started_at.elapsed().as_secs_f64() * 1000.0);
    }

    let encode_started_at = Instant::now();
    let encoded_png = encode_png(&current)?;
    let encode_ms = encode_started_at.elapsed().as_secs_f64() * 1000.0;

    let response = ScannerPostProcessResponse {
        processing_ms: started_at.elapsed().as_secs_f64() * 1000.0,
        decode_ms,
        refine_ms,
        perspective_ms,
        flatten_ms,
        crop_ms,
        enhance_ms,
        rotate_ms,
        encode_ms,
        input_width,
        input_height,
        output_width: current.width(),
        output_height: current.height(),
        encoded_mime_type: "image/png",
        effective_document_points: effective_document_points.map(|points| points.to_vec()),
        refinement_applied,
        local_flattening_applied,
        paper_crop_applied,
    };

    Ok((response, encoded_png))
}

fn normalize_document_points(
    points: Option<&[ScannerPoint]>,
) -> Result<Option<[ScannerPoint; 4]>, String> {
    let Some(points) = points else {
        return Ok(None);
    };

    if points.len() != 4 {
        return Err(format!(
            "Perspective transform requires exactly 4 points, got {}.",
            points.len()
        ));
    }

    let mut ordered = *<&[ScannerPoint; 4]>::try_from(points)
        .map_err(|_| "Failed to normalize document points.".to_string())?;
    ordered = order_points(ordered);
    Ok(Some(ordered))
}

fn order_points(points: [ScannerPoint; 4]) -> [ScannerPoint; 4] {
    let mut sums = [0.0f32; 4];
    let mut diffs = [0.0f32; 4];

    for (index, point) in points.iter().enumerate() {
        sums[index] = point.x + point.y;
        diffs[index] = point.y - point.x;
    }

    [
        points[index_of_min(&sums)],
        points[index_of_min(&diffs)],
        points[index_of_max(&sums)],
        points[index_of_max(&diffs)],
    ]
}

#[derive(Debug, Clone, Copy)]
struct LineFit {
    point: ScannerPoint,
    direction: (f32, f32),
}

const REFINE_SAMPLE_MARGIN_RATIO: f32 = 0.12;
const REFINE_SAMPLE_SPACING_PX: f32 = 18.0;
const REFINE_MIN_SAMPLE_COUNT: usize = 8;
const REFINE_MAX_SAMPLE_COUNT: usize = 32;
const REFINE_SEARCH_RADIUS_RATIO: f32 = 0.08;
const REFINE_MIN_SEARCH_RADIUS_PX: f32 = 4.0;
const REFINE_MAX_SEARCH_RADIUS_PX: f32 = 28.0;
const REFINE_MAX_CORNER_SHIFT_RATIO: f32 = 0.10;
const REFINE_MIN_CORNER_SHIFT_PX: f32 = 8.0;
const REFINE_MAX_CORNER_SHIFT_PX: f32 = 20.0;
const REFINE_APPLIED_DELTA_PX: f32 = 0.75;
const FLATTEN_DETECTION_BAND_RATIO: f32 = 0.22;
const FLATTEN_APPLY_BAND_RATIO: f32 = 0.22;
const FLATTEN_MIN_BAND_PX: usize = 24;
const FLATTEN_MAX_BAND_RATIO: f32 = 0.45;
const FLATTEN_MIN_VALID_ROWS_RATIO: f32 = 0.18;
const FLATTEN_MIN_MEAN_SHIFT_PX: f32 = 3.5;
const FLATTEN_MIN_MAX_SHIFT_PX: f32 = 7.0;
const FLATTEN_MAX_SHIFT_PX: f32 = 14.0;
const FLATTEN_SMOOTHING_RADIUS: usize = 4;
const FLATTEN_EDGE_ANCHOR_PX: f32 = 12.0;
const FLATTEN_MIN_EDGE_ANCHOR_RATIO: f32 = 0.2;
const FLATTEN_DOMINANT_EDGE_ANCHOR_RATIO: f32 = 1.35;
const FLATTEN_DOMINANT_SCORE_RATIO: f32 = 1.15;
#[allow(dead_code)]
const PAPER_CROP_MIN_COMPONENT_AREA_RATIO: f32 = 0.12;
#[allow(dead_code)]
const PAPER_CROP_MIN_BBOX_AREA_RATIO: f32 = 0.18;
#[allow(dead_code)]
const PAPER_CROP_SKIP_IF_NEAR_FULL_RATIO: f32 = 0.985;
#[allow(dead_code)]
const PAPER_CROP_MARGIN_RATIO: f32 = 0.02;
#[allow(dead_code)]
const PAPER_CROP_MIN_MARGIN_PX: u32 = 4;
#[allow(dead_code)]
const PAPER_CROP_MIN_SCORE_THRESHOLD: u8 = 160;
const PAPER_SCORE_BLUR_SIGMA: f32 = 6.0;
const PAPER_SCORE_LOCAL_CONTRAST_WEIGHT: f32 = 1.15;
const ENHANCE_MIN_PAPER_BBOX_RATIO: f32 = 0.88;

#[derive(Debug, Clone)]
struct FlattenCandidate {
    side: FlattenSide,
    row_shifts: Vec<f32>,
    score: f32,
    max_shift: f32,
    edge_anchor_ratio: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlattenSide {
    Left,
    Right,
}

#[derive(Debug)]
struct FlattenResult {
    image: RgbaImage,
    applied: bool,
}

#[allow(dead_code)]
#[derive(Debug)]
struct PaperCropResult {
    image: RgbaImage,
    applied: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct BoundingBox {
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
}

#[allow(dead_code)]
impl BoundingBox {
    fn width(&self) -> u32 {
        self.max_x.saturating_sub(self.min_x).saturating_add(1)
    }

    fn height(&self) -> u32 {
        self.max_y.saturating_sub(self.min_y).saturating_add(1)
    }
}

fn refine_document_points(
    source: &RgbaImage,
    coarse_points: &[ScannerPoint; 4],
) -> [ScannerPoint; 4] {
    let gray = DynamicImage::ImageRgba8(source.clone()).into_luma8();
    let side_lengths = [
        point_distance(coarse_points[0], coarse_points[1]),
        point_distance(coarse_points[1], coarse_points[2]),
        point_distance(coarse_points[2], coarse_points[3]),
        point_distance(coarse_points[3], coarse_points[0]),
    ];
    let shortest_side = side_lengths
        .into_iter()
        .filter(|length| length.is_finite() && *length > 0.0)
        .fold(f32::INFINITY, f32::min);
    if !shortest_side.is_finite() {
        return *coarse_points;
    }

    let search_radius = (shortest_side * REFINE_SEARCH_RADIUS_RATIO)
        .clamp(REFINE_MIN_SEARCH_RADIUS_PX, REFINE_MAX_SEARCH_RADIUS_PX)
        .round() as i32;
    let max_corner_shift = (shortest_side * REFINE_MAX_CORNER_SHIFT_RATIO)
        .clamp(REFINE_MIN_CORNER_SHIFT_PX, REFINE_MAX_CORNER_SHIFT_PX);

    let edges = [
        (coarse_points[0], coarse_points[1]),
        (coarse_points[1], coarse_points[2]),
        (coarse_points[2], coarse_points[3]),
        (coarse_points[3], coarse_points[0]),
    ];
    let fitted_lines = edges.map(|(start, end)| {
        let samples = collect_refined_edge_samples(&gray, start, end, search_radius);
        fit_line_to_points(&samples)
    });

    let refined_candidates = [
        fitted_lines[0]
            .zip(fitted_lines[3])
            .and_then(|(first, second)| intersect_lines(first, second)),
        fitted_lines[0]
            .zip(fitted_lines[1])
            .and_then(|(first, second)| intersect_lines(first, second)),
        fitted_lines[1]
            .zip(fitted_lines[2])
            .and_then(|(first, second)| intersect_lines(first, second)),
        fitted_lines[2]
            .zip(fitted_lines[3])
            .and_then(|(first, second)| intersect_lines(first, second)),
    ];

    let centroid = ScannerPoint {
        x: coarse_points.iter().map(|p| p.x).sum::<f32>() / 4.0,
        y: coarse_points.iter().map(|p| p.y).sum::<f32>() / 4.0,
    };
    let image_boundary_margin = shortest_side * 0.04;

    let mut limited_points = *coarse_points;
    for (index, candidate) in refined_candidates.into_iter().enumerate() {
        if let Some(candidate_point) = candidate {
            let coarse = coarse_points[index];
            let near_boundary = coarse.x < image_boundary_margin
                || coarse.y < image_boundary_margin
                || coarse.x > source.width() as f32 - image_boundary_margin
                || coarse.y > source.height() as f32 - image_boundary_margin;

            if near_boundary {
                let coarse_to_center = ((centroid.x - coarse.x).powi(2) + (centroid.y - coarse.y).powi(2)).sqrt();
                let candidate_to_center = ((centroid.x - candidate_point.x).powi(2) + (centroid.y - candidate_point.y).powi(2)).sqrt();
                if candidate_to_center < coarse_to_center {
                    // Corner is near image edge and refinement wants to pull it inward —
                    // this edge is likely the real page boundary (e.g. spine side).
                    // Skip refinement entirely for this corner.
                    continue;
                }
            }

            let limited =
                limit_point_shift(candidate_point, coarse, max_corner_shift);
            limited_points[index] =
                clamp_point_to_image_bounds(limited, source.width(), source.height());
        }
    }

    let coarse_area = polygon_area(coarse_points);
    let refined_area = polygon_area(&limited_points);
    if !refined_area.is_finite()
        || refined_area <= 0.0
        || refined_area < coarse_area * 0.5
        || refined_area > coarse_area * 1.5
    {
        return *coarse_points;
    }

    limited_points
}

fn collect_refined_edge_samples(
    gray: &GrayImage,
    start: ScannerPoint,
    end: ScannerPoint,
    search_radius: i32,
) -> Vec<ScannerPoint> {
    let edge_length = point_distance(start, end);
    if !edge_length.is_finite() || edge_length < 1.0 {
        return Vec::new();
    }

    let tangent_x = (end.x - start.x) / edge_length;
    let tangent_y = (end.y - start.y) / edge_length;
    let normal_x = -tangent_y;
    let normal_y = tangent_x;
    let sample_count = ((edge_length / REFINE_SAMPLE_SPACING_PX).round() as usize)
        .clamp(REFINE_MIN_SAMPLE_COUNT, REFINE_MAX_SAMPLE_COUNT);
    let mut samples: Vec<(ScannerPoint, f32)> = Vec::with_capacity(sample_count);

    for index in 0..sample_count {
        let progress = if sample_count <= 1 {
            0.5
        } else {
            REFINE_SAMPLE_MARGIN_RATIO
                + (((1.0 - (REFINE_SAMPLE_MARGIN_RATIO * 2.0)) * index as f32)
                    / (sample_count - 1) as f32)
        };
        let center_x = start.x + ((end.x - start.x) * progress);
        let center_y = start.y + ((end.y - start.y) * progress);

        let mut best_strength = f32::NEG_INFINITY;
        let mut best_point = None;
        for offset in -search_radius..=search_radius {
            let strength = compute_normal_edge_strength(
                gray,
                center_x,
                center_y,
                normal_x,
                normal_y,
                offset as f32,
            );
            if let Some(strength_value) = strength {
                if strength_value > best_strength {
                    best_strength = strength_value;
                    best_point = Some(ScannerPoint {
                        x: center_x + (offset as f32 * normal_x),
                        y: center_y + (offset as f32 * normal_y),
                    });
                }
            }
        }

        if let Some(point) = best_point {
            samples.push((point, best_strength));
        }
    }

    if samples.len() < 2 {
        return Vec::new();
    }

    let average_strength =
        samples.iter().map(|(_, strength)| *strength).sum::<f32>() / samples.len() as f32;
    let threshold = average_strength * 0.6;
    let filtered: Vec<ScannerPoint> = samples
        .iter()
        .filter(|(_, strength)| *strength >= threshold)
        .map(|(point, _)| *point)
        .collect();

    if filtered.len() >= 2 {
        filtered
    } else {
        samples.into_iter().map(|(point, _)| point).collect()
    }
}

fn compute_normal_edge_strength(
    gray: &GrayImage,
    center_x: f32,
    center_y: f32,
    normal_x: f32,
    normal_y: f32,
    offset: f32,
) -> Option<f32> {
    let outside_near = sample_gray(
        gray,
        center_x + ((offset - 1.0) * normal_x),
        center_y + ((offset - 1.0) * normal_y),
    )?;
    let outside_far = sample_gray(
        gray,
        center_x + ((offset - 2.0) * normal_x),
        center_y + ((offset - 2.0) * normal_y),
    )?;
    let inside_near = sample_gray(
        gray,
        center_x + ((offset + 1.0) * normal_x),
        center_y + ((offset + 1.0) * normal_y),
    )?;
    let inside_far = sample_gray(
        gray,
        center_x + ((offset + 2.0) * normal_x),
        center_y + ((offset + 2.0) * normal_y),
    )?;

    let outside = (outside_near + outside_far) * 0.5;
    let inside = (inside_near + inside_far) * 0.5;
    Some((inside - outside).max(0.0))
}

fn sample_gray(gray: &GrayImage, x: f32, y: f32) -> Option<f32> {
    if x < 0.0 || y < 0.0 || x > gray.width() as f32 - 1.0 || y > gray.height() as f32 - 1.0 {
        return None;
    }

    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = x0.saturating_add(1).min(gray.width().saturating_sub(1));
    let y1 = y0.saturating_add(1).min(gray.height().saturating_sub(1));
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;

    let top_left = gray.get_pixel(x0, y0).0[0] as f32;
    let top_right = gray.get_pixel(x1, y0).0[0] as f32;
    let bottom_left = gray.get_pixel(x0, y1).0[0] as f32;
    let bottom_right = gray.get_pixel(x1, y1).0[0] as f32;

    let top = top_left + ((top_right - top_left) * tx);
    let bottom = bottom_left + ((bottom_right - bottom_left) * tx);
    Some(top + ((bottom - top) * ty))
}

fn fit_line_to_points(points: &[ScannerPoint]) -> Option<LineFit> {
    if points.len() < 2 {
        return None;
    }

    let centroid = ScannerPoint {
        x: points.iter().map(|point| point.x).sum::<f32>() / points.len() as f32,
        y: points.iter().map(|point| point.y).sum::<f32>() / points.len() as f32,
    };

    let mut covariance_xx = 0.0;
    let mut covariance_xy = 0.0;
    let mut covariance_yy = 0.0;
    for point in points {
        let delta_x = point.x - centroid.x;
        let delta_y = point.y - centroid.y;
        covariance_xx += delta_x * delta_x;
        covariance_xy += delta_x * delta_y;
        covariance_yy += delta_y * delta_y;
    }

    let angle = 0.5 * (2.0 * covariance_xy).atan2(covariance_xx - covariance_yy);
    let direction = (angle.cos(), angle.sin());
    if !direction.0.is_finite() || !direction.1.is_finite() {
        return None;
    }

    Some(LineFit {
        point: centroid,
        direction,
    })
}

fn intersect_lines(first: LineFit, second: LineFit) -> Option<ScannerPoint> {
    let denominator =
        (first.direction.0 * second.direction.1) - (first.direction.1 * second.direction.0);
    if denominator.abs() < 1e-3 {
        return None;
    }

    let delta_x = second.point.x - first.point.x;
    let delta_y = second.point.y - first.point.y;
    let distance_along_first =
        ((delta_x * second.direction.1) - (delta_y * second.direction.0)) / denominator;

    Some(ScannerPoint {
        x: first.point.x + (distance_along_first * first.direction.0),
        y: first.point.y + (distance_along_first * first.direction.1),
    })
}

fn limit_point_shift(
    candidate: ScannerPoint,
    fallback: ScannerPoint,
    max_shift: f32,
) -> ScannerPoint {
    let delta_x = candidate.x - fallback.x;
    let delta_y = candidate.y - fallback.y;
    let distance = (delta_x * delta_x + delta_y * delta_y).sqrt();
    if !distance.is_finite() || distance <= max_shift {
        return candidate;
    }

    let scale = max_shift / distance;
    ScannerPoint {
        x: fallback.x + (delta_x * scale),
        y: fallback.y + (delta_y * scale),
    }
}

fn clamp_point_to_image_bounds(point: ScannerPoint, width: u32, height: u32) -> ScannerPoint {
    ScannerPoint {
        x: point.x.clamp(0.0, width.saturating_sub(1) as f32),
        y: point.y.clamp(0.0, height.saturating_sub(1) as f32),
    }
}

fn polygon_area(points: &[ScannerPoint; 4]) -> f32 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        area += (current.x * next.y) - (next.x * current.y);
    }

    area.abs() * 0.5
}

const QUAD_MIN_EDGE_RATIO: f32 = 0.25;
const QUAD_MIN_AREA_RATIO: f32 = 0.05;

fn validate_quad_geometry(
    points: &[ScannerPoint; 4],
    image_width: u32,
    image_height: u32,
) -> Result<(), String> {
    let [tl, tr, br, bl] = points;

    let top = point_distance(*tl, *tr);
    let right = point_distance(*tr, *br);
    let bottom = point_distance(*bl, *br);
    let left = point_distance(*tl, *bl);

    if !top.is_finite() || !right.is_finite() || !bottom.is_finite() || !left.is_finite() {
        return Err("Document quad has non-finite edge lengths.".to_string());
    }

    let horizontal_ratio = top.min(bottom) / top.max(bottom).max(1.0);
    if horizontal_ratio < QUAD_MIN_EDGE_RATIO {
        return Err(format!(
            "Document quad is degenerate: horizontal edge ratio {horizontal_ratio:.3} < {QUAD_MIN_EDGE_RATIO} \
             (top={top:.0}, bottom={bottom:.0})."
        ));
    }

    let vertical_ratio = left.min(right) / left.max(right).max(1.0);
    if vertical_ratio < QUAD_MIN_EDGE_RATIO {
        return Err(format!(
            "Document quad is degenerate: vertical edge ratio {vertical_ratio:.3} < {QUAD_MIN_EDGE_RATIO} \
             (left={left:.0}, right={right:.0})."
        ));
    }

    let quad_area = polygon_area(points);
    let image_area = image_width as f32 * image_height as f32;
    let area_ratio = quad_area / image_area.max(1.0);
    if area_ratio < QUAD_MIN_AREA_RATIO {
        return Err(format!(
            "Document quad is too small: area ratio {area_ratio:.3} < {QUAD_MIN_AREA_RATIO}."
        ));
    }

    let edges = [
        (tr.x - tl.x, tr.y - tl.y),
        (br.x - tr.x, br.y - tr.y),
        (bl.x - br.x, bl.y - br.y),
        (tl.x - bl.x, tl.y - bl.y),
    ];
    let mut positive = 0u32;
    let mut negative = 0u32;
    for i in 0..4 {
        let (ax, ay) = edges[i];
        let (bx, by) = edges[(i + 1) % 4];
        let cross = ax * by - ay * bx;
        if cross > 0.0 {
            positive += 1;
        } else if cross < 0.0 {
            negative += 1;
        }
    }
    if positive > 0 && negative > 0 {
        return Err("Document quad is non-convex or self-intersecting.".to_string());
    }

    Ok(())
}

fn refinement_applied_between(
    coarse_points: &[ScannerPoint; 4],
    refined_points: &[ScannerPoint; 4],
) -> bool {
    coarse_points
        .iter()
        .zip(refined_points.iter())
        .any(|(coarse, refined)| point_distance(*coarse, *refined) >= REFINE_APPLIED_DELTA_PX)
}

fn apply_local_spine_flattening(source: &RgbaImage) -> FlattenResult {
    if source.width() < 48 || source.height() < 48 {
        return FlattenResult {
            image: source.clone(),
            applied: false,
        };
    }

    let gray = DynamicImage::ImageRgba8(source.clone()).into_luma8();
    let band_width = ((source.width() as f32 * FLATTEN_DETECTION_BAND_RATIO).round() as usize)
        .clamp(
            FLATTEN_MIN_BAND_PX,
            (source.width() as f32 * 0.33)
                .floor()
                .max(FLATTEN_MIN_BAND_PX as f32) as usize,
        );
    let candidates = [
        build_flatten_candidate(&gray, FlattenSide::Left, band_width),
        build_flatten_candidate(&gray, FlattenSide::Right, band_width),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    if candidates.is_empty() {
        return FlattenResult {
            image: source.clone(),
            applied: false,
        };
    }

    let Some(selected) = select_flatten_candidate(candidates) else {
        return FlattenResult {
            image: source.clone(),
            applied: false,
        };
    };

    FlattenResult {
        image: apply_flatten_warp(source, selected.side, &selected.row_shifts),
        applied: true,
    }
}

fn select_flatten_candidate(mut candidates: Vec<FlattenCandidate>) -> Option<FlattenCandidate> {
    candidates.sort_by(|first, second| {
        second
            .edge_anchor_ratio
            .partial_cmp(&first.edge_anchor_ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                second
                    .score
                    .partial_cmp(&first.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                second
                    .max_shift
                    .partial_cmp(&first.max_shift)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let selected = candidates.first()?.clone();
    if selected.max_shift < FLATTEN_MIN_MAX_SHIFT_PX {
        return None;
    }

    if let Some(runner_up) = candidates.get(1) {
        let edge_anchor_dominant = selected.edge_anchor_ratio
            >= runner_up.edge_anchor_ratio * FLATTEN_DOMINANT_EDGE_ANCHOR_RATIO;
        let score_dominant = selected.score >= runner_up.score * FLATTEN_DOMINANT_SCORE_RATIO;
        if !edge_anchor_dominant && !score_dominant {
            return None;
        }
    }

    Some(selected)
}

#[allow(dead_code)]
fn crop_to_paper_region(source: &RgbaImage) -> PaperCropResult {
    if source.width() < 48 || source.height() < 48 {
        return PaperCropResult {
            image: source.clone(),
            applied: false,
        };
    }

    let score_image = build_paper_score_image(source);
    let threshold = otsu_level(&score_image).max(PAPER_CROP_MIN_SCORE_THRESHOLD);
    let Some(bounding_box) = largest_paper_component_bbox(&score_image, threshold) else {
        return PaperCropResult {
            image: source.clone(),
            applied: false,
        };
    };

    let component_bbox_area = bounding_box.width() as f32 * bounding_box.height() as f32;
    let image_area = source.width() as f32 * source.height() as f32;
    let bbox_area_ratio = component_bbox_area / image_area.max(1.0);
    if bbox_area_ratio < PAPER_CROP_MIN_BBOX_AREA_RATIO
        || bbox_area_ratio >= PAPER_CROP_SKIP_IF_NEAR_FULL_RATIO
    {
        return PaperCropResult {
            image: source.clone(),
            applied: false,
        };
    }

    let margin_x = ((bounding_box.width() as f32 * PAPER_CROP_MARGIN_RATIO).round() as u32)
        .max(PAPER_CROP_MIN_MARGIN_PX);
    let margin_y = ((bounding_box.height() as f32 * PAPER_CROP_MARGIN_RATIO).round() as u32)
        .max(PAPER_CROP_MIN_MARGIN_PX);
    let crop_x = bounding_box.min_x.saturating_sub(margin_x);
    let crop_y = bounding_box.min_y.saturating_sub(margin_y);
    let crop_max_x = bounding_box
        .max_x
        .saturating_add(margin_x)
        .min(source.width().saturating_sub(1));
    let crop_max_y = bounding_box
        .max_y
        .saturating_add(margin_y)
        .min(source.height().saturating_sub(1));
    let crop_width = crop_max_x.saturating_sub(crop_x).saturating_add(1);
    let crop_height = crop_max_y.saturating_sub(crop_y).saturating_add(1);

    if crop_width >= source.width() && crop_height >= source.height() {
        return PaperCropResult {
            image: source.clone(),
            applied: false,
        };
    }

    PaperCropResult {
        image: image::imageops::crop_imm(source, crop_x, crop_y, crop_width, crop_height)
            .to_image(),
        applied: true,
    }
}

#[allow(dead_code)]
fn build_paper_score_image(source: &RgbaImage) -> GrayImage {
    let mut score = GrayImage::new(source.width(), source.height());
    let gray = DynamicImage::ImageRgba8(source.clone()).into_luma8();
    let blurred = gaussian_blur_f32(&gray, PAPER_SCORE_BLUR_SIGMA);

    for (x, y, pixel) in source.enumerate_pixels() {
        let [red, green, blue, _] = pixel.0;
        let max_channel = red.max(green).max(blue) as f32;
        let min_channel = red.min(green).min(blue) as f32;
        let saturation = max_channel - min_channel;
        let local_luminance = gray.get_pixel(x, y).0[0] as f32;
        let neighborhood_luminance = blurred.get_pixel(x, y).0[0] as f32;
        let local_contrast = (local_luminance - neighborhood_luminance).abs();
        let paper_score = (neighborhood_luminance
            - saturation * 0.9
            - local_contrast * PAPER_SCORE_LOCAL_CONTRAST_WEIGHT)
            .clamp(0.0, 255.0) as u8;
        score.put_pixel(x, y, Luma([paper_score]));
    }

    score
}

#[allow(dead_code)]
fn largest_paper_component_bbox(score_image: &GrayImage, threshold: u8) -> Option<BoundingBox> {
    let width = score_image.width() as usize;
    let height = score_image.height() as usize;
    let min_component_area =
        ((width * height) as f32 * PAPER_CROP_MIN_COMPONENT_AREA_RATIO).round() as usize;
    let mut visited = vec![false; width * height];
    let mut best_bbox = None;
    let mut best_score = f32::NEG_INFINITY;

    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if visited[index] || score_image.get_pixel(x as u32, y as u32).0[0] < threshold {
                continue;
            }

            let mut queue = VecDeque::from([(x as u32, y as u32)]);
            visited[index] = true;
            let mut area = 0usize;
            let mut score_sum = 0f32;
            let mut bbox = BoundingBox {
                min_x: x as u32,
                min_y: y as u32,
                max_x: x as u32,
                max_y: y as u32,
            };

            while let Some((cx, cy)) = queue.pop_front() {
                area += 1;
                score_sum += score_image.get_pixel(cx, cy).0[0] as f32;
                bbox.min_x = bbox.min_x.min(cx);
                bbox.min_y = bbox.min_y.min(cy);
                bbox.max_x = bbox.max_x.max(cx);
                bbox.max_y = bbox.max_y.max(cy);

                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }

                        let nx = cx as i32 + dx;
                        let ny = cy as i32 + dy;
                        if nx < 0
                            || ny < 0
                            || nx >= score_image.width() as i32
                            || ny >= score_image.height() as i32
                        {
                            continue;
                        }

                        let nx = nx as u32;
                        let ny = ny as u32;
                        let neighbor_index = ny as usize * width + nx as usize;
                        if visited[neighbor_index] || score_image.get_pixel(nx, ny).0[0] < threshold
                        {
                            continue;
                        }

                        visited[neighbor_index] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }

            if area < min_component_area {
                continue;
            }

            let mean_score = score_sum / area as f32 / 255.0;
            let fill_ratio = area as f32 / (bbox.width() as f32 * bbox.height() as f32).max(1.0);
            let component_score = area as f32 * (mean_score + fill_ratio);
            if component_score > best_score {
                best_score = component_score;
                best_bbox = Some(bbox);
            }
        }
    }

    best_bbox
}

fn build_flatten_candidate(
    gray: &GrayImage,
    side: FlattenSide,
    band_width: usize,
) -> Option<FlattenCandidate> {
    let offsets = (0..gray.height())
        .map(|row| detect_edge_offset_for_row(gray, row, side, band_width))
        .collect::<Vec<_>>();
    let valid_offsets = offsets.iter().flatten().copied().collect::<Vec<_>>();

    if valid_offsets.len()
        < ((gray.height() as f32 * FLATTEN_MIN_VALID_ROWS_RATIO).floor() as usize).max(6)
    {
        return None;
    }

    let baseline = percentile(&valid_offsets, 0.15);
    let edge_anchor_ratio = valid_offsets
        .iter()
        .filter(|value| **value <= FLATTEN_EDGE_ANCHOR_PX)
        .count() as f32
        / valid_offsets.len() as f32;
    if edge_anchor_ratio < FLATTEN_MIN_EDGE_ANCHOR_RATIO {
        return None;
    }
    let raw_shifts = offsets
        .iter()
        .map(|value| value.map(|value| (value - baseline).clamp(0.0, FLATTEN_MAX_SHIFT_PX)))
        .collect::<Vec<_>>();
    let filled_shifts = fill_missing_offsets(&raw_shifts, 0.0);
    let smoothed_shifts = smooth_values(&filled_shifts, FLATTEN_SMOOTHING_RADIUS);
    let positive_shifts = smoothed_shifts
        .iter()
        .copied()
        .filter(|value| *value > 0.5)
        .collect::<Vec<_>>();

    if positive_shifts.is_empty() {
        return None;
    }

    let mean_shift = positive_shifts.iter().sum::<f32>() / positive_shifts.len() as f32;
    let max_shift = positive_shifts.iter().copied().fold(0.0, f32::max);
    if mean_shift < FLATTEN_MIN_MEAN_SHIFT_PX || max_shift < FLATTEN_MIN_MAX_SHIFT_PX {
        return None;
    }

    let coverage = positive_shifts.len() as f32 / smoothed_shifts.len() as f32;
    Some(FlattenCandidate {
        side,
        row_shifts: smoothed_shifts,
        max_shift,
        score: mean_shift * (1.0 + coverage),
        edge_anchor_ratio,
    })
}

fn detect_edge_offset_for_row(
    gray: &GrayImage,
    row: u32,
    side: FlattenSide,
    band_width: usize,
) -> Option<f32> {
    let mut row_darkness = 0.0;
    for x in 0..gray.width() {
        row_darkness += 255.0 - gray.get_pixel(x, row).0[0] as f32;
    }
    let threshold = 24.0f32.max((row_darkness / gray.width() as f32) * 2.4);

    match side {
        FlattenSide::Left => {
            for x in 0..band_width.min(gray.width() as usize) {
                if compute_darkness_window(gray, x as i32, row as i32) >= threshold {
                    return Some(x as f32);
                }
            }
        }
        FlattenSide::Right => {
            for offset in 0..band_width.min(gray.width() as usize) {
                let x = gray.width() as i32 - 1 - offset as i32;
                if compute_darkness_window(gray, x, row as i32) >= threshold {
                    return Some(offset as f32);
                }
            }
        }
    }

    None
}

fn compute_darkness_window(gray: &GrayImage, x: i32, y: i32) -> f32 {
    let mut darkness = 0.0;
    let mut samples = 0.0;
    for sample_y in (y - 1).max(0)..=(y + 1).min(gray.height() as i32 - 1) {
        for sample_x in (x - 1).max(0)..=(x + 1).min(gray.width() as i32 - 1) {
            darkness += 255.0 - gray.get_pixel(sample_x as u32, sample_y as u32).0[0] as f32;
            samples += 1.0;
        }
    }

    if samples <= 0.0 {
        0.0
    } else {
        darkness / samples
    }
}

fn percentile(values: &[f32], ratio: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|first, second| {
        first
            .partial_cmp(second)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let index = ((sorted.len() - 1) as f32 * ratio)
        .floor()
        .clamp(0.0, (sorted.len() - 1) as f32) as usize;
    sorted[index]
}

fn fill_missing_offsets(values: &[Option<f32>], fallback: f32) -> Vec<f32> {
    let mut result = values
        .iter()
        .map(|value| value.unwrap_or(f32::NAN))
        .collect::<Vec<_>>();
    let mut last_known_index = None;

    for index in 0..result.len() {
        if !result[index].is_nan() {
            if let Some(previous_index) = last_known_index {
                if index > previous_index + 1 {
                    let start_value = result[previous_index];
                    let end_value = result[index];
                    let gap = (index - previous_index) as f32;
                    for fill_index in (previous_index + 1)..index {
                        let progress = (fill_index - previous_index) as f32 / gap;
                        result[fill_index] = start_value + ((end_value - start_value) * progress);
                    }
                }
            } else {
                for fill_index in 0..index {
                    result[fill_index] = result[index];
                }
            }
            last_known_index = Some(index);
        }
    }

    if let Some(last_known_index) = last_known_index {
        for index in (last_known_index + 1)..result.len() {
            result[index] = result[last_known_index];
        }
    }

    result
        .into_iter()
        .map(|value| if value.is_nan() { fallback } else { value })
        .collect()
}

fn smooth_values(values: &[f32], radius: usize) -> Vec<f32> {
    (0..values.len())
        .map(|index| {
            let start = index.saturating_sub(radius);
            let end = (index + radius).min(values.len().saturating_sub(1));
            let window = &values[start..=end];
            window.iter().sum::<f32>() / window.len() as f32
        })
        .collect()
}

fn apply_flatten_warp(source: &RgbaImage, side: FlattenSide, row_shifts: &[f32]) -> RgbaImage {
    let apply_band_width = ((source.width() as f32 * FLATTEN_APPLY_BAND_RATIO).round() as usize)
        .clamp(
            ((source.width() as f32 * 0.12).floor() as usize).max(FLATTEN_MIN_BAND_PX),
            ((source.width() as f32 * FLATTEN_MAX_BAND_RATIO).floor() as usize)
                .max(FLATTEN_MIN_BAND_PX),
        )
        .max(1);
    let mut output = RgbaImage::new(source.width(), source.height());

    for y in 0..source.height() as usize {
        let shift = row_shifts.get(y).copied().unwrap_or(0.0);
        for x in 0..source.width() as usize {
            let distance_into_band = match side {
                FlattenSide::Left => x as f32,
                FlattenSide::Right => (source.width() as usize - 1 - x) as f32,
            };
            let weight = (1.0 - (distance_into_band / apply_band_width as f32)).clamp(0.0, 1.0);
            let eased_weight = weight * weight;
            let source_x = match side {
                FlattenSide::Left => x as f32 + (shift * eased_weight),
                FlattenSide::Right => x as f32 - (shift * eased_weight),
            };
            output.put_pixel(
                x as u32,
                y as u32,
                sample_rgba_bilinear(source, source_x, y as f32),
            );
        }
    }

    output
}

fn sample_rgba_bilinear(source: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    let clamped_x = x.clamp(0.0, source.width().saturating_sub(1) as f32);
    let clamped_y = y.clamp(0.0, source.height().saturating_sub(1) as f32);
    let x0 = clamped_x.floor() as u32;
    let y0 = clamped_y.floor() as u32;
    let x1 = x0.saturating_add(1).min(source.width().saturating_sub(1));
    let y1 = y0.saturating_add(1).min(source.height().saturating_sub(1));
    let tx = clamped_x - x0 as f32;
    let ty = clamped_y - y0 as f32;

    let top_left = source.get_pixel(x0, y0).0;
    let top_right = source.get_pixel(x1, y0).0;
    let bottom_left = source.get_pixel(x0, y1).0;
    let bottom_right = source.get_pixel(x1, y1).0;
    let mut output = [0u8; 4];

    for channel in 0..4 {
        let top = top_left[channel] as f32
            + ((top_right[channel] as f32 - top_left[channel] as f32) * tx);
        let bottom = bottom_left[channel] as f32
            + ((bottom_right[channel] as f32 - bottom_left[channel] as f32) * tx);
        output[channel] = (top + ((bottom - top) * ty)).round().clamp(0.0, 255.0) as u8;
    }

    Rgba(output)
}

fn index_of_min(values: &[f32; 4]) -> usize {
    let mut best_index = 0usize;
    for index in 1..values.len() {
        if values[index] < values[best_index] {
            best_index = index;
        }
    }
    best_index
}

fn index_of_max(values: &[f32; 4]) -> usize {
    let mut best_index = 0usize;
    for index in 1..values.len() {
        if values[index] > values[best_index] {
            best_index = index;
        }
    }
    best_index
}

fn warp_document_to_rect(
    source: &RgbaImage,
    points: &[ScannerPoint; 4],
) -> Result<RgbaImage, String> {
    let [tl, tr, br, bl] = points;
    let width_top = point_distance(*tl, *tr);
    let width_bottom = point_distance(*bl, *br);
    let max_width = width_top.max(width_bottom).round().max(1.0) as u32;

    let height_left = point_distance(*tl, *bl);
    let height_right = point_distance(*tr, *br);
    let max_height = height_left.max(height_right).round().max(1.0) as u32;

    let from = [(tl.x, tl.y), (tr.x, tr.y), (br.x, br.y), (bl.x, bl.y)];
    let to = [
        (0.0f32, 0.0f32),
        (max_width.saturating_sub(1) as f32, 0.0f32),
        (
            max_width.saturating_sub(1) as f32,
            max_height.saturating_sub(1) as f32,
        ),
        (0.0f32, max_height.saturating_sub(1) as f32),
    ];

    let projection = Projection::from_control_points(from, to).ok_or_else(|| {
        "Failed to build perspective projection from document points.".to_string()
    })?;

    let mut output = RgbaImage::new(max_width, max_height);
    warp_into(
        source,
        &projection,
        Interpolation::Bilinear,
        Rgba([255, 255, 255, 255]),
        &mut output,
    );
    Ok(output)
}

fn point_distance(a: ScannerPoint, b: ScannerPoint) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

fn enhance_document_image(source: &RgbaImage, prefer_soft_tone: bool) -> RgbaImage {
    let gray = DynamicImage::ImageRgba8(source.clone()).into_luma8();
    let background_sigma = compute_background_sigma(source.width(), source.height());
    let background = gaussian_blur_f32(&gray, background_sigma);
    let flattened = flatten_background(&gray, &background);
    let denoised = gaussian_blur_f32(&flattened, 0.8);
    let normalized = normalize_gray(&denoised);
    if should_prefer_soft_tone(source, prefer_soft_tone) {
        return gray_to_rgba(&normalized);
    }
    let threshold = otsu_level(&normalized);
    let binary = threshold_to_binary(&normalized, threshold);

    let selected = if is_reasonable_binary_candidate(&binary) {
        binary
    } else {
        normalized
    };

    gray_to_rgba(&selected)
}

fn should_prefer_soft_tone(source: &RgbaImage, prefer_soft_tone: bool) -> bool {
    if prefer_soft_tone {
        return true;
    }

    let Some(paper_bbox_ratio) = measure_paper_bbox_ratio(source) else {
        return true;
    };

    paper_bbox_ratio < ENHANCE_MIN_PAPER_BBOX_RATIO
}

fn measure_paper_bbox_ratio(source: &RgbaImage) -> Option<f32> {
    let score_image = build_paper_score_image(source);
    let threshold = otsu_level(&score_image).max(PAPER_CROP_MIN_SCORE_THRESHOLD);
    let bounding_box = largest_paper_component_bbox(&score_image, threshold)?;
    let image_area = (source.width() as f32 * source.height() as f32).max(1.0);
    let bbox_area = bounding_box.width() as f32 * bounding_box.height() as f32;
    Some(bbox_area / image_area)
}

fn compute_background_sigma(width: u32, height: u32) -> f32 {
    let shortest_side = width.min(height) as f32;
    (shortest_side * 0.04).clamp(3.0, 18.0)
}

fn flatten_background(gray: &GrayImage, background: &GrayImage) -> GrayImage {
    let mut output = GrayImage::new(gray.width(), gray.height());

    for y in 0..gray.height() {
        for x in 0..gray.width() {
            let luminance = gray.get_pixel(x, y).0[0] as f32;
            let blurred = (background.get_pixel(x, y).0[0] as f32).max(1.0);
            let value = ((luminance / blurred) * 255.0).clamp(0.0, 255.0) as u8;
            output.put_pixel(x, y, Luma([value]));
        }
    }

    output
}

fn normalize_gray(input: &GrayImage) -> GrayImage {
    let mut min_value = u8::MAX;
    let mut max_value = u8::MIN;

    for pixel in input.pixels() {
        let value = pixel.0[0];
        if value < min_value {
            min_value = value;
        }
        if value > max_value {
            max_value = value;
        }
    }

    let range = (max_value.saturating_sub(min_value)).max(1) as f32;
    let mut output = GrayImage::new(input.width(), input.height());

    for (x, y, pixel) in input.enumerate_pixels() {
        let normalized = (((pixel.0[0].saturating_sub(min_value)) as f32) * 255.0 / range)
            .clamp(0.0, 255.0) as u8;
        output.put_pixel(x, y, Luma([normalized]));
    }

    output
}

fn threshold_to_binary(input: &GrayImage, threshold: u8) -> GrayImage {
    let mut output = GrayImage::new(input.width(), input.height());

    for (x, y, pixel) in input.enumerate_pixels() {
        let value = if pixel.0[0] > threshold { 255 } else { 0 };
        output.put_pixel(x, y, Luma([value]));
    }

    output
}

fn is_reasonable_binary_candidate(binary: &GrayImage) -> bool {
    let total_pixels = (binary.width() as u64 * binary.height() as u64).max(1) as f64;
    let white_pixels = binary.pixels().filter(|pixel| pixel.0[0] > 127).count() as f64;
    let white_ratio = white_pixels / total_pixels;
    let black_ratio = 1.0 - white_ratio;

    white_ratio >= 0.45 && white_ratio <= 0.995 && black_ratio >= 0.005
}

fn gray_to_rgba(source: &GrayImage) -> RgbaImage {
    let mut output = RgbaImage::new(source.width(), source.height());

    for (x, y, pixel) in source.enumerate_pixels() {
        let value = pixel.0[0];
        output.put_pixel(x, y, Rgba([value, value, value, 255]));
    }

    output
}

fn rotate_image(image: RgbaImage, rotation: u16) -> Result<RgbaImage, String> {
    match rotation {
        0 => Ok(image),
        90 => Ok(rotate90(&image)),
        180 => Ok(rotate180(&image)),
        270 => Ok(rotate270(&image)),
        other => Err(format!("Unsupported output rotation: {other}")),
    }
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut cursor = Cursor::new(Vec::new());
    let encoder = PngEncoder::new(&mut cursor);
    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ColorType::Rgba8.into(),
        )
        .map_err(|error| format!("Failed to encode PNG payload: {error}"))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn orders_points_into_tl_tr_br_bl() {
        let ordered = order_points([
            ScannerPoint { x: 90.0, y: 10.0 },
            ScannerPoint { x: 10.0, y: 90.0 },
            ScannerPoint { x: 10.0, y: 10.0 },
            ScannerPoint { x: 90.0, y: 90.0 },
        ]);

        assert_eq!(ordered[0].x, 10.0);
        assert_eq!(ordered[0].y, 10.0);
        assert_eq!(ordered[1].x, 90.0);
        assert_eq!(ordered[1].y, 10.0);
        assert_eq!(ordered[2].x, 90.0);
        assert_eq!(ordered[2].y, 90.0);
        assert_eq!(ordered[3].x, 10.0);
        assert_eq!(ordered[3].y, 90.0);
    }

    #[test]
    fn rotates_rgba_images_orthogonally() {
        let mut image = RgbaImage::new(2, 3);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        image.put_pixel(1, 0, Rgba([0, 255, 0, 255]));
        image.put_pixel(0, 2, Rgba([0, 0, 255, 255]));

        let rotated = rotate_image(image, 90).expect("rotation should succeed");
        assert_eq!(rotated.width(), 3);
        assert_eq!(rotated.height(), 2);
        assert_eq!(rotated.get_pixel(2, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn encodes_processed_image_as_png() {
        let mut image = RgbaImage::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                let value = if x < 16 { 24 } else { 232 };
                image.put_pixel(x, y, Rgba([value, value, value, 255]));
            }
        }

        let enhanced = enhance_document_image(&image, false);
        let encoded = encode_png(&enhanced).expect("png encoding should succeed");
        assert!(encoded.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn refines_coarse_quad_toward_document_edges() {
        let mut image = RgbaImage::from_pixel(160, 100, Rgba([255, 255, 255, 255]));
        for x in 20..140 {
            for thickness in 0..3 {
                image.put_pixel(x, 15 + thickness, Rgba([0, 0, 0, 255]));
                image.put_pixel(x, 84 - thickness, Rgba([0, 0, 0, 255]));
            }
        }
        for y in 15..85 {
            for thickness in 0..3 {
                image.put_pixel(20 + thickness, y, Rgba([0, 0, 0, 255]));
                image.put_pixel(139 - thickness, y, Rgba([0, 0, 0, 255]));
            }
        }

        let coarse = order_points([
            ScannerPoint { x: 26.0, y: 19.0 },
            ScannerPoint { x: 133.0, y: 22.0 },
            ScannerPoint { x: 136.0, y: 79.0 },
            ScannerPoint { x: 24.0, y: 81.0 },
        ]);
        let refined = refine_document_points(&image, &coarse);
        let expected = order_points([
            ScannerPoint { x: 20.0, y: 15.0 },
            ScannerPoint { x: 139.0, y: 15.0 },
            ScannerPoint { x: 139.0, y: 84.0 },
            ScannerPoint { x: 20.0, y: 84.0 },
        ]);

        for index in 0..4 {
            let coarse_error = point_distance(coarse[index], expected[index]);
            let refined_error = point_distance(refined[index], expected[index]);
            assert!(
                refined_error < coarse_error,
                "refined point {index} should move closer to the expected corner: coarse={coarse_error}, refined={refined_error}",
            );
        }
        assert!(refinement_applied_between(&coarse, &refined));
    }

    #[test]
    fn refinement_prefers_document_edge_over_external_strong_line() {
        let mut image = RgbaImage::from_pixel(220, 180, Rgba([150, 135, 120, 255]));
        for x in 12..208 {
            for thickness in 0..3 {
                image.put_pixel(x, 18 + thickness, Rgba([8, 8, 8, 255]));
            }
        }

        for y in 34..160 {
            for x in 28..188 {
                image.put_pixel(x, y, Rgba([248, 248, 248, 255]));
            }
        }
        for x in 28..188 {
            for thickness in 0..2 {
                image.put_pixel(x, 34 + thickness, Rgba([32, 32, 32, 255]));
                image.put_pixel(x, 159 - thickness, Rgba([32, 32, 32, 255]));
            }
        }
        for y in 34..160 {
            for thickness in 0..2 {
                image.put_pixel(28 + thickness, y, Rgba([32, 32, 32, 255]));
                image.put_pixel(187 - thickness, y, Rgba([32, 32, 32, 255]));
            }
        }

        let coarse = order_points([
            ScannerPoint { x: 36.0, y: 26.0 },
            ScannerPoint { x: 182.0, y: 24.0 },
            ScannerPoint { x: 186.0, y: 156.0 },
            ScannerPoint { x: 30.0, y: 158.0 },
        ]);
        let refined = refine_document_points(&image, &coarse);

        assert!(refined[0].y > 30.0);
        assert!(refined[1].y > 30.0);
        assert!(refined[0].x < 34.0);
        assert!(refined[1].x > 184.0);
    }

    #[test]
    fn paper_crop_removes_large_background_border() {
        let mut image = RgbaImage::from_pixel(220, 180, Rgba([180, 145, 130, 255]));
        for y in 28..166 {
            for x in 34..194 {
                image.put_pixel(x, y, Rgba([246, 246, 244, 255]));
            }
        }
        for y in 52..150 {
            for x in 56..172 {
                if (x + y) % 11 == 0 {
                    image.put_pixel(x, y, Rgba([24, 24, 24, 255]));
                }
            }
        }

        let cropped = crop_to_paper_region(&image);
        assert!(cropped.applied);
        assert!(cropped.image.width() < image.width());
        assert!(cropped.image.height() < image.height());
        assert!(cropped.image.width() > 140);
        assert!(cropped.image.height() > 120);
    }

    #[test]
    fn paper_crop_rejects_bright_structured_top_band() {
        let mut image = RgbaImage::from_pixel(320, 420, Rgba([206, 164, 142, 255]));
        for y in 0..74 {
            for x in 18..302 {
                let pixel = if ((x / 18) + (y / 10)) % 2 == 0 {
                    Rgba([236, 232, 226, 255])
                } else {
                    Rgba([194, 146, 98, 255])
                };
                image.put_pixel(x, y, pixel);
            }
        }
        for y in 92..392 {
            for x in 26..290 {
                image.put_pixel(x, y, Rgba([246, 246, 242, 255]));
            }
        }
        for y in 128..360 {
            for x in 54..262 {
                if (x + y) % 17 == 0 {
                    image.put_pixel(x, y, Rgba([36, 36, 36, 255]));
                }
            }
        }

        let cropped = crop_to_paper_region(&image);
        assert!(cropped.applied);
        assert!(cropped.image.width() < 320);
        assert!(cropped.image.width() > 240);
        assert!(cropped.image.height() < 360);
        assert!(cropped.image.height() > 260);
    }

    #[test]
    fn process_image_request_applies_paper_crop_for_background_heavy_scene() {
        let mut image = RgbaImage::from_pixel(220, 180, Rgba([180, 145, 130, 255]));
        for y in 28..166 {
            for x in 34..194 {
                image.put_pixel(x, y, Rgba([246, 246, 244, 255]));
            }
        }
        for y in 52..150 {
            for x in 56..172 {
                if (x + y) % 11 == 0 {
                    image.put_pixel(x, y, Rgba([24, 24, 24, 255]));
                }
            }
        }

        let source_bytes = encode_png(&image).expect("source png encoding should succeed");
        let (response, _encoded) = process_image_request(ScannerPostProcessRequest {
            source_bytes,
            document_points: Some(vec![
                ScannerPoint { x: 0.0, y: 0.0 },
                ScannerPoint { x: 219.0, y: 0.0 },
                ScannerPoint { x: 219.0, y: 179.0 },
                ScannerPoint { x: 0.0, y: 179.0 },
            ]),
            output_rotation: 0,
            image_enhancement: false,
        })
        .expect("background-heavy post-process should succeed");

        assert!(
            response.paper_crop_applied,
            "background-heavy scenes should crop the paper region before enhancement",
        );
        assert!(response.crop_ms.is_some());
        assert!(response.output_width < 220);
        assert!(response.output_height < 180);
    }

    #[test]
    fn local_flattening_reduces_single_side_boundary_variation() {
        let mut image = RgbaImage::from_pixel(120, 90, Rgba([255, 255, 255, 255]));
        for y in 0..90 {
            let left_inset = 8 + ((y as f32 / 89.0) * 10.0).round() as u32;
            for x in left_inset..54 {
                image.put_pixel(x, y, Rgba([24, 24, 24, 255]));
            }
        }

        let before_gray = DynamicImage::ImageRgba8(image.clone()).into_luma8();
        let before_offsets = (0..before_gray.height())
            .filter_map(|row| detect_edge_offset_for_row(&before_gray, row, FlattenSide::Left, 28))
            .collect::<Vec<_>>();
        let before_spread = before_offsets.iter().copied().fold(f32::MIN, f32::max)
            - before_offsets.iter().copied().fold(f32::MAX, f32::min);

        let flattened = apply_local_spine_flattening(&image);
        assert!(flattened.applied);

        let after_gray = DynamicImage::ImageRgba8(flattened.image).into_luma8();
        let after_offsets = (0..after_gray.height())
            .filter_map(|row| detect_edge_offset_for_row(&after_gray, row, FlattenSide::Left, 28))
            .collect::<Vec<_>>();
        let after_spread = after_offsets.iter().copied().fold(f32::MIN, f32::max)
            - after_offsets.iter().copied().fold(f32::MAX, f32::min);

        assert!(after_spread < before_spread);
    }

    #[test]
    fn local_flattening_skips_centered_document_content() {
        let mut image = RgbaImage::from_pixel(160, 120, Rgba([255, 255, 255, 255]));
        for y in 18..102 {
            for x in 34..126 {
                if (x + (y * 3)) % 9 == 0 {
                    image.put_pixel(x, y, Rgba([18, 18, 18, 255]));
                }
            }
        }

        let flattened = apply_local_spine_flattening(&image);
        assert!(
            !flattened.applied,
            "centered single-sheet content should not be mistaken for a one-sided gutter case",
        );
    }

    #[test]
    fn skips_paper_crop_after_local_flattening() {
        let mut image = RgbaImage::from_pixel(120, 90, Rgba([255, 255, 255, 255]));
        for y in 0..90 {
            let left_inset = 8 + ((y as f32 / 89.0) * 10.0).round() as u32;
            for x in left_inset..54 {
                image.put_pixel(x, y, Rgba([20, 20, 20, 255]));
            }
        }

        let source_bytes = encode_png(&image).expect("source png encoding should succeed");
        let (response, _encoded) = process_image_request(ScannerPostProcessRequest {
            source_bytes,
            document_points: Some(vec![
                ScannerPoint { x: 0.0, y: 0.0 },
                ScannerPoint { x: 119.0, y: 0.0 },
                ScannerPoint { x: 119.0, y: 89.0 },
                ScannerPoint { x: 0.0, y: 89.0 },
            ]),
            output_rotation: 0,
            image_enhancement: false,
        })
        .expect("book-like post-process should succeed");

        assert!(response.local_flattening_applied);
        assert!(
            !response.paper_crop_applied,
            "paper crop must not amputate the edge that local flattening is trying to preserve",
        );
        assert!(response.crop_ms.is_none());
    }

    #[test]
    fn local_flattening_prefers_soft_tone_enhancement() {
        let mut image = RgbaImage::from_pixel(120, 90, Rgba([255, 255, 255, 255]));
        for y in 0..90 {
            let left_inset = 8 + ((y as f32 / 89.0) * 10.0).round() as u32;
            for x in left_inset..54 {
                let shade = 28 + ((x - left_inset) % 24) as u8;
                image.put_pixel(x, y, Rgba([shade, shade, shade, 255]));
            }
        }

        let source_bytes = encode_png(&image).expect("source png encoding should succeed");
        let (response, encoded) = process_image_request(ScannerPostProcessRequest {
            source_bytes,
            document_points: Some(vec![
                ScannerPoint { x: 0.0, y: 0.0 },
                ScannerPoint { x: 119.0, y: 0.0 },
                ScannerPoint { x: 119.0, y: 89.0 },
                ScannerPoint { x: 0.0, y: 89.0 },
            ]),
            output_rotation: 0,
            image_enhancement: true,
        })
        .expect("book-like enhancement should succeed");

        assert!(response.local_flattening_applied);

        let decoded = image::load_from_memory(&encoded)
            .expect("enhanced output should decode")
            .into_luma8();
        let mut seen = [false; 256];
        let mut unique_values = 0usize;
        for value in decoded.pixels().map(|pixel| pixel.0[0]) {
            let slot = &mut seen[value as usize];
            if !*slot {
                *slot = true;
                unique_values += 1;
            }
        }

        assert!(
            unique_values > 2,
            "soft-tone enhancement should preserve grayscale detail for locally flattened book pages",
        );
    }

    #[test]
    fn background_heavy_scene_prefers_soft_tone_enhancement() {
        let mut image = RgbaImage::from_pixel(180, 120, Rgba([224, 192, 180, 255]));
        for y in 14..112 {
            for x in 34..150 {
                image.put_pixel(x, y, Rgba([242, 242, 238, 255]));
            }
        }
        for y in 0..20 {
            for x in 0..180 {
                image.put_pixel(x, y, Rgba([197, 155, 120, 255]));
            }
        }
        for y in 36..96 {
            for x in 44..140 {
                if (x + (y * 2)) % 11 == 0 {
                    image.put_pixel(x, y, Rgba([72, 72, 72, 255]));
                }
            }
        }

        let enhanced = enhance_document_image(&image, false);
        let decoded = DynamicImage::ImageRgba8(enhanced).into_luma8();
        let mut seen = [false; 256];
        let mut unique_values = 0usize;
        for value in decoded.pixels().map(|pixel| pixel.0[0]) {
            let slot = &mut seen[value as usize];
            if !*slot {
                *slot = true;
                unique_values += 1;
            }
        }

        assert!(
            unique_values > 2,
            "background-heavy book/page scenes should stay in grayscale instead of collapsing into binary",
        );
    }

    #[test]
    fn page_dominant_scene_keeps_binary_enhancement() {
        let mut image = RgbaImage::from_pixel(160, 120, Rgba([246, 246, 244, 255]));
        for y in 18..102 {
            for x in 22..138 {
                if (x + y) % 9 == 0 {
                    image.put_pixel(x, y, Rgba([24, 24, 24, 255]));
                }
            }
        }

        let enhanced = enhance_document_image(&image, false);
        let decoded = DynamicImage::ImageRgba8(enhanced).into_luma8();
        let mut seen = [false; 256];
        let mut unique_values = 0usize;
        for value in decoded.pixels().map(|pixel| pixel.0[0]) {
            let slot = &mut seen[value as usize];
            if !*slot {
                *slot = true;
                unique_values += 1;
            }
        }

        assert!(
            unique_values <= 2,
            "page-dominant single-sheet scenes should still use binary enhancement",
        );
    }

    #[test]
    fn rotates_output_before_local_flattening_for_portrait_exports() {
        let mut image = RgbaImage::from_pixel(120, 90, Rgba([255, 255, 255, 255]));
        for y in 0..90 {
            let left_inset = 8 + ((y as f32 / 89.0) * 10.0).round() as u32;
            for x in left_inset..110 {
                image.put_pixel(x, y, Rgba([18, 18, 18, 255]));
            }
        }

        let source_bytes = encode_png(&image).expect("source png encoding should succeed");
        let (response, _encoded) = process_image_request(ScannerPostProcessRequest {
            source_bytes,
            document_points: Some(vec![
                ScannerPoint { x: 0.0, y: 0.0 },
                ScannerPoint { x: 119.0, y: 0.0 },
                ScannerPoint { x: 119.0, y: 89.0 },
                ScannerPoint { x: 0.0, y: 89.0 },
            ]),
            output_rotation: 90,
            image_enhancement: false,
        })
        .expect("portrait export should succeed");

        assert!(
            !response.local_flattening_applied,
            "rotation must happen before flattening so a top edge is not treated as a spine edge",
        );
        assert!(response.rotate_ms.is_some());
    }

    #[test]
    fn processes_encoded_source_bytes_end_to_end() {
        let mut image = RgbaImage::new(40, 24);
        for y in 0..24 {
            for x in 0..40 {
                let pixel = if x < 20 {
                    Rgba([32, 32, 32, 255])
                } else {
                    Rgba([224, 224, 224, 255])
                };
                image.put_pixel(x, y, pixel);
            }
        }

        let source_bytes = encode_png(&image).expect("source png encoding should succeed");
        let (response, encoded) = process_image_request(ScannerPostProcessRequest {
            source_bytes,
            document_points: Some(vec![
                ScannerPoint { x: 4.0, y: 2.0 },
                ScannerPoint { x: 35.0, y: 2.0 },
                ScannerPoint { x: 35.0, y: 21.0 },
                ScannerPoint { x: 4.0, y: 21.0 },
            ]),
            output_rotation: 90,
            image_enhancement: true,
        })
        .expect("post-process request should succeed");

        assert!(encoded.starts_with(&[0x89, b'P', b'N', b'G']));
        assert_eq!(response.input_width, 40);
        assert_eq!(response.input_height, 24);
        assert!(response.output_width >= 19);
        assert!(response.output_height >= 19);
        assert!(response.refine_ms.is_some());
        assert!(response.perspective_ms.is_some());
        assert!(response.flatten_ms.is_some());
        assert!(response.enhance_ms.is_some());
        assert!(response.rotate_ms.is_some());
        assert!(response.effective_document_points.is_some());
        assert!(response.refinement_applied);
    }

    #[test]
    fn rejects_degenerate_edge_ratio_quad() {
        // 5af-native style: top edge ~135px, bottom ~1554px, ratio 0.07
        let points = [
            ScannerPoint { x: 1468.0, y: 7.0 },
            ScannerPoint { x: 1603.0, y: 19.0 },
            ScannerPoint { x: 1568.0, y: 2321.0 },
            ScannerPoint { x: 14.0, y: 2533.0 },
        ];
        let result = validate_quad_geometry(&points, 1604, 2549);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("horizontal edge ratio"));
    }

    #[test]
    fn rejects_tiny_area_quad() {
        let points = [
            ScannerPoint { x: 100.0, y: 100.0 },
            ScannerPoint { x: 110.0, y: 100.0 },
            ScannerPoint { x: 110.0, y: 110.0 },
            ScannerPoint { x: 100.0, y: 110.0 },
        ];
        let result = validate_quad_geometry(&points, 1000, 1000);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too small"));
    }

    #[test]
    fn rejects_non_convex_quad() {
        // Concave quad: BR vertex pushed far inward past the diagonal
        let points = [
            ScannerPoint { x: 10.0, y: 10.0 },
            ScannerPoint { x: 90.0, y: 10.0 },
            ScannerPoint { x: 30.0, y: 30.0 }, // pushed deep inside — concave
            ScannerPoint { x: 10.0, y: 90.0 },
        ];
        let result = validate_quad_geometry(&points, 100, 100);
        assert!(result.is_err(), "expected error, got: {:?}", result);
    }

    #[test]
    fn accepts_valid_quad() {
        let points = [
            ScannerPoint { x: 10.0, y: 10.0 },
            ScannerPoint { x: 90.0, y: 10.0 },
            ScannerPoint { x: 90.0, y: 90.0 },
            ScannerPoint { x: 10.0, y: 90.0 },
        ];
        let result = validate_quad_geometry(&points, 100, 100);
        assert!(result.is_ok());
    }
}
