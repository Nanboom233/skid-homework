use std::path::PathBuf;

use serde::Serialize;
use tauri::{command, AppHandle, Manager};

use crate::scanner_resource::{path_to_string, validate_path_containment};

pub const CAMERA_SERVER_RESOURCE_NAME: &str = "camera-server.jar";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraServerArtifact {
    pub path: String,
    pub source: String,
}

pub fn resolve_camera_server_artifact_from_resource_dir(
    resource_dir_hint: Option<PathBuf>,
) -> Result<CameraServerArtifact, String> {
    let resource_dir = resource_dir_hint.ok_or_else(|| {
        "Tauri resource directory is unavailable; cannot locate bundled camera-server.jar."
            .to_string()
    })?;

    let candidates = [
        resource_dir.join(CAMERA_SERVER_RESOURCE_NAME),
        resource_dir
            .join("resources")
            .join(CAMERA_SERVER_RESOURCE_NAME),
    ];

    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }

        let canonical = validate_path_containment(&candidate, &resource_dir)?;
        return Ok(CameraServerArtifact {
            path: path_to_string(&canonical),
            source: "tauri-resource-dir".to_string(),
        });
    }

    Err(format!(
        "Bundled camera-server.jar was not found under {}.",
        path_to_string(&resource_dir)
    ))
}

pub fn resolve_camera_server_artifact(app: &AppHandle) -> Result<CameraServerArtifact, String> {
    resolve_camera_server_artifact_from_resource_dir(app.path().resource_dir().ok())
}

#[command]
pub async fn scanner_camera_server_artifact(
    app: AppHandle,
) -> Result<CameraServerArtifact, String> {
    tauri::async_runtime::spawn_blocking(move || resolve_camera_server_artifact(&app))
        .await
        .map_err(|error| format!("Camera server resource resolution task failed: {error}"))?
}
