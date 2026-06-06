#![cfg(any(target_os = "windows", target_os = "linux"))]

mod scanner_platform_under_test {
    #![allow(dead_code)]

    include!("../src/scanner_platform.rs");

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn provider_names_are_normalized() {
            assert_eq!(normalize_provider_name(" directml "), "DirectML");
            assert_eq!(normalize_provider_name("TENSORRT"), "TensorRT");
            assert_eq!(normalize_provider_name("cuda"), "CUDA");
            assert_eq!(normalize_provider_name("cpu"), "CPU");
        }

        #[test]
        fn provider_availability_is_case_insensitive() {
            let providers = vec!["directml".to_string(), "CPU".to_string()];
            assert!(is_provider_available("DirectML", &providers));
        }
    }
}
