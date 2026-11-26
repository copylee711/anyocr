//! 批量任务：文档（PDF 文本层 + 文本导出）与图片批量识别。进度通过 `job` 事件推送给前端。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use anyocr_core::review::{self, ReviewPool};
use anyocr_core::tbpu::Parser;
use anyocr_core::{OcrEngine, PageResult};
use anyocr_output::{Format, OutputConfig, OutputSet, Record, data_text, fill_name_template};
use anyocr_pdf::PdfTool;
use anyocr_pdf::pipeline::{
    LayerMode, PageReport, Progress, Report, ReviewSetup, SearchableOptions,
    make_searchable_with_review,
};
use serde::{Deserialize, Serialize};

pub const IMAGE_EXTS: [&str; 9] = [
    "png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff", "gif", "jfif",
];
pub const DOC_EXTS: [&str; 1] = ["pdf"];

#[derive(Default)]
pub struct Jobs {
    next: AtomicU64,
    cancels: Mutex<HashMap<u64, Arc<AtomicBool>>>,
}

impl Jobs {
    pub fn register(&self) -> (u64, Arc<AtomicBool>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut m) = self.cancels.lock() {
            m.insert(id, Arc::clone(&flag));
        }
        (id, flag)
    }

    pub fn cancel(&self, id: u64) -> bool {
        self.cancels
            .lock()
            .ok()
            .and_then(|m| m.get(&id).map(|f| f.store(true, Ordering::Relaxed)))
            .is_some()
    }

    pub fn finish(&self, id: u64) {
        if let Ok(mut m) = self.cancels.lock() {
            m.remove(&id);
        }
    }
}

/// 推送给前端的任务事件（事件名 `job`）。
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum JobEvent {
    /// 开始处理第 `file` 个文件（0 起始）。
    FileStart { job: u64, file: usize },
    /// 文档：已确定要识别的页数。
    Planned {
        job: u64,
        file: usize,
        total: usize,
        to_ocr: usize,
    },
    /// 文档：一页完成。
    Page {
        job: u64,
        file: usize,
        page: PageReport,
    },
    /// 文档：一页的 VLM 复核结果（`skipped` 为未应用的原因，如超时）。
    Reviewed {
        job: u64,
        file: usize,
        page: usize,
        accepted: usize,
        rejected: usize,
        skipped: Option<String>,
        /// 图片任务：复核后的新文本（有修改时）。
        text: Option<String>,
    },
    /// 在线文档解析的进度：阶段（uploading / pending / running / converting / downloading）与页数。
    Progress {
        job: u64,
        file: usize,
        stage: String,
        done: usize,
        total: usize,
    },
    /// 一个文件完成。图片任务的 `text` 为识别文本。
    FileDone {
        job: u64,
        file: usize,
        ok: bool,
        error: Option<String>,
        elapsed_ms: u64,
        outputs: Vec<String>,
        report: Option<Report>,
        text: Option<String>,
        blocks: usize,
    },
    /// 全部完成。
    JobDone {
        job: u64,
        cancelled: bool,
        outputs: Vec<String>,
        error: Option<String>,
    },
}

/// 事件回调（桌面端转发为 Tauri `job` 事件；测试中收集到列表）。
pub type Emit = Arc<dyn Fn(JobEvent) + Send + Sync>;

fn emit(app: &Emit, e: JobEvent) {
    app(e);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DirType {
    /// 源文件所在目录。
    #[default]
    Source,
    /// 指定目录。
    Custom,
}

/// 输出相关的公共参数。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputArgs {
    pub dir_type: DirType,
    #[serde(default)]
    pub dir: String,
    pub formats: Vec<Format>,
    pub file_name_template: String,
    pub datetime_format: String,
    pub ignore_blank: bool,
}

impl OutputArgs {
    fn dir_for(&self, source: &Path) -> PathBuf {
        match self.dir_type {
            DirType::Custom if !self.dir.trim().is_empty() => PathBuf::from(self.dir.trim()),
            _ => source.parent().map(Path::to_path_buf).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocJob {
    pub files: Vec<String>,
    pub mode: LayerMode,
    #[serde(default)]
    pub pages: String,
    pub dpi: f32,
    pub parser: Parser,
    #[serde(default)]
    pub password: Option<String>,
    /// 是否生成可搜索 PDF。
    pub write_pdf: bool,
    /// 可搜索 PDF 的文件名后缀（不含扩展名），如 `.ocr`。
    pub pdf_suffix: String,
    pub output: OutputArgs,
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into())
}

pub fn run_doc(
    app: Emit,
    pdf: Arc<PdfTool>,
    engines: Vec<Arc<dyn OcrEngine>>,
    job: u64,
    cancel: Arc<AtomicBool>,
    args: DocJob,
    review: Option<ReviewSetup>,
) {
    let mut all_outputs = Vec::new();
    let mut job_error = None;
    for (fi, file) in args.files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        emit(&app, JobEvent::FileStart { job, file: fi });
        let start = Instant::now();
        let input = PathBuf::from(file);
        let out_dir = args.output.dir_for(&input);
        let pdf_out = out_dir.join(format!("{}{}.pdf", stem(&input), args.pdf_suffix));
        let result = (|| -> Result<(Report, Vec<String>), String> {
            std::fs::create_dir_all(&out_dir).map_err(|e| format!("无法创建输出目录：{e}"))?;
            if args.write_pdf && pdf_out == input {
                return Err("输出文件与输入相同，请设置文件名后缀或输出目录".into());
            }
            let text_formats: Vec<Format> = args.output.formats.clone();
            let mut set = if text_formats.is_empty() {
                None
            } else {
                let now = chrono::Local::now();
                let name = fill_name_template(
                    &args.output.file_name_template,
                    &stem(&input),
                    &args.output.datetime_format,
                    now,
                );
                Some(
                    OutputSet::new(
                        OutputConfig {
                            dir: out_dir.clone(),
                            file_name: name,
                            formats: text_formats,
                            ignore_blank: args.output.ignore_blank,
                            individual_beside_source: args.output.dir_type == DirType::Source,
                        },
                        now,
                    )
                    .map_err(|e| e.to_string())?,
                )
            };
            let opts = SearchableOptions {
                mode: args.mode,
                pages: args.pages.clone(),
                dpi: args.dpi,
                password: args.password.clone().filter(|p| !p.is_empty()),
                parser: args.parser,
                write_pdf: args.write_pdf,
                ..Default::default()
            };
            let file_name = input
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut write_err = None;
            let mut review_log: Vec<serde_json::Value> = Vec::new();
            let report =
                make_searchable_with_review(
                    &pdf,
                    &engines,
                    review.as_ref(),
                    &input,
                    &pdf_out,
                    &opts,
                    &cancel,
                    |p| match p {
                        Progress::Planned { total, to_ocr } => emit(
                            &app,
                            JobEvent::Planned {
                                job,
                                file: fi,
                                total,
                                to_ocr,
                            },
                        ),
                        Progress::Page(page) => emit(
                            &app,
                            JobEvent::Page {
                                job,
                                file: fi,
                                page,
                            },
                        ),
                        Progress::Recognized { index, result } => {
                            if let Some(set) = set.as_mut() {
                                let r = Record {
                                    name: format!("{file_name} #{}", index + 1),
                                    path: input.clone(),
                                    page: Some(index + 1),
                                    result: Ok(result),
                                };
                                if let Err(e) = set.push(&r) {
                                    write_err.get_or_insert(e.to_string());
                                }
                            }
                        }
                        Progress::Reviewed { index, outcome } => {
                            emit(
                                &app,
                                JobEvent::Reviewed {
                                    job,
                                    file: fi,
                                    page: index + 1,
                                    accepted: outcome.accepted(),
                                    rejected: outcome.rejected(),
                                    skipped: None,
                                    text: None,
                                },
                            );
                            review_log.push(serde_json::json!({ "page": index + 1, "model": outcome.model, "elapsed_ms": outcome.elapsed_ms, "changes": outcome.changes }));
                        }
                        Progress::ReviewSkipped { index, reason } => {
                            emit(
                                &app,
                                JobEvent::Reviewed {
                                    job,
                                    file: fi,
                                    page: index + 1,
                                    accepted: 0,
                                    rejected: 0,
                                    skipped: Some(reason.clone()),
                                    text: None,
                                },
                            );
                            review_log.push(serde_json::json!({ "page": index + 1, "skipped": reason }));
                        }
                        Progress::Saving => {}
                    },
                )
                .map_err(|e| e.to_string())?;
            let mut outputs = Vec::new();
            if args.write_pdf {
                outputs.push(pdf_out.display().to_string());
            }
            // 复核修改日志（含被拒绝的修改），便于核查
            if !review_log.is_empty() {
                let path = out_dir.join(format!("{}.review.jsonl", stem(&input)));
                let body: Vec<String> = review_log.iter().map(|v| v.to_string()).collect();
                std::fs::write(&path, body.join("\n") + "\n")
                    .map_err(|e| format!("写入复核日志失败：{e}"))?;
                outputs.push(path.display().to_string());
            }
            if let Some(set) = set {
                outputs.extend(
                    set.finish()
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|p| p.display().to_string()),
                );
            }
            if let Some(e) = write_err {
                return Err(e);
            }
            Ok((report, outputs))
        })();
        let elapsed_ms = start.elapsed().as_millis() as u64;
        match result {
            Ok((report, outputs)) => {
                all_outputs.extend(outputs.clone());
                emit(
                    &app,
                    JobEvent::FileDone {
                        job,
                        file: fi,
                        ok: true,
                        error: None,
                        elapsed_ms,
                        outputs,
                        report: Some(report),
                        text: None,
                        blocks: 0,
                    },
                );
            }
            Err(e) => {
                job_error.get_or_insert(e.clone());
                emit(
                    &app,
                    JobEvent::FileDone {
                        job,
                        file: fi,
                        ok: false,
                        error: Some(e),
                        elapsed_ms,
                        outputs: vec![],
                        report: None,
                        text: None,
                        blocks: 0,
                    },
                );
            }
        }
    }
    emit(
        &app,
        JobEvent::JobDone {
            job,
            cancelled: cancel.load(Ordering::Relaxed),
            outputs: all_outputs,
            error: job_error,
        },
    );
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchJob {
    pub files: Vec<String>,
    pub parser: Parser,
    pub output: OutputArgs,
}

pub fn run_batch(
    app: Emit,
    engines: Vec<Arc<dyn OcrEngine>>,
    job: u64,
    cancel: Arc<AtomicBool>,
    args: BatchJob,
    review: Option<ReviewSetup>,
) {
    let fail = |app: &Emit, e: String| {
        emit(
            app,
            JobEvent::JobDone {
                job,
                cancelled: false,
                outputs: vec![],
                error: Some(e),
            },
        );
    };
    let Some(first) = args.files.first().map(PathBuf::from) else {
        return fail(&app, "没有文件".into());
    };
    let out_dir = args.output.dir_for(&first);
    let folder = first
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let now = chrono::Local::now();
    let name = fill_name_template(
        &args.output.file_name_template,
        &folder,
        &args.output.datetime_format,
        now,
    );
    let mut set = if args.output.formats.is_empty() {
        None
    } else {
        match OutputSet::new(
            OutputConfig {
                dir: out_dir,
                file_name: name,
                formats: args.output.formats.clone(),
                ignore_blank: args.output.ignore_blank,
                individual_beside_source: args.output.dir_type == DirType::Source,
            },
            now,
        ) {
            Ok(s) => Some(s),
            Err(e) => return fail(&app, e.to_string()),
        }
    };

    // 多个识别线程从队列取任务；主线程按原顺序写出结果
    let queue = Arc::new(Mutex::new(
        args.files
            .iter()
            .cloned()
            .enumerate()
            .collect::<Vec<_>>()
            .into_iter(),
    ));
    type Msg = (
        usize,
        Result<PageResult, String>,
        u64,
        Option<image::RgbImage>,
    );
    let (tx, rx) = mpsc::channel::<Msg>();
    let keep_image = review.is_some();
    let mut pool = review.as_ref().map(ReviewPool::start);
    let workers: Vec<_> = engines
        .into_iter()
        .map(|engine| {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let cancel = Arc::clone(&cancel);
            let app = app.clone();
            let parser = args.parser;
            std::thread::spawn(move || {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let next = queue.lock().ok().and_then(|mut q| q.next());
                    let Some((i, path)) = next else { break };
                    emit(&app, JobEvent::FileStart { job, file: i });
                    let t = Instant::now();
                    let mut kept = None;
                    let res = image::open(&path)
                        .map_err(|e| format!("无法打开图片：{e}"))
                        .and_then(|img| {
                            let rgb = img.to_rgb8();
                            let r = engine.recognize(&rgb).map_err(|e| e.to_string());
                            if keep_image {
                                kept = Some(rgb);
                            }
                            r
                        })
                        .map(|mut p| {
                            p.blocks = parser.run(p.blocks);
                            p
                        });
                    if tx
                        .send((i, res, t.elapsed().as_millis() as u64, kept))
                        .is_err()
                    {
                        break;
                    }
                }
            })
        })
        .collect();
    drop(tx);

    let mut pending: BTreeMap<usize, (Result<PageResult, String>, u64)> = BTreeMap::new();
    let mut next_write = 0usize;
    let mut write_err = None;
    let mut write_ready = |pending: &mut BTreeMap<usize, (Result<PageResult, String>, u64)>,
                           next_write: &mut usize,
                           write_err: &mut Option<String>| {
        while let Some((res, _)) = pending.remove(next_write) {
            if let Some(set) = set.as_mut() {
                let path = PathBuf::from(&args.files[*next_write]);
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Err(e) = set.push(&Record {
                    name,
                    path,
                    page: None,
                    result: res,
                }) {
                    write_err.get_or_insert(e.to_string());
                }
            }
            *next_write += 1;
        }
    };
    for (i, res, ms, image) in rx.iter() {
        let (text, blocks, error) = match &res {
            Ok(p) => (Some(data_text(p)), p.blocks.len(), None),
            Err(e) => (None, 0, Some(e.clone())),
        };
        emit(
            &app,
            JobEvent::FileDone {
                job,
                file: i,
                ok: error.is_none(),
                error,
                elapsed_ms: ms,
                outputs: vec![],
                report: None,
                text,
                blocks,
            },
        );
        if let (Some(pool), Some(img), Ok(page)) = (pool.as_mut(), image, &res) {
            pool.submit(i, img, page);
            pool.poll();
        }
        pending.insert(i, (res, ms));
        // 开启复核时等全部复核结束再按序写出，否则边识别边写
        if pool.is_none() {
            write_ready(&mut pending, &mut next_write, &mut write_err);
        }
    }
    for w in workers {
        let _ = w.join();
    }
    if let (Some(pool), Some(setup)) = (pool.as_mut(), review.as_ref()) {
        pool.finish(Instant::now() + setup.grace, &cancel);
        for (i, (res, _)) in pending.iter_mut() {
            let Ok(page) = res else { continue };
            let (accepted, rejected, skipped) = match pool.outcomes.remove(i) {
                Some(Ok(o)) => {
                    review::apply(page, &o);
                    (o.accepted(), o.rejected(), None)
                }
                Some(Err(e)) => (0, 0, Some(e)),
                None if pool.was_submitted(*i) => {
                    (0, 0, Some("超过截止时间，保留 OCR 结果".to_owned()))
                }
                None => continue,
            };
            emit(
                &app,
                JobEvent::Reviewed {
                    job,
                    file: *i,
                    page: 1,
                    accepted,
                    rejected,
                    skipped,
                    text: (accepted > 0).then(|| data_text(page)),
                },
            );
        }
        write_ready(&mut pending, &mut next_write, &mut write_err);
    }
    let outputs = match set.map(|s| s.finish()) {
        Some(Ok(files)) => files.into_iter().map(|p| p.display().to_string()).collect(),
        Some(Err(e)) => {
            write_err.get_or_insert(e.to_string());
            vec![]
        }
        None => vec![],
    };
    emit(
        &app,
        JobEvent::JobDone {
            job,
            cancelled: cancel.load(Ordering::Relaxed),
            outputs,
            error: write_err,
        },
    );
}

/// 展开文件与文件夹，返回符合扩展名的文件（去重、保持顺序；文件夹内按名称排序）。
pub fn scan_files(paths: &[String], recurse: bool, exts: &[&str]) -> Vec<String> {
    fn ok_ext(p: &Path, exts: &[&str]) -> bool {
        p.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .is_some_and(|e| exts.contains(&e.as_str()))
    }
    fn walk(dir: &Path, recurse: bool, exts: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if recurse {
                    walk(&p, recurse, exts, out);
                }
            } else if ok_ext(&p, exts) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        let p = PathBuf::from(p);
        if p.is_dir() {
            walk(&p, recurse, exts, &mut out);
        } else if ok_ext(&p, exts) {
            out.push(p);
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.into_iter()
        .filter(|p| seen.insert(p.clone()))
        .map(|p| p.display().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    //! 任务流水线集成测试：用返回固定结果的假引擎，不依赖模型与窗口。
    use super::*;
    use anyocr_core::{EngineError, EngineInfo, EngineKind, TextBlock};

    struct FakeEngine(EngineInfo);

    impl FakeEngine {
        fn shared() -> Arc<dyn OcrEngine> {
            Arc::new(Self(EngineInfo {
                id: "fake".into(),
                name: "fake".into(),
                kind: EngineKind::Local,
                key_url: None,
                homepage: None,
                has_boxes: true,
            }))
        }
    }

    impl OcrEngine for FakeEngine {
        fn info(&self) -> &EngineInfo {
            &self.0
        }
        fn recognize(&self, img: &image::RgbImage) -> Result<PageResult, EngineError> {
            let (w, h) = (img.width() as f32, img.height() as f32);
            Ok(PageResult {
                width: img.width(),
                height: img.height(),
                blocks: vec![
                    TextBlock::from_rect(
                        format!("宽{}", img.width()),
                        [w * 0.1, h * 0.1, w * 0.5, h * 0.15],
                        0.9,
                    ),
                    TextBlock::from_rect(
                        "第二行 hello",
                        [w * 0.1, h * 0.2, w * 0.6, h * 0.25],
                        0.8,
                    ),
                ],
                elapsed_ms: 1,
                engine: "fake".into(),
            })
        }
    }

    fn collector() -> (Emit, Arc<Mutex<Vec<JobEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let e2 = Arc::clone(&events);
        (Arc::new(move |e| e2.lock().unwrap().push(e)), events)
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join("anyocr-jobs-test").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn output(dir: &Path, formats: Vec<Format>) -> OutputArgs {
        OutputArgs {
            dir_type: DirType::Custom,
            dir: dir.display().to_string(),
            formats,
            file_name_template: "res".into(),
            datetime_format: "%Y".into(),
            ignore_blank: false,
        }
    }

    #[test]
    fn events_use_camel_case_for_frontend() {
        let v = serde_json::to_value(JobEvent::Planned {
            job: 1,
            file: 0,
            total: 3,
            to_ocr: 2,
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "kind": "planned", "job": 1, "file": 0, "total": 3, "toOcr": 2 })
        );
        let v = serde_json::to_value(JobEvent::JobDone {
            job: 1,
            cancelled: false,
            outputs: vec![],
            error: None,
        })
        .unwrap();
        assert_eq!(v["kind"], "job_done");
    }

    /// 把“宽N”改成“宽N（已复核）”的假复核器。
    struct FakeReviewer;

    impl anyocr_core::review::Reviewer for FakeReviewer {
        fn name(&self) -> &str {
            "fake"
        }
        fn review(
            &self,
            _: &image::RgbImage,
            page: &PageResult,
        ) -> Result<anyocr_core::ReviewOutcome, EngineError> {
            let b = &page.blocks[0];
            Ok(anyocr_core::ReviewOutcome {
                changes: vec![anyocr_core::LineChange {
                    index: 0,
                    before: b.text.clone(),
                    after: format!("{}（已复核）", b.text),
                    accepted: true,
                    reason: None,
                }],
                elapsed_ms: 1,
                model: "fake".into(),
            })
        }
    }

    #[test]
    fn batch_job_applies_review_before_writing() {
        let dir = tmp("batch-review");
        let p = dir.join("a.png");
        image::RgbImage::from_pixel(120, 80, image::Rgb([255, 255, 255]))
            .save(&p)
            .unwrap();
        let out = dir.join("out");
        let (emit, events) = collector();
        let args = BatchJob {
            files: vec![p.display().to_string()],
            parser: Parser::None,
            output: output(&out, vec![Format::Txt]),
        };
        let review = anyocr_core::review::ReviewSetup {
            reviewer: Arc::new(FakeReviewer),
            concurrency: 1,
            grace: std::time::Duration::from_secs(5),
            low_conf: None,
        };
        run_batch(
            emit,
            vec![FakeEngine::shared()],
            9,
            Arc::new(AtomicBool::new(false)),
            args,
            Some(review),
        );
        let ev = events.lock().unwrap();
        assert!(ev.iter().any(|e| matches!(
            e,
            JobEvent::Reviewed { accepted: 1, text: Some(t), .. } if t.contains("宽120（已复核）")
        )));
        let txt = std::fs::read_to_string(out.join("res.txt")).unwrap();
        assert!(txt.contains("宽120（已复核）"), "{txt}");
    }

    #[test]
    fn batch_job_writes_outputs_in_order() {
        let dir = tmp("batch");
        let mut files = vec![];
        for (i, w) in [300u32, 200, 100].iter().enumerate() {
            let p = dir.join(format!("img{i}.png"));
            image::RgbImage::from_pixel(*w, 80, image::Rgb([255, 255, 255]))
                .save(&p)
                .unwrap();
            files.push(p.display().to_string());
        }
        let bad = dir.join("broken.png");
        std::fs::write(&bad, b"not an image").unwrap();
        files.insert(1, bad.display().to_string());

        let out = dir.join("out");
        let (emit, events) = collector();
        let args = BatchJob {
            files,
            parser: Parser::None,
            output: output(&out, vec![Format::Txt, Format::Csv, Format::TxtIndividual]),
        };
        // 两个“线程”的假引擎，结果乱序到达，写出时须恢复原顺序
        run_batch(
            emit,
            vec![FakeEngine::shared(), FakeEngine::shared()],
            7,
            Arc::new(AtomicBool::new(false)),
            args,
            None,
        );

        let ev = events.lock().unwrap();
        let done: Vec<_> = ev
            .iter()
            .filter(|e| matches!(e, JobEvent::FileDone { .. }))
            .collect();
        assert_eq!(done.len(), 4);
        assert!(ev.iter().any(|e| matches!(
            e,
            JobEvent::FileDone {
                file: 1,
                ok: false,
                ..
            }
        )));
        let Some(JobEvent::JobDone {
            outputs,
            error,
            cancelled,
            ..
        }) = ev.last()
        else {
            panic!("缺少 JobDone")
        };
        assert!(!cancelled);
        assert!(error.is_none(), "{error:?}");
        assert!(outputs.iter().any(|o| o.ends_with("res.txt")));

        let txt = std::fs::read_to_string(out.join("res.txt")).unwrap();
        let p0 = txt.find("img0.png").unwrap();
        let p1 = txt.find("broken.png").unwrap();
        let p2 = txt.find("img1.png").unwrap();
        assert!(p0 < p1 && p1 < p2, "顺序错误：{txt}");
        assert!(txt.contains("宽300\n第二行 hello"));
        assert!(txt.contains("OCR识别失败"));
        assert!(out.join("img2.txt").exists());
        assert!(
            std::fs::read_to_string(out.join("res.csv"))
                .unwrap()
                .contains("宽100")
        );
    }

    #[test]
    fn doc_job_creates_searchable_pdf_and_text() {
        use lopdf::{Document, Object, Stream, dictionary};
        let dir = tmp("doc");
        let src = dir.join("scan.pdf");
        // 两页空白 PDF（无文字），模拟扫描件
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = vec![];
        for _ in 0..2 {
            let c = doc.add_object(Stream::new(dictionary! {}, b"".to_vec()));
            kids.push(Object::Reference(doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Contents" => c,
                "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()],
            })));
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 2 }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", cat);
        doc.save(&src).unwrap();

        let out = dir.join("out");
        let (emit, events) = collector();
        let pdf = Arc::new(PdfTool::new().expect("需要先运行 scripts/fetch-pdfium.ps1"));
        let args = DocJob {
            files: vec![src.display().to_string()],
            mode: LayerMode::SkipText,
            pages: String::new(),
            dpi: 72.0,
            parser: Parser::MultiPara,
            password: None,
            write_pdf: true,
            pdf_suffix: ".ocr".into(),
            output: output(&out, vec![Format::Txt]),
        };
        run_doc(
            emit,
            Arc::clone(&pdf),
            vec![FakeEngine::shared()],
            3,
            Arc::new(AtomicBool::new(false)),
            args,
            None,
        );

        let ev = events.lock().unwrap();
        assert_eq!(
            ev.iter()
                .filter(|e| matches!(e, JobEvent::Page { .. }))
                .count(),
            2
        );
        let Some(JobEvent::JobDone { outputs, error, .. }) = ev.last() else {
            panic!("缺少 JobDone")
        };
        assert!(error.is_none(), "{error:?}");
        let out_pdf = out.join("scan.ocr.pdf");
        assert!(outputs.contains(&out_pdf.display().to_string()));
        let text = pdf.page_text(&out_pdf, None, 1).unwrap();
        assert!(text.contains("第二行"), "提取文本：{text:?}");
        let txt = std::fs::read_to_string(out.join("res.txt")).unwrap();
        assert!(txt.contains("≦ scan.pdf #1 ≧") && txt.contains("≦ scan.pdf #2 ≧"));
    }
}
