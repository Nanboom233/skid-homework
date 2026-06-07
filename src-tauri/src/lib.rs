mod adb_plugin;

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod png_bridge;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_assets;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_camera_resource;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_cv_detect;
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub mod scanner_detect;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_detect_config;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_detect_image;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_detect_loop;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_detect_model;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_detect_runtime;
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub mod scanner_frame_protocol;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_ort;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_platform;
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub mod scanner_postprocess;
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub mod scanner_postprocess_model;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_resource;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_tracker;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_transport;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod stream_decoder;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        adb_plugin::tauri_adb_list_devices,
        adb_plugin::tauri_adb_pair,
        adb_plugin::tauri_adb_connect,
        adb_plugin::tauri_adb_push,
        adb_plugin::tauri_adb_forward,
        adb_plugin::tauri_adb_remove_forward,
        adb_plugin::tauri_adb_screenshot,
        adb_plugin::tauri_adb_start_server,
        adb_plugin::tauri_adb_stop_server,
        adb_plugin::tauri_adb_start_log_tailer,
        adb_plugin::tauri_adb_stop_log_tailer,
        adb_plugin::tauri_adb_await_server_ready,
        scanner_assets::scanner_assets_status,
        scanner_assets::scanner_assets_download,
        scanner_assets::scanner_assets_download_update,
        scanner_assets::scanner_assets_cancel,
        scanner_assets::scanner_assets_import,
        scanner_assets::scanner_assets_clear,
        scanner_assets::scanner_assets_check_update,
        scanner_camera_resource::scanner_camera_server_artifact,
        scanner_ort::scanner_probe_ort,
        scanner_detect::tauri_scanner_probe_detect,
        scanner_detect::tauri_scanner_detect_document,
        scanner_detect_loop::tauri_scanner_start_detection_loop,
        scanner_detect_loop::tauri_scanner_stop_detection_loop,
        scanner_postprocess::tauri_scanner_postprocess_image,
        scanner_postprocess::tauri_scanner_refine_document_corners,
        scanner_transport::tauri_adb_capture_still,
        scanner_transport::tauri_adb_capture_still_stream,
        png_bridge::tauri_scanner_encode_png_rgba,
        stream_decoder::tauri_scanner_start_stream,
        stream_decoder::tauri_scanner_stop_stream,
    ]);

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        adb_plugin::tauri_adb_list_devices,
        adb_plugin::tauri_adb_pair,
        adb_plugin::tauri_adb_connect,
        adb_plugin::tauri_adb_push,
        adb_plugin::tauri_adb_forward,
        adb_plugin::tauri_adb_remove_forward,
        adb_plugin::tauri_adb_screenshot,
    ]);

    builder
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
