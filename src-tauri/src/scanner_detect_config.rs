use std::path::{Path, PathBuf};

use crate::scanner_detect::ScannerDetectResourceStatus;
use crate::scanner_platform;
use crate::scanner_resource;

pub(crate) const WINDOWS_ORT_RELATIVE_PATH: &str = "onnxruntime/windows/onnxruntime.dll";
pub(crate) const WINDOWS_ORT_SHARED_RELATIVE_PATH: &str =
    "onnxruntime/windows/onnxruntime_providers_shared.dll";
pub(crate) const WINDOWS_DIRECTML_RELATIVE_PATH: &str = "onnxruntime/windows/DirectML.dll";
pub(crate) const LINUX_ORT_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime.so";
const LINUX_TENSORRT_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime_providers_tensorrt.so";
const LINUX_CUDA_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime_providers_cuda.so";

/// Interesting paths used to score resource roots for the detection subsystem.
pub(crate) const DETECT_INTERESTING_PATHS: [&str; 7] = [
    "models/docaligner-fastvit_sa24.onnx",
    WINDOWS_ORT_RELATIVE_PATH,
    WINDOWS_ORT_SHARED_RELATIVE_PATH,
    WINDOWS_DIRECTML_RELATIVE_PATH,
    LINUX_ORT_RELATIVE_PATH,
    LINUX_TENSORRT_RELATIVE_PATH,
    LINUX_CUDA_RELATIVE_PATH,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScannerModelVariant {
    ActivePublicBaseline,
}

impl ScannerModelVariant {
    pub(crate) fn resource_key(self) -> &'static str {
        match self {
            Self::ActivePublicBaseline => "model-active-public-baseline",
        }
    }

    pub(crate) fn detection_implemented(self) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ScannerDetectModelConfig {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) task: String,
    pub(crate) model_path: String,
    pub(crate) input_name: Option<String>,
    pub(crate) output_name: Option<String>,
    pub(crate) input_size: Option<[u32; 2]>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScannerDetectConfig {
    pub(crate) active_public_baseline: ScannerDetectModelConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedScannerModel {
    pub(crate) variant: ScannerModelVariant,
    pub(crate) config: ScannerDetectModelConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct ScannerDetectConfigHandle {
    pub(crate) config: ScannerDetectConfig,
    pub(crate) source: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct ResourceSpec {
    key: String,
    relative_path: String,
    required: bool,
}

/// Hardcoded model configuration. These values are tightly coupled to the
/// bundled ONNX model files and must not be user-editable.
fn hardcoded_scanner_detect_config() -> ScannerDetectConfig {
    ScannerDetectConfig {
        active_public_baseline: ScannerDetectModelConfig {
            id: "docaligner-fastvit-sa24".to_string(),
            kind: "public-baseline".to_string(),
            task: "document-corner-heatmap".to_string(),
            model_path: "models/docaligner-fastvit_sa24.onnx".to_string(),
            input_name: Some("img".to_string()),
            output_name: Some("heatmap".to_string()),
            input_size: Some([256, 256]),
        },
    }
}

/// Returns the hardcoded scanner detect config. No external file is read.
pub(crate) fn resolve_scanner_detect_config() -> ScannerDetectConfigHandle {
    ScannerDetectConfigHandle {
        config: hardcoded_scanner_detect_config(),
        source: "hardcoded-internal",
    }
}

pub(crate) fn resource_specs_for_current_platform(
    config: Option<&ScannerDetectConfig>,
) -> Vec<ResourceSpec> {
    let mut specs = Vec::new();

    if let Some(config) = config {
        for model in candidate_model_variants(config) {
            specs.push(ResourceSpec {
                key: model.variant.resource_key().to_string(),
                relative_path: model.config.model_path.clone(),
                required: model.variant.detection_implemented(),
            });
        }
    }

    match std::env::consts::OS {
        "windows" => {
            specs.push(ResourceSpec {
                key: "windows-ort-core".to_string(),
                relative_path: WINDOWS_ORT_RELATIVE_PATH.to_string(),
                required: true,
            });
            specs.push(ResourceSpec {
                key: "windows-ort-shared".to_string(),
                relative_path: WINDOWS_ORT_SHARED_RELATIVE_PATH.to_string(),
                required: true,
            });
            specs.push(ResourceSpec {
                key: "windows-directml".to_string(),
                relative_path: WINDOWS_DIRECTML_RELATIVE_PATH.to_string(),
                required: false,
            });
        }
        "linux" => {
            specs.push(ResourceSpec {
                key: "linux-ort-core".to_string(),
                relative_path: LINUX_ORT_RELATIVE_PATH.to_string(),
                required: true,
            });
            specs.push(ResourceSpec {
                key: "linux-tensorrt-provider".to_string(),
                relative_path: LINUX_TENSORRT_RELATIVE_PATH.to_string(),
                required: false,
            });
            specs.push(ResourceSpec {
                key: "linux-cuda-provider".to_string(),
                relative_path: LINUX_CUDA_RELATIVE_PATH.to_string(),
                required: false,
            });
        }
        _ => {}
    }

    specs
}

pub(crate) fn build_resource_statuses(
    resource_base_dir: Option<&Path>,
    specs: &[ResourceSpec],
) -> Vec<ScannerDetectResourceStatus> {
    specs
        .iter()
        .map(|spec| {
            let resolved_path =
                resource_base_dir.map(|base_dir| base_dir.join(&spec.relative_path));
            let exists = resolved_path
                .as_ref()
                .map(|path| path.exists())
                .unwrap_or(false);

            ScannerDetectResourceStatus {
                key: spec.key.clone(),
                relative_path: spec.relative_path.clone(),
                resolved_path: resolved_path
                    .as_deref()
                    .map(scanner_resource::path_to_string),
                exists,
                required: spec.required,
            }
        })
        .collect()
}

pub(crate) fn preferred_provider_from_config(_config: &ScannerDetectConfig) -> String {
    scanner_platform::default_preferred_provider().to_string()
}

pub(crate) fn select_model_variant(
    config: &ScannerDetectConfig,
    resources: &[ScannerDetectResourceStatus],
) -> Option<ResolvedScannerModel> {
    candidate_model_variants(config)
        .into_iter()
        .find(|model| resource_exists(resources, model.variant.resource_key()))
}

pub(crate) fn runtime_library_path_for_current_platform(
    resource_base_dir: &Path,
) -> Option<PathBuf> {
    match std::env::consts::OS {
        "windows" => Some(resource_base_dir.join(WINDOWS_ORT_RELATIVE_PATH)),
        "linux" => Some(resource_base_dir.join(LINUX_ORT_RELATIVE_PATH)),
        _ => None,
    }
}

fn candidate_model_variants(config: &ScannerDetectConfig) -> Vec<ResolvedScannerModel> {
    vec![ResolvedScannerModel {
        variant: ScannerModelVariant::ActivePublicBaseline,
        config: config.active_public_baseline.clone(),
    }]
}

fn resource_exists(resources: &[ScannerDetectResourceStatus], key: &str) -> bool {
    resources
        .iter()
        .find(|resource| resource.key == key)
        .map(|resource| resource.exists)
        .unwrap_or(false)
}
