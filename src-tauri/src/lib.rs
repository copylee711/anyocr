//! AnyOCR 桌面应用壳：窗口、插件、命令转发。业务逻辑都在 `crates/` 中。

mod commands;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "anyocr=info,anyocr_lib=info,warn".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let models_dir = app.path().app_data_dir()?.join("models");
            app.manage(commands::EngineState::new(models_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::ocr_image_file])
        .run(tauri::generate_context!())
        .expect("启动 AnyOCR 失败");
}
