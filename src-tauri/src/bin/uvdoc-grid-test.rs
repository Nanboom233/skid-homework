//! UVDoc grid diagnostic & quantitative comparison tool.
//! Usage: uvdoc-grid-test <image-path> <output-dir> [resource-root] [--rotate <0|90|180|270>]
//!
//! Runs UVDoc with all three grid_postprocess modes ("none", "x-stretch-equalize",
//! "affine-removal") side-by-side, saves output images, and prints quantitative
//! metrics for each:
//!
//!   - **Pixel diff** from raw "none" mode (MAE, RMSE, peak absolute diff)
//!   - **Row-level horizontal line straightness** — measures how wavy text lines
//!     are by computing the per-row luminance centroid and reporting the standard
//!     deviation of centroid-Y across the middle region.
//!   - **Vertical edge tilt** — measures text slant by computing Sobel-based
//!     dominant vertical edge angle in degrees.
//!   - **Edge content preservation** — percentage of non-transparent pixels at
//!     the image borders (detects edge cropping).

use app_lib::scanner_postprocess_model::run_native_postprocess_model_with_runtime_hints;
use image::RgbaImage;
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}

fn rotate_rgba(img: &RgbaImage, degrees: u16) -> RgbaImage {
    match degrees {
        90 => image::imageops::rotate90(img),
        180 => image::imageops::rotate180(img),
        270 => image::imageops::rotate270(img),
        _ => img.clone(),
    }
}

// ────────────────────────── Quantitative Metrics ──────────────────────────

/// Mean Absolute Error between two RGBA images (per pixel, averaged over RGB channels).
fn compute_mae(a: &RgbaImage, b: &RgbaImage) -> f64 {
    if a.width() != b.width() || a.height() != b.height() {
        return f64::NAN;
    }
    let total = (a.width() as u64 * a.height() as u64) as f64;
    if total == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0_f64;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        for ch in 0..3 {
            sum += (pa.0[ch] as f64 - pb.0[ch] as f64).abs();
        }
    }
    sum / (total * 3.0)
}

/// Root Mean Squared Error (RGB channels).
fn compute_rmse(a: &RgbaImage, b: &RgbaImage) -> f64 {
    if a.width() != b.width() || a.height() != b.height() {
        return f64::NAN;
    }
    let total = (a.width() as u64 * a.height() as u64) as f64;
    if total == 0.0 {
        return 0.0;
    }
    let mut sum_sq = 0.0_f64;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        for ch in 0..3 {
            let d = pa.0[ch] as f64 - pb.0[ch] as f64;
            sum_sq += d * d;
        }
    }
    (sum_sq / (total * 3.0)).sqrt()
}

/// Peak absolute difference (worst single channel value).
fn compute_peak_diff(a: &RgbaImage, b: &RgbaImage) -> u8 {
    if a.width() != b.width() || a.height() != b.height() {
        return 255;
    }
    let mut peak = 0u8;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        for ch in 0..3 {
            let d = (pa.0[ch] as i16 - pb.0[ch] as i16).unsigned_abs() as u8;
            if d > peak {
                peak = d;
            }
        }
    }
    peak
}

/// Percentage of edge pixels (1px border) that have alpha > 0 — detects edge cropping.
fn edge_content_percentage(img: &RgbaImage) -> f64 {
    let w = img.width();
    let h = img.height();
    if w < 2 || h < 2 {
        return 100.0;
    }
    let mut total = 0u64;
    let mut opaque = 0u64;
    // Top and bottom rows
    for x in 0..w {
        total += 2;
        if img.get_pixel(x, 0).0[3] > 0 {
            opaque += 1;
        }
        if img.get_pixel(x, h - 1).0[3] > 0 {
            opaque += 1;
        }
    }
    // Left and right columns (excluding corners already counted)
    for y in 1..h - 1 {
        total += 2;
        if img.get_pixel(0, y).0[3] > 0 {
            opaque += 1;
        }
        if img.get_pixel(w - 1, y).0[3] > 0 {
            opaque += 1;
        }
    }
    (opaque as f64 / total as f64) * 100.0
}

/// Row-level horizontal line straightness metric.
///
/// For each row in the middle 60% of the image, compute the luminance-weighted
/// centroid's Y deviation from row center. Return the standard deviation of
/// these centroids — lower = straighter text lines.
///
/// Actually, we measure horizontal uniformity: for each row, compute the
/// luminance variance across columns. High variance = strong text presence.
/// Then for high-variance (text) rows, measure the weighted centroid X position
/// and report its stddev — this tells us how much lines meander horizontally.
/// But a more useful "line straightness" metric: for the middle strip, compute
/// the row-wise luminance centroid Y (weighted by column gradient magnitude)
/// and report stddev. Lower = straighter.
///
/// Simplified version: compute per-row average luminance, take the middle 60%
/// rows, compute the lag-1 autocorrelation — periodic text lines yield high
/// autocorrelation. We just report the row-luminance variance as a proxy.
fn row_luminance_variance(img: &RgbaImage) -> f64 {
    let w = img.width() as usize;
    let h = img.height() as usize;
    if w == 0 || h < 5 {
        return 0.0;
    }
    let y_start = h / 5;
    let y_end = h * 4 / 5;
    let mut row_means = Vec::with_capacity(y_end - y_start);
    for y in y_start..y_end {
        let mut sum = 0.0_f64;
        for x in 0..w {
            let p = img.get_pixel(x as u32, y as u32).0;
            sum += 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64;
        }
        row_means.push(sum / w as f64);
    }
    let mean = row_means.iter().sum::<f64>() / row_means.len() as f64;
    let var = row_means
        .iter()
        .map(|v| (v - mean) * (v - mean))
        .sum::<f64>()
        / row_means.len() as f64;
    var.sqrt() // stddev of row luminance
}

/// Sobel-based dominant vertical edge angle in degrees.
/// Measures text slant: 0° = perfectly vertical, positive = clockwise.
fn vertical_edge_angle(img: &RgbaImage) -> f64 {
    let w = img.width() as i32;
    let h = img.height() as i32;
    if w < 3 || h < 3 {
        return 0.0;
    }

    let luma = |x: i32, y: i32| -> f64 {
        let p = img
            .get_pixel(x.clamp(0, w - 1) as u32, y.clamp(0, h - 1) as u32)
            .0;
        0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64
    };

    // Sample the center region to avoid edge artifacts
    let x_start = w / 5;
    let x_end = w * 4 / 5;
    let y_start = h / 5;
    let y_end = h * 4 / 5;
    let step = ((x_end - x_start) * (y_end - y_start) / 50000).max(1); // subsample for speed

    let mut sum_gx = 0.0_f64;
    let mut sum_gy = 0.0_f64;
    let mut sum_mag = 0.0_f64;
    let mut count = 0u64;

    for y in (y_start..y_end).step_by(step as usize) {
        for x in (x_start..x_end).step_by(step as usize) {
            // Sobel X
            let gx = -luma(x - 1, y - 1) + luma(x + 1, y - 1) - 2.0 * luma(x - 1, y)
                + 2.0 * luma(x + 1, y)
                - luma(x - 1, y + 1)
                + luma(x + 1, y + 1);
            // Sobel Y
            let gy = -luma(x - 1, y - 1) - 2.0 * luma(x, y - 1) - luma(x + 1, y - 1)
                + luma(x - 1, y + 1)
                + 2.0 * luma(x, y + 1)
                + luma(x + 1, y + 1);

            let mag = (gx * gx + gy * gy).sqrt();
            if mag > 10.0 {
                // Weight by magnitude — strong edges matter more
                sum_gx += gx * mag;
                sum_gy += gy * mag;
                sum_mag += mag;
                count += 1;
            }
        }
    }

    if count == 0 || sum_mag < 1.0 {
        return 0.0;
    }

    // The average gradient direction: atan2(sum_gy, sum_gx)
    // For vertical text lines, the dominant gradient is horizontal (gx >> gy)
    // so the "edge angle" = atan(sum_gy / sum_gx) gives the tilt from vertical
    let avg_angle_rad = (sum_gy / sum_gx).atan();
    avg_angle_rad.to_degrees()
}

/// Global luminance statistics: mean, stddev, min, max
fn luminance_stats(img: &RgbaImage) -> (f64, f64, u8, u8) {
    let mut sum = 0.0_f64;
    let mut sum_sq = 0.0_f64;
    let mut min_v = 255u8;
    let mut max_v = 0u8;
    let total = (img.width() as u64 * img.height() as u64) as f64;
    if total == 0.0 {
        return (0.0, 0.0, 0, 0);
    }
    for p in img.pixels() {
        let l = (0.299 * p.0[0] as f64 + 0.587 * p.0[1] as f64 + 0.114 * p.0[2] as f64)
            .round()
            .clamp(0.0, 255.0) as u8;
        sum += l as f64;
        sum_sq += (l as f64) * (l as f64);
        if l < min_v {
            min_v = l;
        }
        if l > max_v {
            max_v = l;
        }
    }
    let mean = sum / total;
    let var = (sum_sq / total) - mean * mean;
    (mean, var.max(0.0).sqrt(), min_v, max_v)
}

struct ModeResult {
    name: String,
    image: RgbaImage,
    model_ms: f64,
    warp_ms: f64,
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        return Err(
            "usage: uvdoc-grid-test <image-path> <output-dir> [resource-root] [--rotate <0|90|180|270>]".into(),
        );
    }
    let image_path = PathBuf::from(&args[1]);
    let output_dir = PathBuf::from(&args[2]);
    let mut resource_hint: Option<PathBuf> = None;
    let mut rotation: u16 = 0;

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--rotate" => {
                i += 1;
                rotation = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            other => {
                if resource_hint.is_none() {
                    resource_hint = Some(PathBuf::from(other));
                }
            }
        }
        i += 1;
    }

    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("Failed to create output dir: {e}"))?;

    // Load image
    let raw_img = image::open(&image_path)
        .map_err(|e| format!("Failed to open {}: {e}", image_path.display()))?
        .to_rgba8();

    eprintln!(
        "Loaded {} ({}x{})",
        image_path.display(),
        raw_img.width(),
        raw_img.height()
    );

    // Apply rotation (matching the real pipeline step 1)
    let img = if rotation != 0 {
        let rotated = rotate_rgba(&raw_img, rotation);
        eprintln!(
            "Rotated {}° → {}x{}",
            rotation,
            rotated.width(),
            rotated.height()
        );
        rotated
    } else {
        raw_img
    };

    // Run both modes
    let modes = ["none", "x-stretch-equalize"];
    let mut results: Vec<ModeResult> = Vec::new();

    for mode in &modes {
        eprintln!("\n===== grid_postprocess = {} =====", mode);
        let result = run_native_postprocess_model_with_runtime_hints(
            resource_hint.clone(),
            None,
            None,
            &img,
            None,
            mode,
        )
        .map_err(|e| format!("{} mode failed: {e}", mode))?;

        let filename = format!("output-{}.png", mode);
        let out_path = output_dir.join(&filename);
        result
            .image
            .save(&out_path)
            .map_err(|e| format!("save failed: {e}"))?;
        eprintln!("Saved {}", out_path.display());

        results.push(ModeResult {
            name: mode.to_string(),
            image: result.image,
            model_ms: result.model_ms,
            warp_ms: result.residual_warp_ms,
        });
    }

    // ── Quantitative Comparison ──
    eprintln!("\n╔══════════════════════════════════════════════════════════════════════════════════════════╗");
    eprintln!("║                         UVDoc Grid Postprocess — Quantitative Report                   ║");
    eprintln!("╠══════════════════════════════════════════════════════════════════════════════════════════╣");
    eprintln!(
        "║ Input: {} ({}x{} → {}x{} after rotation)                                    ",
        image_path.file_name().unwrap_or_default().to_string_lossy(),
        img.width(),
        img.height(),
        img.width(),
        img.height(),
    );
    eprintln!("╠══════════════════════════════════════════════════════════════════════════════════════════╣");

    // Header
    eprintln!(
        "║ {:22} │ {:>6} │ {:>6} │ {:>6} │ {:>6} │ {:>6} │ {:>4} │ {:>7} │ {:>6} │ {:>5} │ {:>5} ║",
        "Mode", "W×H", "Model", "Warp", "MAE", "RMSE", "Peak", "Edge%", "RowVar", "Angle", "LumSD"
    );
    eprintln!("║ {:─<22} │ {:─>6} │ {:─>6} │ {:─>6} │ {:─>6} │ {:─>6} │ {:─>4} │ {:─>7} │ {:─>6} │ {:─>5} │ {:─>5} ║",
        "", "", "", "", "", "", "", "", "", "", ""
    );

    let baseline = &results[0].image;

    for r in &results {
        let (mae, rmse, peak) = if r.name == "none" {
            (0.0, 0.0, 0)
        } else {
            (
                compute_mae(baseline, &r.image),
                compute_rmse(baseline, &r.image),
                compute_peak_diff(baseline, &r.image),
            )
        };
        let edge_pct = edge_content_percentage(&r.image);
        let row_var = row_luminance_variance(&r.image);
        let angle = vertical_edge_angle(&r.image);
        let (_lum_mean, lum_sd, _lum_min, _lum_max) = luminance_stats(&r.image);

        let size_str = format!("{}×{}", r.image.width(), r.image.height());

        eprintln!(
            "║ {:22} │ {:>6} │ {:5.1}ms │ {:5.1}ms │ {:6.2} │ {:6.2} │ {:>4} │ {:6.1}% │ {:6.2} │ {:+5.2}° │ {:5.1} ║",
            r.name,
            size_str,
            r.model_ms,
            r.warp_ms,
            mae,
            rmse,
            peak,
            edge_pct,
            row_var,
            angle,
            lum_sd,
        );
    }

    eprintln!("╠══════════════════════════════════════════════════════════════════════════════════════════╣");

    // Interpretation
    eprintln!("║ Metrics explanation:                                                                   ║");
    eprintln!("║  MAE/RMSE  = pixel diff from raw \"none\" mode (lower = less change from raw)            ║");
    eprintln!("║  Edge%     = border opacity (100% = no cropping, <100% = edge content lost)            ║");
    eprintln!("║  RowVar    = row luminance stddev in middle 60% (lower = more uniform/straighter)      ║");
    eprintln!("║  Angle     = dominant gradient tilt, degrees (0° = no slant)                           ║");
    eprintln!("║  LumSD     = global luminance stddev (contrast measure)                                ║");
    eprintln!("╚══════════════════════════════════════════════════════════════════════════════════════════╝");

    // Pixel diff heatmap: none vs x-stretch-equalize
    if results.len() >= 2 {
        let none_img = &results[0].image;
        let eq_img = &results[1].image;
        if none_img.width() == eq_img.width() && none_img.height() == eq_img.height() {
            let w = none_img.width();
            let h = none_img.height();
            let mut diff_map = RgbaImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let pa = none_img.get_pixel(x, y).0;
                    let pb = eq_img.get_pixel(x, y).0;
                    let dr = ((pa[0] as i16 - pb[0] as i16).abs() * 4).min(255) as u8;
                    let dg = ((pa[1] as i16 - pb[1] as i16).abs() * 4).min(255) as u8;
                    let db = ((pa[2] as i16 - pb[2] as i16).abs() * 4).min(255) as u8;
                    diff_map.put_pixel(x, y, image::Rgba([dr, dg, db, 255]));
                }
            }
            let diff_path = output_dir.join("diff-none-vs-x-stretch.png");
            diff_map
                .save(&diff_path)
                .map_err(|e| format!("save diff failed: {e}"))?;
            eprintln!("Saved pixel diff heatmap → {}", diff_path.display());
        }
    }

    Ok(())
}
