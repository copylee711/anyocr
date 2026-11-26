//! 截图与剪贴板识别。
//!
//! 流程：截下所有屏幕 → 每块屏幕一个置顶遮罩窗口（显示冻结画面）→ 用户框选 → 裁剪 →
//! OCR 或二维码识别 → 按设置复制结果、唤出主窗口，并通过事件把结果发给主窗口。

use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyocr_core::PageResult;
use anyocr_core::tbpu::Parser;
use anyocr_qrcode::{Code, Preprocess};
use image::{RgbImage, RgbaImage};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

use crate::engines::EngineState;
use crate::settings::SettingsState;

/// 截图用途。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Ocr,
    Qrcode,
}

struct Shot {
    monitor: usize,
    /// 屏幕左上角（物理像素）。
    x: i32,
    y: i32,
    image: RgbaImage,
}

/// 选区：(屏幕序号, x, y, w, h)，物理像素。
type Region = (usize, u32, u32, u32, u32);

#[derive(Default)]
pub struct CaptureState {
    shots: Mutex<Vec<Shot>>,
    mode: Mutex<Mode>,
    /// 截图前主窗口是否可见（结束后据此恢复）。
    main_was_visible: Mutex<bool>,
    /// 上次选区。
    last: Mutex<Option<Region>>,
}

/// 发给主窗口的识别结果（事件名 `capture-result`）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    pub mode: Mode,
    /// 识别所用图片（临时 PNG），用于预览。
    pub image: String,
    pub page: Option<PageResult>,
    pub codes: Vec<Code>,
    pub text: String,
    pub error: Option<String>,
}

/// 截图相关设置（存于设置的 `screenshot` 字段）。
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ShotSettings {
    parser: Parser,
    copy: bool,
    pop_main_window: bool,
    hide_window: bool,
}

impl Default for ShotSettings {
    fn default() -> Self {
        Self {
            parser: Parser::MultiPara,
            copy: true,
            pop_main_window: true,
            hide_window: true,
        }
    }
}

fn shot_settings(app: &AppHandle) -> ShotSettings {
    app.state::<SettingsState>()
        .get()
        .get("screenshot")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

fn qr_preprocess(app: &AppHandle) -> Preprocess {
    app.state::<SettingsState>()
        .get()
        .pointer("/qrcode/preprocess")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

fn temp_dir(app: &AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_cache_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("capture");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 快速 PNG 编码（截图要求低延迟，压缩率次要）。
fn save_png(img: &RgbaImage, path: &PathBuf) -> Result<(), String> {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    PngEncoder::new_with_quality(
        BufWriter::new(f),
        CompressionType::Fast,
        FilterType::NoFilter,
    )
    .write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}

/// 开始截图：截屏并打开遮罩窗口。
pub fn start(app: &AppHandle, mode: Mode) -> Result<(), String> {
    let state = app.state::<CaptureState>();
    if !state.shots.lock().map_err(|e| e.to_string())?.is_empty() {
        return Ok(()); // 正在截图
    }
    let ss = shot_settings(app);
    let main = app.get_webview_window("main");
    let visible = main
        .as_ref()
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
        && !main
            .as_ref()
            .and_then(|w| w.is_minimized().ok())
            .unwrap_or(false);
    *state.main_was_visible.lock().map_err(|e| e.to_string())? = visible;
    if visible && ss.hide_window {
        if let Some(w) = &main {
            let _ = w.hide();
        }
        std::thread::sleep(Duration::from_millis(120)); // 等待窗口隐藏动画结束
    }

    let monitors = xcap::Monitor::all().map_err(|e| format!("无法枚举屏幕：{e}"))?;
    let mut shots = Vec::new();
    for (i, m) in monitors.iter().enumerate() {
        // 截图保存在内存中，遮罩页通过二进制 IPC 取原始像素（省去 PNG 编码与写盘）
        let image = m.capture_image().map_err(|e| format!("截屏失败：{e}"))?;
        shots.push(Shot {
            monitor: i,
            x: m.x().unwrap_or(0),
            y: m.y().unwrap_or(0),
            image,
        });
    }
    *state.mode.lock().map_err(|e| e.to_string())? = mode;

    for s in &shots {
        let label = format!("overlay-{}", s.monitor);
        let url = format!("index.html#/overlay?i={}", s.monitor);
        let win = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
            .title("AnyOCR 截图")
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .visible(false)
            .focused(true)
            .build()
            .map_err(|e| format!("无法创建截图窗口：{e}"))?;
        let _ = win.set_position(PhysicalPosition::new(s.x, s.y));
        let _ = win.set_size(PhysicalSize::new(s.image.width(), s.image.height()));
    }
    *state.shots.lock().map_err(|e| e.to_string())? = shots;
    Ok(())
}

/// 遮罩页取截图像素：前 8 字节为宽、高（u32 小端），其后为 RGBA。
pub fn overlay_pixels(app: &AppHandle, monitor: usize) -> Result<Vec<u8>, String> {
    let state = app.state::<CaptureState>();
    let shots = state.shots.lock().map_err(|e| e.to_string())?;
    let s = shots
        .iter()
        .find(|s| s.monitor == monitor)
        .ok_or("截图已失效")?;
    let mut out = Vec::with_capacity(8 + s.image.as_raw().len());
    out.extend_from_slice(&s.image.width().to_le_bytes());
    out.extend_from_slice(&s.image.height().to_le_bytes());
    out.extend_from_slice(s.image.as_raw());
    Ok(out)
}

fn close_overlays(app: &AppHandle) {
    for (label, w) in app.webview_windows() {
        if label.starts_with("overlay-") {
            let _ = w.destroy();
        }
    }
}

fn cleanup(app: &AppHandle) -> Vec<Shot> {
    close_overlays(app);
    let state = app.state::<CaptureState>();
    state
        .shots
        .lock()
        .map(|mut s| std::mem::take(&mut *s))
        .unwrap_or_default()
}

fn restore_main(app: &AppHandle, pop: bool) {
    let was = app
        .state::<CaptureState>()
        .main_was_visible
        .lock()
        .map(|v| *v)
        .unwrap_or(false);
    if let Some(w) = app.get_webview_window("main")
        && (pop || was)
    {
        let _ = w.unminimize();
        let _ = w.show();
        if pop {
            let _ = w.set_focus();
        }
    }
}

/// 取消截图。
pub fn cancel(app: &AppHandle) {
    cleanup(app);
    restore_main(app, false);
}

/// 选区（遮罩页面的 CSS 像素）与页面视口尺寸。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub monitor: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub view_width: f64,
    pub view_height: f64,
}

/// 完成框选：裁剪并识别。
pub fn finish(app: &AppHandle, sel: Selection) -> Result<(), String> {
    let shots = cleanup(app);
    let shot = shots
        .into_iter()
        .find(|s| s.monitor == sel.monitor)
        .ok_or("截图已失效")?;
    let (iw, ih) = shot.image.dimensions();
    let sx = iw as f64 / sel.view_width.max(1.0);
    let sy = ih as f64 / sel.view_height.max(1.0);
    let x = ((sel.x * sx).round().max(0.0) as u32).min(iw.saturating_sub(1));
    let y = ((sel.y * sy).round().max(0.0) as u32).min(ih.saturating_sub(1));
    let w = ((sel.width * sx).round() as u32).clamp(1, iw - x);
    let h = ((sel.height * sy).round() as u32).clamp(1, ih - y);
    if let Ok(mut last) = app.state::<CaptureState>().last.lock() {
        *last = Some((sel.monitor, x, y, w, h));
    }
    let crop = image::imageops::crop_imm(&shot.image, x, y, w, h).to_image();
    let mode = app
        .state::<CaptureState>()
        .mode
        .lock()
        .map(|m| *m)
        .unwrap_or_default();
    recognize_and_report(app, crop, mode);
    Ok(())
}

/// 重复上次选区（不弹遮罩，直接截取同一区域）。
pub fn repeat_last(app: &AppHandle, mode: Mode) -> Result<(), String> {
    let last = app
        .state::<CaptureState>()
        .last
        .lock()
        .map_err(|e| e.to_string())?
        .ok_or("还没有截过图")?;
    let (mi, x, y, w, h) = last;
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let m = monitors.get(mi).ok_or("屏幕已变化，请重新截图")?;
    let img = m
        .capture_region(x, y, w, h)
        .map_err(|e| format!("截屏失败：{e}"))?;
    recognize_and_report(app, img, mode);
    Ok(())
}

/// 识别剪贴板中的图片。
pub fn from_clipboard(app: &AppHandle, mode: Mode) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let data = cb.get_image().map_err(|_| "剪贴板中没有图片".to_string())?;
    let img = RgbaImage::from_raw(
        data.width as u32,
        data.height as u32,
        data.bytes.into_owned(),
    )
    .ok_or("剪贴板图片格式无效")?;
    recognize_and_report(app, img, mode);
    Ok(())
}

/// 在后台线程识别，完成后发事件、复制、唤出主窗口。
fn recognize_and_report(app: &AppHandle, img: RgbaImage, mode: Mode) {
    let app = app.clone();
    std::thread::spawn(move || {
        let ss = shot_settings(&app);
        let png = temp_dir(&app).join(format!(
            "crop-{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        let _ = save_png(&img, &png);
        let rgb: RgbImage = image::DynamicImage::ImageRgba8(img).to_rgb8();
        let mut result = CaptureResult {
            mode,
            image: png.display().to_string(),
            page: None,
            codes: vec![],
            text: String::new(),
            error: None,
        };
        match mode {
            Mode::Ocr => {
                let st = app.state::<SettingsState>();
                match app
                    .state::<EngineState>()
                    .task_engines(&st.engine(), &st.online(), Some(1))
                    .and_then(|mut e| {
                        let engine = e.remove(0);
                        engine.recognize(&rgb).map_err(|e| e.to_string())
                    }) {
                    Ok(mut page) => {
                        page.blocks = ss.parser.run(page.blocks);
                        result.text = anyocr_output::data_text(&page);
                        result.page = Some(page);
                    }
                    Err(e) => result.error = Some(e),
                }
            }
            Mode::Qrcode => match anyocr_qrcode::read(&rgb, &qr_preprocess(&app)) {
                Ok(codes) => {
                    result.text = codes
                        .iter()
                        .map(|c| c.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    result.codes = codes;
                }
                Err(e) => result.error = Some(e.to_string()),
            },
        }
        if ss.copy
            && !result.text.is_empty()
            && let Ok(mut cb) = arboard::Clipboard::new()
        {
            let _ = cb.set_text(result.text.clone());
        }
        let _ = app.emit("capture-result", result);
        restore_main(&app, ss.pop_main_window);
    });
}

#[cfg(test)]
mod tests {
    /// 需要真实桌面环境：`cargo test -p anyocr-desktop -- --ignored capture_all_monitors`
    #[test]
    #[ignore]
    fn capture_all_monitors() {
        let monitors = xcap::Monitor::all().unwrap();
        assert!(!monitors.is_empty());
        for m in monitors {
            let t = std::time::Instant::now();
            let img = m.capture_image().unwrap();
            println!(
                "屏幕 {:?} 位置 ({}, {}) 缩放 {} → {}x{}，截屏 {} ms",
                m.name().ok(),
                m.x().unwrap_or(0),
                m.y().unwrap_or(0),
                m.scale_factor().unwrap_or(1.0),
                img.width(),
                img.height(),
                t.elapsed().as_millis()
            );
            assert!(img.width() > 0 && img.height() > 0);
        }
    }
}
