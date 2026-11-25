//! 前端可调用的命令。重计算放到后台线程，避免卡住 UI 事件循环。

use std::sync::{Arc, Mutex};

use anyocr_core::PageResult;
use anyocr_core::tbpu::Parser;
use anyocr_pdf::PdfTool;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::engines::EngineState;
use crate::jobs::{self, BatchJob, DocJob, Jobs};
use crate::settings::SettingsState;

/// 进程内共享的 PDFium 实例（首次使用时绑定动态库）。
#[derive(Default)]
pub struct PdfState(Mutex<Option<Arc<PdfTool>>>);

impl PdfState {
    fn get(&self) -> Result<Arc<PdfTool>, String> {
        let mut g = self.0.lock().map_err(|_| "PDF 状态已损坏".to_string())?;
        if let Some(p) = g.as_ref() {
            return Ok(Arc::clone(p));
        }
        let p = Arc::new(PdfTool::new().map_err(|e| e.to_string())?);
        *g = Some(Arc::clone(&p));
        Ok(p)
    }
}

#[tauri::command]
pub fn get_settings(settings: State<'_, SettingsState>) -> Value {
    settings.get()
}

#[tauri::command]
pub fn save_settings(settings: State<'_, SettingsState>, value: Value) -> Result<(), String> {
    settings.save(value)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    models_dir: String,
}

#[tauri::command]
pub fn app_info(app: AppHandle, engines: State<'_, EngineState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        models_dir: engines.models_dir().display().to_string(),
    }
}

#[tauri::command]
pub async fn ocr_image_file(
    settings: State<'_, SettingsState>,
    engines: State<'_, EngineState>,
    path: String,
    parser: String,
) -> Result<PageResult, String> {
    let parser: Parser = parser.parse()?;
    let engine = engines.engines(&settings.engine(), 1)?.remove(0);
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

/// 展开拖入或选择的文件 / 文件夹。`kind` 为 `image` 或 `doc`。
#[tauri::command]
pub async fn scan_files(paths: Vec<String>, recurse: bool, kind: String) -> Vec<String> {
    let exts: &[&str] = if kind == "doc" {
        &jobs::DOC_EXTS
    } else {
        &jobs::IMAGE_EXTS
    };
    let exts: Vec<&'static str> = exts.to_vec();
    tauri::async_runtime::spawn_blocking(move || jobs::scan_files(&paths, recurse, &exts))
        .await
        .unwrap_or_default()
}

fn emitter(app: &AppHandle) -> jobs::Emit {
    let app = app.clone();
    Arc::new(move |e| {
        let _ = app.emit("job", e);
    })
}

/// 在任务线程中加载引擎（首次可能需要下载模型），失败时以 `JobDone` 事件报告。
fn load_engines(app: &AppHandle, job: u64) -> Option<Vec<Arc<dyn anyocr_core::OcrEngine>>> {
    let es = app.state::<SettingsState>().engine();
    match app.state::<EngineState>().engines(&es, es.worker_count()) {
        Ok(e) => Some(e),
        Err(e) => {
            let _ = app.emit(
                "job",
                jobs::JobEvent::JobDone {
                    job,
                    cancelled: false,
                    outputs: vec![],
                    error: Some(format!("引擎加载失败：{e}")),
                },
            );
            None
        }
    }
}

/// 启动文档任务，立即返回任务 id；进度通过 `job` 事件推送。
#[tauri::command]
pub fn doc_start(
    app: AppHandle,
    pdf: State<'_, PdfState>,
    jobs_state: State<'_, Arc<Jobs>>,
    args: DocJob,
) -> Result<u64, String> {
    let pdf = pdf.get()?;
    let jobs_state = Arc::clone(&jobs_state);
    let (job, cancel) = jobs_state.register();
    std::thread::spawn(move || {
        if let Some(engines) = load_engines(&app, job) {
            jobs::run_doc(emitter(&app), pdf, engines, job, cancel, args);
        }
        jobs_state.finish(job);
    });
    Ok(job)
}

/// 启动图片批量任务。
#[tauri::command]
pub fn batch_start(app: AppHandle, jobs_state: State<'_, Arc<Jobs>>, args: BatchJob) -> u64 {
    let jobs_state = Arc::clone(&jobs_state);
    let (job, cancel) = jobs_state.register();
    std::thread::spawn(move || {
        if let Some(engines) = load_engines(&app, job) {
            jobs::run_batch(emitter(&app), engines, job, cancel, args);
        }
        jobs_state.finish(job);
    });
    job
}

#[tauri::command]
pub fn job_cancel(jobs_state: State<'_, Arc<Jobs>>, job: u64) -> bool {
    jobs_state.cancel(job)
}
