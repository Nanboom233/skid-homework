use std::collections::VecDeque;

use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};
use ort::value::Tensor;

use crate::scanner_detect::ScannerPoint;
use crate::scanner_detect_config::{ResolvedScannerModel, ScannerModelVariant};
use crate::scanner_detect_image::scale_coordinate;
use crate::scanner_detect_runtime::run_ort_session_inference;

pub(crate) fn run_selected_model_inference(
    model: &ResolvedScannerModel,
    image: &DynamicImage,
) -> Result<Vec<ScannerPoint>, String> {
    match model.variant {
        ScannerModelVariant::ActivePublicBaseline => run_docaligner_fastvit_sa24(model, image),
    }
}

fn run_docaligner_fastvit_sa24(
    model: &ResolvedScannerModel,
    image: &DynamicImage,
) -> Result<Vec<ScannerPoint>, String> {
    let original = image.to_rgb8();
    let (original_width, original_height) = original.dimensions();
    let input_size = model
        .config
        .input_size
        .ok_or_else(|| "activePublicBaseline.inputSize must be configured.".to_string())?;
    let input_name = model
        .config
        .input_name
        .as_deref()
        .ok_or_else(|| "activePublicBaseline.inputName must be configured.".to_string())?;
    let output_name = model
        .config
        .output_name
        .as_deref()
        .ok_or_else(|| "activePublicBaseline.outputName must be configured.".to_string())?;
    let resized = image::imageops::resize(
        &original,
        input_size[0],
        input_size[1],
        FilterType::Triangle,
    );
    let input = build_docaligner_input(&resized);
    let input_tensor = Tensor::<f32>::from_array((
        vec![
            1_i64,
            3_i64,
            i64::from(input_size[1]),
            i64::from(input_size[0]),
        ],
        input,
    ))
    .map_err(|error| format!("Failed to build ORT input tensor: {error}"))?;
    let (heatmap_data, heatmap_shape) =
        run_ort_session_inference(input_name, input_tensor, output_name)?;

    decode_docaligner_heatmap_output(
        &heatmap_data,
        &heatmap_shape,
        original_width,
        original_height,
    )
}

fn build_docaligner_input(image: &RgbImage) -> Vec<f32> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let plane_len = width * height;
    let mut input = vec![0.0_f32; plane_len * 3];

    for (idx, pixel) in image.pixels().enumerate() {
        let [r, g, b] = pixel.0;
        input[idx] = f32::from(b) / 255.0;
        input[plane_len + idx] = f32::from(g) / 255.0;
        input[(plane_len * 2) + idx] = f32::from(r) / 255.0;
    }

    input
}

fn decode_docaligner_heatmap_output(
    heatmap: &[f32],
    shape: &[i64],
    output_width: u32,
    output_height: u32,
) -> Result<Vec<ScannerPoint>, String> {
    if shape.len() != 4 {
        return Err(format!(
            "Unexpected DocAligner output rank {}. Expected [1, 4, H, W].",
            shape.len()
        ));
    }

    let channels =
        usize::try_from(shape[1]).map_err(|_| "Invalid heatmap channel count.".to_string())?;
    let heatmap_height =
        usize::try_from(shape[2]).map_err(|_| "Invalid heatmap height.".to_string())?;
    let heatmap_width =
        usize::try_from(shape[3]).map_err(|_| "Invalid heatmap width.".to_string())?;

    if channels < 4 {
        return Err(format!(
            "Unexpected DocAligner heatmap channel count {channels}. Expected at least 4."
        ));
    }

    let plane_len = heatmap_width * heatmap_height;
    let expected_len = plane_len * channels;
    if heatmap.len() < expected_len {
        return Err(format!(
            "Heatmap tensor is truncated: expected at least {expected_len} values, got {}.",
            heatmap.len()
        ));
    }

    let mut points = Vec::with_capacity(4);
    for channel in 0..4 {
        let start = channel * plane_len;
        let end = start + plane_len;
        let plane = &heatmap[start..end];
        let centroid =
            largest_component_centroid_f32(plane, heatmap_width, heatmap_height, 77.0 / 255.0)
                .or_else(|| argmax_point_f32(plane, heatmap_width, heatmap_height))
                .ok_or_else(|| {
                    format!(
                "DocAligner heatmap channel {channel} did not produce a detectable corner region."
            )
                })?;

        points.push(ScannerPoint {
            x: scale_coordinate(centroid.0, heatmap_width as u32, output_width),
            y: scale_coordinate(centroid.1, heatmap_height as u32, output_height),
        });
    }

    Ok(points)
}

fn largest_component_centroid_f32(
    heatmap: &[f32],
    width: usize,
    height: usize,
    threshold: f32,
) -> Option<(f32, f32)> {
    let mut visited = vec![false; width * height];
    let mut best_count = 0usize;
    let mut best_centroid = None;

    for y in 0..height {
        for x in 0..width {
            let index = (y * width) + x;
            if visited[index] || heatmap.get(index).copied().unwrap_or(0.0) <= threshold {
                continue;
            }

            let mut queue = VecDeque::from([(x, y)]);
            visited[index] = true;
            let mut count = 0usize;
            let mut sum_x = 0f64;
            let mut sum_y = 0f64;
            let mut total_weight = 0f64;

            while let Some((cx, cy)) = queue.pop_front() {
                count += 1;
                let current_index = (cy * width) + cx;
                let weight = heatmap
                    .get(current_index)
                    .copied()
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0) as f64;
                let effective_weight = if weight > 0.0 { weight } else { 1.0 };
                sum_x += cx as f64 * effective_weight;
                sum_y += cy as f64 * effective_weight;
                total_weight += effective_weight;

                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }

                        let nx = cx as isize + dx;
                        let ny = cy as isize + dy;
                        if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                            continue;
                        }

                        let nx = nx as usize;
                        let ny = ny as usize;
                        let neighbor_index = (ny * width) + nx;
                        if visited[neighbor_index]
                            || heatmap.get(neighbor_index).copied().unwrap_or(0.0) <= threshold
                        {
                            continue;
                        }

                        visited[neighbor_index] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }

            if count > best_count {
                best_count = count;
                let divisor = if total_weight > 0.0 {
                    total_weight
                } else {
                    count as f64
                };
                best_centroid = Some(((sum_x / divisor) as f32, (sum_y / divisor) as f32));
            }
        }
    }

    best_centroid
}

fn argmax_point_f32(heatmap: &[f32], width: usize, _height: usize) -> Option<(f32, f32)> {
    let (best_index, _) = heatmap
        .iter()
        .copied()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))?;

    Some(((best_index % width) as f32, (best_index / width) as f32))
}
