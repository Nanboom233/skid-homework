fn main() {
    let probe = app_lib::scanner_detect::probe_native_yolo_runtime();
    println!(
        "{}",
        serde_json::to_string_pretty(&probe).expect("probe output should serialize")
    );
}
