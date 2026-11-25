//! 全局快捷键：截图 OCR、粘贴 OCR、重复上次截图、截图识别二维码。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::capture::{self, Mode};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Hotkeys {
    pub screenshot: String,
    pub paste: String,
    pub re_screenshot: String,
    pub qr_screenshot: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HotkeyError {
    pub name: String,
    pub error: String,
}

fn run(app: &AppHandle, name: &str) {
    let app = app.clone();
    let name = name.to_owned();
    // 快捷键回调在主线程，截图会短暂等待窗口隐藏，放到后台线程
    std::thread::spawn(move || {
        let r = match name.as_str() {
            "screenshot" => capture::start(&app, Mode::Ocr),
            "qrScreenshot" => capture::start(&app, Mode::Qrcode),
            "reScreenshot" => capture::repeat_last(&app, Mode::Ocr),
            "paste" => capture::from_clipboard(&app, Mode::Ocr),
            _ => Ok(()),
        };
        if let Err(e) = r {
            let _ = app.emit("capture-error", e);
        }
    });
}

/// 重新注册全部快捷键，返回注册失败的项（如格式错误或被其他程序占用）。
pub fn apply(app: &AppHandle, hk: &Hotkeys) -> Vec<HotkeyError> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut errors = Vec::new();
    for (name, key) in [
        ("screenshot", &hk.screenshot),
        ("paste", &hk.paste),
        ("reScreenshot", &hk.re_screenshot),
        ("qrScreenshot", &hk.qr_screenshot),
    ] {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let parsed: Result<Shortcut, _> = key.parse();
        let result = match parsed {
            Ok(sc) => gs
                .on_shortcut(sc, move |app, _sc, ev| {
                    if ev.state == ShortcutState::Pressed {
                        run(app, name);
                    }
                })
                .map_err(|e| e.to_string()),
            Err(e) => Err(format!("格式无效：{e}")),
        };
        if let Err(error) = result {
            errors.push(HotkeyError {
                name: name.to_owned(),
                error,
            });
        }
    }
    errors
}
