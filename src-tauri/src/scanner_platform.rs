pub fn platform_target() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows-directml",
        "linux" => "linux-tensorrt-cuda",
        _ => "desktop-unsupported",
    }
}

pub fn provider_candidates() -> Vec<&'static str> {
    match std::env::consts::OS {
        "windows" => vec!["DirectML", "CPU"],
        "linux" => vec!["TensorRT", "CUDA", "CPU"],
        _ => vec!["CPU"],
    }
}

pub fn default_preferred_provider() -> &'static str {
    match std::env::consts::OS {
        "windows" => "DirectML",
        "linux" => "TensorRT",
        _ => "CPU",
    }
}

pub fn normalize_provider_name(provider: &str) -> String {
    match provider.trim().to_ascii_lowercase().as_str() {
        "directml" => "DirectML".to_string(),
        "tensorrt" => "TensorRT".to_string(),
        "cuda" => "CUDA".to_string(),
        "cpu" => "CPU".to_string(),
        other => other.to_string(),
    }
}

pub fn is_provider_available(preferred_provider: &str, available_providers: &[String]) -> bool {
    let normalized = normalize_provider_name(preferred_provider);
    available_providers
        .iter()
        .any(|provider| normalize_provider_name(provider) == normalized)
}
