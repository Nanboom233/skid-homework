#![cfg(any(target_os = "windows", target_os = "linux"))]

#[path = "../src/scanner_platform.rs"]
mod scanner_platform;
#[path = "../src/scanner_resource.rs"]
mod scanner_resource;

mod scanner_assets {
    #![allow(dead_code)]

    include!("../src/scanner_assets.rs");
}

mod scanner_ort_under_test {
    #![allow(dead_code)]

    include!("../src/scanner_ort.rs");

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn manifest_contains_only_approved_models() {
            let paths = SCANNER_ORT_MODEL_MANIFEST
                .iter()
                .map(|model| model.relative_path)
                .collect::<Vec<_>>();
            assert_eq!(paths.len(), 2);
            assert!(paths.contains(&"models/docaligner-fastvit_sa24.onnx"));
            assert!(paths.contains(&"models/uvdoc-best-model.onnx"));
        }

        #[test]
        fn execution_providers_always_include_cpu_fallback() {
            let providers = build_scanner_execution_providers(&[]);
            assert!(!providers.is_empty());
        }

        #[test]
        fn current_platform_interesting_paths_include_models() {
            let paths = interesting_paths_for_current_platform();
            assert!(paths.contains(&"models/docaligner-fastvit_sa24.onnx"));
            assert!(paths.contains(&"models/uvdoc-best-model.onnx"));
        }

        #[test]
        fn runtime_path_mismatch_error_is_absent_for_same_path() {
            let path = PathBuf::from("assets/current/onnxruntime/runtime.dll");

            assert!(runtime_path_mismatch_error(Some(&path), Some(&path)).is_none());
        }

        #[test]
        fn runtime_path_mismatch_error_reports_loaded_and_selected_paths() {
            let selected = PathBuf::from("assets/current/onnxruntime/runtime.dll");
            let loaded = PathBuf::from("assets/old-current/onnxruntime/runtime.dll");

            let error = runtime_path_mismatch_error(Some(&selected), Some(&loaded))
                .expect("different runtime paths must report a mismatch");

            assert!(error.contains("already initialized"));
            assert!(error.contains(&scanner_resource::path_to_string(&loaded)));
            assert!(error.contains(&scanner_resource::path_to_string(&selected)));
        }

        #[test]
        fn probe_message_prioritizes_runtime_path_mismatch() {
            let selected = PathBuf::from("assets/current/onnxruntime/runtime.dll");
            let loaded = PathBuf::from("assets/old-current/onnxruntime/runtime.dll");
            let runtime_error = runtime_path_mismatch_error(Some(&selected), Some(&loaded));
            let snapshot = OrtRuntimeSnapshot {
                ready: true,
                runtime_error: runtime_error.clone(),
                ort_build_info: Some("ort-test".to_string()),
                available_providers: vec!["CPU".to_string()],
                selected_runtime_library_path: Some(selected),
                loaded_runtime_library_path: Some(loaded),
                runtime_path_mismatch: true,
            };

            assert_eq!(
                build_probe_message(&snapshot, &[], true),
                runtime_error.unwrap()
            );
        }

        #[test]
        fn runtime_library_path_status_texts_keep_loaded_path_as_compat_field() {
            let selected = PathBuf::from("assets/current/onnxruntime/runtime.dll");
            let loaded = PathBuf::from("assets/old-current/onnxruntime/runtime.dll");
            let snapshot = OrtRuntimeSnapshot {
                ready: true,
                runtime_error: None,
                ort_build_info: Some("ort-test".to_string()),
                available_providers: vec!["CPU".to_string()],
                selected_runtime_library_path: Some(selected.clone()),
                loaded_runtime_library_path: Some(loaded.clone()),
                runtime_path_mismatch: true,
            };

            let (selected_text, loaded_text, compat_text) =
                runtime_library_path_status_texts(&snapshot);

            assert_eq!(
                selected_text,
                Some(scanner_resource::path_to_string(&selected))
            );
            assert_eq!(loaded_text, Some(scanner_resource::path_to_string(&loaded)));
            assert_eq!(compat_text, Some(scanner_resource::path_to_string(&loaded)));
        }

        #[test]
        #[ignore = "loads installed/dev ORT runtime and ONNX model sessions"]
        fn installed_or_dev_probe_loads_approved_model_sessions() {
            let resource_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
            let status = probe_scanner_ort_with_hints(Some(resource_dir), None);

            assert!(status.runtime_ready, "{}", status.message);
            assert!(status.model_load_ready, "{}", status.message);
            assert_eq!(status.models.len(), 2);
            assert!(status.models.iter().all(|model| model.session_ready));
            assert!(status.models.iter().all(|model| !model.inputs.is_empty()));
            assert!(status.models.iter().all(|model| !model.outputs.is_empty()));
        }
    }
}
