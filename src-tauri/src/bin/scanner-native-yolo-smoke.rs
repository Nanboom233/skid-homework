use std::fs;
use std::path::PathBuf;

use app_lib::scanner_detect::{detect_document_native_yolo, ScannerDetectDocumentRequest};

fn main() {
    let mut args = std::env::args().skip(1);
    let image_path = args
        .next()
        .map(PathBuf::from)
        .expect("usage: scanner-native-yolo-smoke <image-path> [--raw-rgba] [--max-width <px>] [--max-height <px>]");
    let mut raw_rgba = false;
    let mut max_width = None;
    let mut max_height = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--raw-rgba" => {
                raw_rgba = true;
            }
            "--max-width" => {
                let value = args.next().expect("missing value for --max-width");
                max_width = Some(value.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid --max-width value {value:?}: {error}")
                }));
            }
            "--max-height" => {
                let value = args.next().expect("missing value for --max-height");
                max_height = Some(value.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid --max-height value {value:?}: {error}")
                }));
            }
            other => {
                panic!("unknown argument: {other}");
            }
        }
    }

    let image_bytes = fs::read(&image_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", image_path.display()));
    let request = if raw_rgba {
        let rgba = image::load_from_memory(&image_bytes)
            .unwrap_or_else(|error| panic!("failed to decode {}: {error}", image_path.display()))
            .to_rgba8();
        let (width, height) = rgba.dimensions();
        ScannerDetectDocumentRequest {
            source_bytes: Vec::new(),
            rgba_bytes: rgba.into_raw(),
            rgba_width: Some(width),
            rgba_height: Some(height),
            max_width,
            max_height,
        }
    } else {
        ScannerDetectDocumentRequest {
            source_bytes: image_bytes,
            rgba_bytes: Vec::new(),
            rgba_width: None,
            rgba_height: None,
            max_width,
            max_height,
        }
    };

    let response =
        detect_document_native_yolo(request, None, None).expect("smoke inference should succeed");

    println!(
        "{}",
        serde_json::to_string_pretty(&response).expect("smoke inference output should serialize")
    );
}
