use app_lib::scanner_postprocess_model::describe_native_postprocess_model_with_hints;

fn main() {
    let resource_hint = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let status = describe_native_postprocess_model_with_hints(resource_hint);
    println!(
        "{}",
        serde_json::to_string_pretty(&status).expect("postprocess model status should serialize")
    );
}
