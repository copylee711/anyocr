//! 前端可调用的命令。重计算放到阻塞线程池，避免卡住 UI 事件循环。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyocr_core::tbpu::Parser;
use anyocr_core::{OcrEngine, PageResult};
use anyocr_engines::{LocalConfig, LocalEngine};

/// 懒加载的本地引擎：首次识别时加载（必要时下载模型）。
pub struct EngineState {
    models_dir: PathBuf,
    local: Mutex<Option<Arc<LocalEngine>>>,
}

impl EngineState {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            models_dir,
            local: Mutex::new(None),
        }
    }

    fn local(&self) -> Result<Arc<LocalEngine>, String> {
        let mut guard = self
            .local
            .lock()
            .map_err(|_| "引擎状态已损坏".to_string())?;
        if let Some(e) = guard.as_ref() {
            return Ok(Arc::clone(e));
        }
        let engine = LocalEngine::new(LocalConfig {
            models_dir: Some(self.models_dir.clone()),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        let engine = Arc::new(engine);
        *guard = Some(Arc::clone(&engine));
        Ok(engine)
    }
}

#[tauri::command]
pub async fn ocr_image_file(
    state: tauri::State<'_, EngineState>,
    path: String,
    parser: String,
) -> Result<PageResult, String> {
    let parser: Parser = parser.parse()?;
    let engine = state.local()?;
    tauri::async_runtime::spawn_blocking(move || {
        let img = image::open(&path)
            .map_err(|e| format!("无法打开图片：{e}"))?
            .to_rgb8();
        let mut page = engine.recognize(&img).map_err(|e| e.to_string())?;
        page.blocks = parser.run(page.blocks);
        Ok(page)
    })
    .await
    .map_err(|e| e.to_string())?
}
