mod adb_plugin;

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_assets;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_ort;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_platform;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod scanner_resource;

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
        scanner_assets::scanner_assets_status,
        scanner_assets::scanner_assets_download,
        scanner_assets::scanner_assets_download_update,
        scanner_assets::scanner_assets_cancel,
        scanner_assets::scanner_assets_import,
        scanner_assets::scanner_assets_clear,
        scanner_assets::scanner_assets_check_update,
        scanner_ort::scanner_probe_ort,
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
