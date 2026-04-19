mod adb_plugin;
mod png_bridge;
pub mod scanner_detect;
pub mod scanner_detect_loop;
pub mod scanner_postprocess;
pub mod scanner_postprocess_model;
mod scanner_transport;
pub mod scanner_tracker;
mod stream_decoder;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            adb_plugin::tauri_adb_list_devices,
            adb_plugin::tauri_adb_pair,
            adb_plugin::tauri_adb_connect,
            adb_plugin::tauri_adb_shell,
            adb_plugin::tauri_adb_push,
            adb_plugin::tauri_adb_forward,
            adb_plugin::tauri_adb_remove_forward,
            scanner_transport::tauri_adb_screenshot,
            scanner_transport::tauri_adb_capture_still,
            scanner_transport::tauri_adb_capture_still_stream,
            scanner_transport::tauri_adb_start_server,
            scanner_transport::tauri_adb_stop_server,
            png_bridge::tauri_scanner_encode_png_rgba,
            scanner_detect::tauri_scanner_probe_detect,
            scanner_detect::tauri_scanner_read_detect_config,
            scanner_detect::tauri_scanner_write_detect_config,
            scanner_detect::tauri_scanner_detect_document,
            scanner_detect_loop::tauri_scanner_start_detection_loop,
            scanner_detect_loop::tauri_scanner_stop_detection_loop,
            scanner_postprocess::tauri_scanner_postprocess_image,
            scanner_postprocess::tauri_scanner_refine_document_corners,
            stream_decoder::tauri_scanner_start_stream,
            stream_decoder::tauri_scanner_stop_stream,
        ])
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
