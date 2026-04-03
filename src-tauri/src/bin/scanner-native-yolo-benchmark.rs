use std::fs;
use std::path::PathBuf;

use app_lib::scanner_detect::{
    detect_document_native_yolo, reset_scanner_yolo_runtime_caches, ScannerDetectDocumentRequest,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkSample {
    mode: &'static str,
    warmup_processing_ms: f64,
    average_processing_ms: f64,
    runs: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkReport {
    image_path: String,
    iterations: usize,
    samples: Vec<BenchmarkSample>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let image_path = args
        .next()
        .map(PathBuf::from)
        .expect("usage: scanner-native-yolo-benchmark <image-path> [--iterations <n>]");
    let mut iterations = 3usize;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--iterations" => {
                let value = args.next().expect("missing value for --iterations");
                iterations = value.parse::<usize>().unwrap_or_else(|error| {
                    panic!("invalid --iterations value {value:?}: {error}")
                });
            }
            other => {
                panic!("unknown argument: {other}");
            }
        }
    }

    let image_bytes = fs::read(&image_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", image_path.display()));
    let rgba = image::load_from_memory(&image_bytes)
        .unwrap_or_else(|error| panic!("failed to decode {}: {error}", image_path.display()))
        .to_rgba8();
    let (rgba_width, rgba_height) = rgba.dimensions();
    let rgba_bytes = rgba.into_raw();

    let report = BenchmarkReport {
        image_path: image_path.display().to_string(),
        iterations,
        samples: vec![
            benchmark_mode(
                "encoded-fullres",
                ScannerDetectDocumentRequest {
                    source_bytes: image_bytes.clone(),
                    rgba_bytes: Vec::new(),
                    use_latest_preview_frame: false,
                    rgba_width: None,
                    rgba_height: None,
                    max_width: None,
                    max_height: None,
                },
                iterations,
                false,
            ),
            benchmark_mode(
                "raw-rgba-bounded-320x180-cached",
                ScannerDetectDocumentRequest {
                    source_bytes: Vec::new(),
                    rgba_bytes: rgba_bytes.clone(),
                    use_latest_preview_frame: false,
                    rgba_width: Some(rgba_width),
                    rgba_height: Some(rgba_height),
                    max_width: Some(320),
                    max_height: Some(180),
                },
                iterations,
                false,
            ),
            benchmark_mode(
                "raw-rgba-bounded-320x180-reset-each-run",
                ScannerDetectDocumentRequest {
                    source_bytes: Vec::new(),
                    rgba_bytes,
                    use_latest_preview_frame: false,
                    rgba_width: Some(rgba_width),
                    rgba_height: Some(rgba_height),
                    max_width: Some(320),
                    max_height: Some(180),
                },
                iterations,
                true,
            ),
        ],
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("benchmark output should serialize")
    );
}

fn benchmark_mode(
    mode: &'static str,
    request: ScannerDetectDocumentRequest,
    iterations: usize,
    reset_before_each_run: bool,
) -> BenchmarkSample {
    if reset_before_each_run {
        reset_scanner_yolo_runtime_caches();
    }
    let warmup = detect_document_native_yolo(request.clone(), None, None)
        .unwrap_or_else(|error| panic!("warmup for {mode} failed: {error}"));
    let mut total_processing_ms = 0.0;

    for _ in 0..iterations {
        if reset_before_each_run {
            reset_scanner_yolo_runtime_caches();
        }
        let response = detect_document_native_yolo(request.clone(), None, None)
            .unwrap_or_else(|error| panic!("benchmark run for {mode} failed: {error}"));
        total_processing_ms += extract_processing_ms(&response);
    }

    BenchmarkSample {
        mode,
        warmup_processing_ms: extract_processing_ms(&warmup),
        average_processing_ms: total_processing_ms / iterations.max(1) as f64,
        runs: iterations,
    }
}

fn extract_processing_ms(response: &impl Serialize) -> f64 {
    serde_json::to_value(response)
        .expect("benchmark response should serialize")
        .get("processingMs")
        .and_then(|value| value.as_f64())
        .expect("benchmark response should include processingMs")
}
