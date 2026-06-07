/// Native contour-based document detection using `imageproc`.
///
/// This is a faithful Rust port of the legacy TypeScript `document-detector.ts`
/// which ran inside a Web Worker backed by OpenCV.js.  The pipeline is:
///
/// 1. Optional downscale to `max_width × max_height`
/// 2. RGBA → Grayscale → Gaussian Blur (5×5)
/// 3. Canny edge detection (30/90)
/// 4. Dilate (3×3) + Morphological Close (5×5) to bridge gaps
/// 5. Find external contours
/// 6. Filter by area (4%–98%), sort descending
/// 7. For top-15 contours, try Douglas-Peucker approximation with 5 ε-factors
/// 8. Score each 4-point convex candidate (angles, extent, coverage, balance, area, border)
/// 9. Return the best quad ordered TL → TR → BR → BL, or `None`
use image::DynamicImage;
use imageproc::contours::{find_contours_with_threshold, BorderType};
use imageproc::distance_transform::Norm;
use imageproc::edges::canny;
use imageproc::filter::gaussian_blur_f32;
use imageproc::morphology::{close, dilate};

use crate::scanner_detect::ScannerPoint;
use crate::stream_decoder::get_latest_preview_frame_packet;

// ---------------------------------------------------------------------------
// Constants (ported 1:1 from legacy document-detector.ts)
// ---------------------------------------------------------------------------

const CONTOUR_AREA_MIN_RATIO: f64 = 0.04;
const CONTOUR_AREA_MAX_RATIO: f64 = 0.98;
const APPROXIMATION_EPSILON_FACTORS: &[f64] = &[0.015, 0.02, 0.03, 0.04, 0.05];
const MAX_SCORING_CONTOURS: usize = 15;
const MIN_ACCEPTABLE_QUAD_SCORE: f64 = 1.5;
const BORDER_TOUCH_MARGIN_RATIO: f64 = 0.02;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run contour-based document detection on the latest cached preview frame.
///
/// Returns `(points, processing_ms, frame_width, frame_height, message)`.
pub fn detect_document_opencv(
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> (Option<Vec<ScannerPoint>>, f64, u32, u32, String) {
    let started = std::time::Instant::now();

    let packet = match get_latest_preview_frame_packet() {
        Some(p) => p,
        None => {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            return (
                None,
                ms,
                0,
                0,
                "No preview frame available for OpenCV detection.".to_string(),
            );
        }
    };

    let dynamic_image =
        match crate::scanner_detect::build_dynamic_image_from_preview_frame_packet(&packet) {
            Ok(img) => img,
            Err(e) => {
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                return (
                    None,
                    ms,
                    0,
                    0,
                    format!("Failed to decode preview frame for OpenCV detection: {e}"),
                );
            }
        };

    let original_width = dynamic_image.width();
    let original_height = dynamic_image.height();

    let points = detect_contour_quad(&dynamic_image, max_width, max_height);

    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let message = if points.is_some() {
        format!("OpenCV contour detection completed in {ms:.1}ms.")
    } else {
        format!("No document detected via OpenCV contours ({ms:.1}ms).")
    };

    (points, ms, original_width, original_height, message)
}

/// Run contour detection on an already-loaded `DynamicImage`.
/// Useful for bin tests and unit tests.
pub fn detect_contour_quad(
    source: &DynamicImage,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> Option<Vec<ScannerPoint>> {
    let original_w = source.width();
    let original_h = source.height();

    // Step 1: Optional downscale.
    let (working, scale_x, scale_y) = downscale_if_needed(source, max_width, max_height);
    let working_w = working.width();
    let working_h = working.height();

    // Step 2: Grayscale.
    let gray = image::imageops::grayscale(&working);

    // Step 3: Gaussian blur (σ ≈ 1.0 with 5×5 kernel equivalent).
    let blurred = gaussian_blur_f32(&gray, 1.0);

    // Step 4: Canny edge detection (low=30, high=90).
    let edges = canny(&blurred, 30.0, 90.0);

    // Step 5: Dilate (3×3) then morphological close (5×5).
    let dilated = dilate(&edges, Norm::LInf, 1); // 3×3 square
    let closed = close(&dilated, Norm::LInf, 2); // 5×5 square

    // Step 6: Find contours.
    let contours = find_contours_with_threshold::<i32>(&closed, 128);

    // Step 7: Filter by area and sort.
    let frame_area = (working_w as f64) * (working_h as f64);
    let min_area = frame_area * CONTOUR_AREA_MIN_RATIO;
    let max_area = frame_area * CONTOUR_AREA_MAX_RATIO;

    let mut scored_contours: Vec<(usize, f64)> = contours
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            if c.border_type == BorderType::Hole {
                return None;
            }
            let area = contour_area(&c.points);
            if area < min_area || area > max_area {
                return None;
            }
            Some((i, area))
        })
        .collect();

    scored_contours.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored_contours.truncate(MAX_SCORING_CONTOURS);

    // Step 8: Try polygon approximation on top contours.
    let mut best_score = f64::NEG_INFINITY;
    let mut best_points: Option<[Point; 4]> = None;

    for &(contour_idx, contour_area_val) in &scored_contours {
        let contour = &contours[contour_idx];
        let perimeter = contour_perimeter(&contour.points);

        for &epsilon_factor in APPROXIMATION_EPSILON_FACTORS {
            let epsilon = epsilon_factor * perimeter;
            let approx = douglas_peucker(&contour.points, epsilon);

            if approx.len() == 4 && is_convex_polygon(&approx) {
                let candidate: [Point; 4] = [
                    Point {
                        x: approx[0].x as f64 * scale_x,
                        y: approx[0].y as f64 * scale_y,
                    },
                    Point {
                        x: approx[1].x as f64 * scale_x,
                        y: approx[1].y as f64 * scale_y,
                    },
                    Point {
                        x: approx[2].x as f64 * scale_x,
                        y: approx[2].y as f64 * scale_y,
                    },
                    Point {
                        x: approx[3].x as f64 * scale_x,
                        y: approx[3].y as f64 * scale_y,
                    },
                ];

                let ordered = order_points(&candidate);
                let score = compute_quad_score(
                    &ordered,
                    contour_area_val * scale_x * scale_y,
                    original_w as f64,
                    original_h as f64,
                );

                if score > best_score {
                    best_score = score;
                    best_points = Some(ordered);
                }
            }
        }
    }

    if best_score < MIN_ACCEPTABLE_QUAD_SCORE {
        return None;
    }

    best_points.map(|pts| {
        pts.iter()
            .map(|p| ScannerPoint {
                x: p.x as f32,
                y: p.y as f32,
            })
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Internal geometry types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

// ---------------------------------------------------------------------------
// Downscale
// ---------------------------------------------------------------------------

fn downscale_if_needed(
    source: &DynamicImage,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> (DynamicImage, f64, f64) {
    let w = source.width();
    let h = source.height();
    let max_w = max_width.unwrap_or(w).max(1);
    let max_h = max_height.unwrap_or(h).max(1);

    let scale = (max_w as f64 / w.max(1) as f64)
        .min(max_h as f64 / h.max(1) as f64)
        .min(1.0);

    if scale >= 1.0 {
        return (source.clone(), 1.0, 1.0);
    }

    let target_w = (w as f64 * scale).round().max(1.0) as u32;
    let target_h = (h as f64 * scale).round().max(1.0) as u32;
    let resized = source.resize_exact(target_w, target_h, image::imageops::FilterType::Triangle);

    let scale_x = w as f64 / target_w as f64;
    let scale_y = h as f64 / target_h as f64;
    (resized, scale_x, scale_y)
}

// ---------------------------------------------------------------------------
// Contour area & perimeter
// ---------------------------------------------------------------------------

fn contour_area(points: &[imageproc::point::Point<i32>]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0_f64;
    let n = points.len();
    for i in 0..n {
        let current = &points[i];
        let next = &points[(i + 1) % n];
        area += (current.x as f64) * (next.y as f64) - (next.x as f64) * (current.y as f64);
    }
    area.abs() / 2.0
}

fn contour_perimeter(points: &[imageproc::point::Point<i32>]) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }
    let mut perimeter = 0.0_f64;
    let n = points.len();
    for i in 0..n {
        let current = &points[i];
        let next = &points[(i + 1) % n];
        let dx = (next.x - current.x) as f64;
        let dy = (next.y - current.y) as f64;
        perimeter += (dx * dx + dy * dy).sqrt();
    }
    perimeter
}

// ---------------------------------------------------------------------------
// Douglas-Peucker polygon approximation
// ---------------------------------------------------------------------------

fn douglas_peucker(
    points: &[imageproc::point::Point<i32>],
    epsilon: f64,
) -> Vec<imageproc::point::Point<i32>> {
    if points.len() <= 2 {
        return points.to_vec();
    }

    // For closed contours, find the point farthest from the first point
    // to split the contour into two open polylines.
    let first = points[0];
    let mut max_dist = 0.0_f64;
    let mut max_idx = 0;
    for (i, p) in points.iter().enumerate() {
        let d = ((p.x - first.x) as f64).hypot((p.y - first.y) as f64);
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }

    // Split into two halves and simplify each.
    let half1: Vec<_> = points[..=max_idx].to_vec();
    let half2: Vec<_> = {
        let mut v: Vec<_> = points[max_idx..].to_vec();
        v.push(points[0]);
        v
    };

    let mut simplified1 = dp_simplify(&half1, epsilon);
    let simplified2 = dp_simplify(&half2, epsilon);

    // Merge (remove duplicate junction point).
    if !simplified2.is_empty() {
        simplified1.extend_from_slice(&simplified2[1..simplified2.len().saturating_sub(1)]);
    }

    simplified1
}

fn dp_simplify(
    points: &[imageproc::point::Point<i32>],
    epsilon: f64,
) -> Vec<imageproc::point::Point<i32>> {
    if points.len() <= 2 {
        return points.to_vec();
    }

    let first = points[0];
    let last = points[points.len() - 1];

    let mut max_dist = 0.0_f64;
    let mut max_idx = 0;
    for (i, p) in points.iter().enumerate().skip(1).take(points.len() - 2) {
        let d = point_to_line_distance(p, &first, &last);
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }

    if max_dist > epsilon {
        let mut left = dp_simplify(&points[..=max_idx], epsilon);
        let right = dp_simplify(&points[max_idx..], epsilon);
        left.pop(); // remove duplicate junction
        left.extend(right);
        left
    } else {
        vec![first, last]
    }
}

fn point_to_line_distance(
    p: &imageproc::point::Point<i32>,
    line_start: &imageproc::point::Point<i32>,
    line_end: &imageproc::point::Point<i32>,
) -> f64 {
    let dx = (line_end.x - line_start.x) as f64;
    let dy = (line_end.y - line_start.y) as f64;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-12 {
        return ((p.x - line_start.x) as f64).hypot((p.y - line_start.y) as f64);
    }
    let numerator = ((line_end.x - line_start.x) as f64 * (line_start.y - p.y) as f64
        - (line_start.x - p.x) as f64 * (line_end.y - line_start.y) as f64)
        .abs();
    numerator / len_sq.sqrt()
}

// ---------------------------------------------------------------------------
// Convexity test
// ---------------------------------------------------------------------------

fn is_convex_polygon(points: &[imageproc::point::Point<i32>]) -> bool {
    let n = points.len();
    if n < 3 {
        return false;
    }
    let mut sign = 0_i64;
    for i in 0..n {
        let a = &points[i];
        let b = &points[(i + 1) % n];
        let c = &points[(i + 2) % n];
        let cross = (b.x as i64 - a.x as i64) * (c.y as i64 - b.y as i64)
            - (b.y as i64 - a.y as i64) * (c.x as i64 - b.x as i64);
        if cross != 0 {
            if sign == 0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Point ordering: TL → TR → BR → BL (ported from legacy orderPoints)
// ---------------------------------------------------------------------------

fn order_points(pts: &[Point; 4]) -> [Point; 4] {
    // Compute centroid.
    let cx = (pts[0].x + pts[1].x + pts[2].x + pts[3].x) / 4.0;
    let cy = (pts[0].y + pts[1].y + pts[2].y + pts[3].y) / 4.0;

    // Sort by angle around centroid.
    let mut indexed: Vec<(usize, f64)> = pts
        .iter()
        .enumerate()
        .map(|(i, p)| (i, (p.y - cy).atan2(p.x - cx)))
        .collect();
    indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let sorted: Vec<Point> = indexed.iter().map(|&(i, _)| pts[i]).collect();

    // Find top-left (smallest x+y sum).
    let tl_idx = sorted
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            (a.x + a.y)
                .partial_cmp(&(b.x + b.y))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0);

    // Rotate so TL is first.
    let mut ordered = [Point { x: 0.0, y: 0.0 }; 4];
    for i in 0..4 {
        ordered[i] = sorted[(tl_idx + i) % 4];
    }

    // Ensure second point is top-right (lower y) not bottom-left (higher y).
    if ordered[1].y > ordered[3].y {
        ordered = [ordered[0], ordered[3], ordered[2], ordered[1]];
    }

    ordered
}

// ---------------------------------------------------------------------------
// Quad scoring (ported 1:1 from legacy computeQuadScore)
// ---------------------------------------------------------------------------

fn compute_quad_score(
    points: &[Point; 4],
    contour_area: f64,
    frame_width: f64,
    frame_height: f64,
) -> f64 {
    let polygon_area = polygon_area_f64(points);
    if polygon_area <= 0.0 {
        return 0.0;
    }

    let extent_score = (polygon_area / bounding_box_area(points).max(1.0)).clamp(0.0, 1.0);
    let contour_coverage_score = (contour_area / polygon_area).clamp(0.0, 1.0);
    let right_angle_score = compute_right_angle_score(points);
    let side_balance_score = compute_side_balance_score(points);
    let area_score = compute_area_score(points, frame_width, frame_height);
    let border_touch_penalty = compute_border_touch_penalty(points, frame_width, frame_height);

    (right_angle_score * 2.2)
        + (extent_score * 1.2)
        + (contour_coverage_score * 0.9)
        + (side_balance_score * 0.7)
        + (area_score * 1.0)
        - (border_touch_penalty * 0.8)
}

fn polygon_area_f64(points: &[Point]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0_f64;
    let n = points.len();
    for i in 0..n {
        let c = &points[i];
        let nx = &points[(i + 1) % n];
        area += c.x * nx.y - nx.x * c.y;
    }
    area.abs() / 2.0
}

fn bounding_box_area(points: &[Point]) -> f64 {
    let xs: Vec<f64> = points.iter().map(|p| p.x).collect();
    let ys: Vec<f64> = points.iter().map(|p| p.y).collect();
    let w = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - xs.iter().cloned().fold(f64::INFINITY, f64::min);
    let h = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - ys.iter().cloned().fold(f64::INFINITY, f64::min);
    (w * h).max(1.0)
}

fn dist(a: &Point, b: &Point) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn compute_right_angle_score(points: &[Point; 4]) -> f64 {
    let mut score_sum = 0.0;
    for i in 0..4 {
        let prev = &points[(i + 3) % 4];
        let curr = &points[i];
        let next = &points[(i + 1) % 4];
        score_sum += 1.0 - compute_angle_cosine(prev, curr, next).clamp(0.0, 1.0);
    }
    score_sum / 4.0
}

fn compute_angle_cosine(prev: &Point, curr: &Point, next: &Point) -> f64 {
    let ax = prev.x - curr.x;
    let ay = prev.y - curr.y;
    let bx = next.x - curr.x;
    let by = next.y - curr.y;
    let mag_a = (ax * ax + ay * ay).sqrt();
    let mag_b = (bx * bx + by * by).sqrt();
    if mag_a < 1e-12 || mag_b < 1e-12 {
        return 1.0;
    }
    ((ax * bx + ay * by) / (mag_a * mag_b)).abs()
}

fn compute_side_balance_score(points: &[Point; 4]) -> f64 {
    let top_w = dist(&points[0], &points[1]);
    let bottom_w = dist(&points[3], &points[2]);
    let left_h = dist(&points[0], &points[3]);
    let right_h = dist(&points[1], &points[2]);

    let width_bal = top_w.min(bottom_w) / top_w.max(bottom_w).max(1.0);
    let height_bal = left_h.min(right_h) / left_h.max(right_h).max(1.0);
    (width_bal + height_bal) / 2.0
}

fn compute_area_score(points: &[Point; 4], frame_width: f64, frame_height: f64) -> f64 {
    let frame_area = (frame_width * frame_height).max(1.0);
    let area_ratio = polygon_area_f64(points) / frame_area;
    if area_ratio < CONTOUR_AREA_MIN_RATIO || area_ratio > CONTOUR_AREA_MAX_RATIO {
        return 0.0;
    }
    let target = 0.50;
    let normalized_dist = (area_ratio - target).abs() / 0.55;
    1.0 - normalized_dist.min(1.0)
}

fn compute_border_touch_penalty(points: &[Point; 4], frame_width: f64, frame_height: f64) -> f64 {
    let margin_x = (frame_width * BORDER_TOUCH_MARGIN_RATIO).max(4.0);
    let margin_y = (frame_height * BORDER_TOUCH_MARGIN_RATIO).max(4.0);

    let mut touching = 0_u32;
    for p in points {
        if p.x <= margin_x
            || p.x >= frame_width - margin_x
            || p.y <= margin_y
            || p.y >= frame_height - margin_y
        {
            touching += 1;
        }
    }
    (touching as f64 / points.len() as f64).min(1.0)
}
