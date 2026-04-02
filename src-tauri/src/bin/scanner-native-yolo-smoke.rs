use std::fs;
use std::path::PathBuf;

use app_lib::scanner_detect::{detect_document_native_yolo, ScannerDetectDocumentRequest};

fn main() {
    let image_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: scanner-native-yolo-smoke <image-path>");
    let image_bytes = fs::read(&image_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", image_path.display()));

    let response = detect_document_native_yolo(
        ScannerDetectDocumentRequest {
            source_bytes: image_bytes,
            max_width: None,
            max_height: None,
        },
        None,
        None,
    )
    .expect("smoke inference should succeed");

    println!(
        "{}",
        serde_json::to_string_pretty(&response)
            .expect("smoke inference output should serialize")
    );
}
