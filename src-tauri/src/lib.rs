//! AnyOCR 桌面应用壳：窗口、插件、命令转发。业务逻辑都在 `crates/` 中。

mod commands;
mod engines;
mod jobs;
mod settings;

use std::sync::Arc;

use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

/// 显示并聚焦主窗口。
fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 AnyOCR", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("AnyOCR")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "anyocr=info,anyocr_lib=info,warn".into()),
        )
        .init();

    tauri::Builder::default()
        // 单实例：再次启动时唤出已有窗口（须最先注册）
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 安装包中 pdfium.dll 位于资源目录
            if std::env::var_os("ANYOCR_PDFIUM_DIR").is_none()
                && let Ok(dir) = app.path().resource_dir()
                && dir.join("pdfium.dll").exists()
            {
                // SAFETY: 启动阶段、尚未创建工作线程时设置一次。
                unsafe { std::env::set_var("ANYOCR_PDFIUM_DIR", dir) };
            }
            let data = app.path().app_data_dir()?;
            let config = app.path().app_config_dir()?;
            app.manage(engines::EngineState::new(data.join("models")));
            app.manage(settings::SettingsState::load(config.join("settings.json")));
            app.manage(commands::PdfState::default());
            app.manage(Arc::new(jobs::Jobs::default()));
            setup_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::app_info,
            commands::ocr_image_file,
            commands::scan_files,
            commands::doc_start,
            commands::batch_start,
            commands::job_cancel,
        ])
        .run(tauri::generate_context!())
        .expect("启动 AnyOCR 失败");
}
