use app_lib::scanner_postprocess_model::run_native_postprocess_model_with_runtime_hints;
use image::{Rgba, RgbaImage};

fn main() {
    let resource_hint = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    let mut source = RgbaImage::new(256, 192);
    for y in 0..source.height() {
        for x in 0..source.width() {
            let value = if (x / 16 + y / 16) % 2 == 0 { 240 } else { 32 };
            source.put_pixel(x, y, Rgba([value, value, value, 255]));
        }
    }

    let result =
        run_native_postprocess_model_with_runtime_hints(resource_hint, None, &source, None, "none")
            .expect("UVDoc smoke run should succeed");
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "modelId": result.model_id,
            "controlGridShape": result.control_grid_shape,
            "modelMs": result.model_ms,
            "residualWarpMs": result.residual_warp_ms,
            "outputWidth": result.image.width(),
            "outputHeight": result.image.height()
        }))
        .expect("smoke JSON should serialize")
    );
}
