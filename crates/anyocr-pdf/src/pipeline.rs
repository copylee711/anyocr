//! 扫描件 PDF → 可搜索 PDF 的处理流水线。
//!
//! 主线程用 PDFium 逐页渲染，多个识别线程并行 OCR（每个线程一个引擎实例），
//! 结果经排版解析后写入不可见文字层，最后增量保存。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyocr_core::OcrEngine;
use anyocr_core::tbpu::Parser;
use serde::{Deserialize, Serialize};

use crate::textlayer::TextLayerWriter;
use crate::{PdfError, PdfTool, RenderedPage, parse_page_ranges};

/// 已有文字的页面如何处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LayerMode {
    /// 跳过已有文字的页面（默认，等同 OCRmyPDF `--skip-text`）。
    #[default]
    SkipText,
    /// 所有选中页面都识别（已有 AnyOCR 文字层的会先移除，避免重复）。
    Force,
    /// 移除 AnyOCR 之前写入的文字层后重新识别；含其他原生文字的页面仍跳过。
    Redo,
}

impl std::str::FromStr for LayerMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "skip-text" | "skip" => Ok(LayerMode::SkipText),
            "force" => Ok(LayerMode::Force),
            "redo" => Ok(LayerMode::Redo),
            _ => Err(format!("未知模式：{s}，可选：skip-text、force、redo")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchableOptions {
    pub mode: LayerMode,
    /// 页码范围（1 起始，如 `1-3,8`），空为全部页。
    pub pages: String,
    /// 渲染 DPI。
    pub dpi: f32,
    /// 渲染短边下限（像素）。
    pub min_side: u32,
    pub password: Option<String>,
    /// 文字层的排版解析方案（影响提取 / 复制时的文字顺序）。
    pub parser: Parser,
    /// 页面已有不少于该数量的非空白字符时视为“已有文字”。
    pub text_threshold: usize,
    /// 是否写出带文字层的 PDF；为假时只识别（结果通过 [`Progress::Recognized`] 取得）。
    pub write_pdf: bool,
}

impl Default for SearchableOptions {
    fn default() -> Self {
        Self {
            mode: LayerMode::SkipText,
            pages: String::new(),
            dpi: 200.0,
            min_side: 1080,
            password: None,
            parser: Parser::MultiPara,
            text_threshold: 10,
            write_pdf: true,
        }
    }
}

/// 单页处理结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PageStatus {
    /// 已有文字，跳过。
    Skipped { text_chars: usize },
    /// 已识别并写入文字层。
    Done {
        blocks: usize,
        chars: usize,
        ocr_ms: u64,
        score: f32,
    },
    /// 识别失败。
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageReport {
    /// 1 起始页码。
    pub page: usize,
    #[serde(flatten)]
    pub status: PageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Report {
    pub total_pages: usize,
    pub pages: Vec<PageReport>,
    pub elapsed_ms: u64,
    pub render_ms: u64,
    pub ocr_ms: u64,
    pub input_bytes: u64,
    pub output_bytes: u64,
}

impl Report {
    pub fn done(&self) -> usize {
        self.pages
            .iter()
            .filter(|p| matches!(p.status, PageStatus::Done { .. }))
            .count()
    }
    pub fn skipped(&self) -> usize {
        self.pages
            .iter()
            .filter(|p| matches!(p.status, PageStatus::Skipped { .. }))
            .count()
    }
    pub fn failed(&self) -> usize {
        self.pages
            .iter()
            .filter(|p| matches!(p.status, PageStatus::Failed { .. }))
            .count()
    }
}

/// 进度事件。
#[derive(Debug, Clone)]
pub enum Progress {
    /// 已确定需要识别的页数。
    Planned { total: usize, to_ocr: usize },
    /// 一页处理完成（含跳过与失败）。
    Page(PageReport),
    /// 一页的识别结果（按页序在保存前依次给出），供导出 txt / md 等格式。
    Recognized {
        index: usize,
        result: anyocr_core::PageResult,
    },
    /// 正在保存。
    Saving,
}

/// 把扫描件 PDF 转为可搜索 PDF。`engines` 的个数即并行识别线程数。
pub fn make_searchable(
    pdf: &PdfTool,
    engines: &[Arc<dyn OcrEngine>],
    input: &Path,
    output: &Path,
    opts: &SearchableOptions,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(Progress),
) -> Result<Report, PdfError> {
    let start = Instant::now();
    let password = opts.password.as_deref();
    let mut writer = TextLayerWriter::open(input, password)?;
    let count = writer.page_count();
    let selected = parse_page_ranges(&opts.pages, count).map_err(PdfError::Open)?;
    let text_counts = pdf.page_text_counts(input, password)?;

    let mut reports: Vec<PageReport> = Vec::new();
    let mut to_ocr = Vec::new();
    for &i in &selected {
        let own = writer.has_own_layer(i);
        let chars = text_counts.get(i).copied().unwrap_or(0);
        let skip = match opts.mode {
            LayerMode::Force => false,
            // 已有 AnyOCR 文字层：skip-text 视为有文字；redo 重新识别
            LayerMode::SkipText => chars >= opts.text_threshold,
            LayerMode::Redo => !own && chars >= opts.text_threshold,
        };
        if skip {
            let r = PageReport {
                page: i + 1,
                status: PageStatus::Skipped { text_chars: chars },
            };
            reports.push(r);
        } else {
            if own {
                writer.remove_own_layer(i)?;
            }
            to_ocr.push(i);
        }
    }
    on_progress(Progress::Planned {
        total: selected.len(),
        to_ocr: to_ocr.len(),
    });
    for r in &reports {
        on_progress(Progress::Page(r.clone()));
    }

    let mut render_ms = 0u64;
    let mut ocr_ms = 0u64;
    if !to_ocr.is_empty() && !engines.is_empty() {
        let (tx, rx) = mpsc::sync_channel::<RenderedPage>(engines.len() * 2);
        let rx = Arc::new(Mutex::new(rx));
        let (rtx, rrx) = mpsc::channel::<(usize, Result<anyocr_core::PageResult, String>)>();
        let parser = opts.parser;
        let workers: Vec<_> = engines
            .iter()
            .map(|engine| {
                let engine = Arc::clone(engine);
                let rx = Arc::clone(&rx);
                let rtx = rtx.clone();
                std::thread::spawn(move || {
                    loop {
                        let job = rx.lock().ok().and_then(|r| r.recv().ok());
                        let Some(page) = job else { break };
                        let res = engine
                            .recognize(&page.image)
                            .map(|mut p| {
                                p.blocks = parser.run(p.blocks);
                                p
                            })
                            .map_err(|e| e.to_string());
                        if rtx.send((page.index, res)).is_err() {
                            break;
                        }
                    }
                })
            })
            .collect();
        drop(rtx);

        // 主线程渲染；同时非阻塞地收取识别结果，及时推送进度
        let mut results = Vec::with_capacity(to_ocr.len());
        let drain = |results: &mut Vec<(usize, Result<anyocr_core::PageResult, String>)>,
                     on_progress: &mut dyn FnMut(Progress)| {
            while let Ok((i, r)) = rrx.try_recv() {
                on_progress(Progress::Page(page_report(i, &r)));
                results.push((i, r));
            }
        };
        let mut t = Instant::now();
        let render_result =
            pdf.render_pages(input, password, &to_ocr, opts.dpi, opts.min_side, |p| {
                render_ms += t.elapsed().as_millis() as u64;
                if cancel.load(Ordering::Relaxed) {
                    return false;
                }
                let ok = tx.send(p).is_ok();
                drain(&mut results, &mut on_progress);
                t = Instant::now();
                ok
            });
        drop(tx);
        for (i, r) in rrx.iter() {
            on_progress(Progress::Page(page_report(i, &r)));
            results.push((i, r));
        }
        for w in workers {
            let _ = w.join();
        }
        render_result?;
        if cancel.load(Ordering::Relaxed) {
            return Err(PdfError::Cancelled);
        }

        results.sort_by_key(|r| r.0);
        for (i, r) in results {
            let rep = page_report(i, &r);
            if let Ok(page) = r {
                ocr_ms += page.elapsed_ms;
                if opts.write_pdf {
                    writer.add_page(i, &page)?;
                }
                on_progress(Progress::Recognized {
                    index: i,
                    result: page,
                });
            }
            reports.push(rep);
        }
    }

    reports.sort_by_key(|r| r.page);
    if opts.write_pdf {
        on_progress(Progress::Saving);
        writer.save(output)?;
    }
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    Ok(Report {
        total_pages: count,
        pages: reports,
        elapsed_ms: start.elapsed().as_millis() as u64,
        render_ms,
        ocr_ms,
        input_bytes: size(input),
        output_bytes: size(output),
    })
}

fn page_report(index: usize, r: &Result<anyocr_core::PageResult, String>) -> PageReport {
    let status = match r {
        Ok(p) => PageStatus::Done {
            blocks: p.blocks.len(),
            chars: p
                .blocks
                .iter()
                .map(|b| b.text.chars().filter(|c| !c.is_whitespace()).count())
                .sum(),
            ocr_ms: p.elapsed_ms,
            score: p.mean_score(),
        },
        Err(e) => PageStatus::Failed { error: e.clone() },
    };
    PageReport {
        page: index + 1,
        status,
    }
}
