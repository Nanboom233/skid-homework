/// Platform-specific ORT execution provider helpers.
///
/// Centralizes the mapping between host OS and ONNX Runtime provider
/// names so that the detection module stays focused on inference logic.

/// The release-target identifier for the current platform.
pub fn platform_target() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows-directml",
        "linux" => "linux-tensorrt-cuda",
        _ => "desktop-unsupported",
    }
}

/// Ordered list of provider candidates for the current platform.
pub fn provider_candidates() -> Vec<&'static str> {
    match std::env::consts::OS {
        "windows" => vec!["DirectML", "CPU"],
        "linux" => vec!["TensorRT", "CUDA", "CPU"],
        _ => vec!["CPU"],
    }
}

/// The default preferred provider for the current platform.
pub fn default_preferred_provider() -> &'static str {
    match std::env::consts::OS {
        "windows" => "DirectML",
        "linux" => "TensorRT",
        _ => "CPU",
    }
}

/// Normalize a user-supplied or config-supplied provider name to its
/// canonical casing (e.g. `"directml"` → `"DirectML"`).
pub fn normalize_provider_name(provider: &str) -> String {
    match provider.trim().to_ascii_lowercase().as_str() {
        "directml" => "DirectML".to_string(),
        "tensorrt" => "TensorRT".to_string(),
        "cuda" => "CUDA".to_string(),
        "cpu" => "CPU".to_string(),
        other => other.to_string(),
    }
}

/// Check whether a preferred provider is present in the list of available
/// providers (comparison is case-insensitive via [`normalize_provider_name`]).
pub fn is_provider_available(preferred_provider: &str, available_providers: &[String]) -> bool {
    let normalized = normalize_provider_name(preferred_provider);
    available_providers
        .iter()
        .any(|provider| normalize_provider_name(provider) == normalized)
}
