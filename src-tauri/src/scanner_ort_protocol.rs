use std::io::{self, Read, Write};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub const STAGE: &str = "scanner-ort-base";
pub const WINDOWS_ORT_RELATIVE_PATH: &str = "onnxruntime/windows/onnxruntime.dll";
pub const WINDOWS_ORT_SHARED_RELATIVE_PATH: &str =
    "onnxruntime/windows/onnxruntime_providers_shared.dll";
pub const WINDOWS_DIRECTML_RELATIVE_PATH: &str = "onnxruntime/windows/DirectML.dll";
pub const LINUX_ORT_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime.so";
pub const LINUX_ORT_SONAME_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime.so.1";
pub const LINUX_ORT_VERSIONED_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime.so.1.24.4";
pub const LINUX_SHARED_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime_providers_shared.so";
pub const LINUX_CUDA_RELATIVE_PATH: &str = "onnxruntime/linux/libonnxruntime_providers_cuda.so";
pub const LINUX_TENSORRT_RELATIVE_PATH: &str =
    "onnxruntime/linux/libonnxruntime_providers_tensorrt.so";

const MAX_WORKER_MESSAGE_BYTES: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct ScannerOrtModelManifestEntry {
    pub id: &'static str,
    pub role: &'static str,
    pub relative_path: &'static str,
}

pub const SCANNER_ORT_MODEL_MANIFEST: [ScannerOrtModelManifestEntry; 2] = [
    ScannerOrtModelManifestEntry {
        id: "docaligner-fastvit-sa24",
        role: "stage-1-public-baseline",
        relative_path: "models/docaligner-fastvit_sa24.onnx",
    },
    ScannerOrtModelManifestEntry {
        id: "uvdoc-grid-v1",
        role: "stage-2-session-readiness",
        relative_path: "models/uvdoc-best-model.onnx",
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtOutletStatus {
    pub name: String,
    pub dtype: String,
    pub shape: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtModelStatus {
    pub id: String,
    pub role: String,
    pub relative_path: String,
    pub resolved_path: Option<String>,
    pub file_exists: bool,
    pub session_ready: bool,
    pub session_error: Option<String>,
    pub inputs: Vec<ScannerOrtOutletStatus>,
    pub outputs: Vec<ScannerOrtOutletStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtWorkerRuntimeStatus {
    pub ready: bool,
    pub runtime_error: Option<String>,
    pub ort_build_info: Option<String>,
    pub available_providers: Vec<String>,
    pub selected_runtime_library_path: Option<String>,
    pub loaded_runtime_library_path: Option<String>,
    pub runtime_path_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtWorkerProbeResult {
    pub runtime: ScannerOrtWorkerRuntimeStatus,
    pub models: Vec<ScannerOrtModelStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtWorkerRequest {
    pub id: u64,
    #[serde(flatten)]
    pub payload: ScannerOrtWorkerRequestPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "camelCase")]
pub enum ScannerOrtWorkerRequestPayload {
    Init(ScannerOrtWorkerResourceRequest),
    Probe(ScannerOrtWorkerResourceRequest),
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtWorkerResourceRequest {
    pub resource_base_dir: String,
    pub runtime_library_path: Option<String>,
    pub models: Vec<ScannerOrtWorkerModelRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtWorkerModelRequest {
    pub id: String,
    pub role: String,
    pub relative_path: String,
    pub resolved_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerOrtWorkerResponse {
    pub id: u64,
    #[serde(flatten)]
    pub payload: ScannerOrtWorkerResponsePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScannerOrtWorkerResponsePayload {
    Ok {
        operation: ScannerOrtWorkerOperation,
        #[serde(skip_serializing_if = "Option::is_none")]
        probe: Option<ScannerOrtWorkerProbeResult>,
    },
    Error {
        code: String,
        message: String,
    },
    Event {
        event: String,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScannerOrtWorkerOperation {
    Init,
    Probe,
    Shutdown,
}

#[allow(dead_code)]
pub fn build_runtime_path_mismatch_error(selected: &str, loaded: &str) -> String {
    format!(
        "ONNX Runtime is already initialized from {loaded}, but the selected scanner asset runtime is {selected}. Restart the app to switch runtime libraries."
    )
}

pub fn write_framed_json<W, T>(writer: &mut W, value: &T) -> io::Result<()>
where
    W: Write,
    T: Serialize,
{
    let payload = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let len = u32::try_from(payload.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "scanner ORT worker message is too large",
        )
    })?;
    if len > MAX_WORKER_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "scanner ORT worker message exceeds protocol limit",
        ));
    }

    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

pub fn read_framed_json<R, T>(reader: &mut R) -> io::Result<Option<T>>
where
    R: Read,
    T: DeserializeOwned,
{
    let mut len_bytes = [0_u8; 4];
    match reader.read_exact(&mut len_bytes) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }

    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_WORKER_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "scanner ORT worker message exceeds protocol limit",
        ));
    }
    let mut payload = vec![0_u8; len as usize];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
