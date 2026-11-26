//! `anyocr pdf`：扫描件 PDF → 可搜索 PDF。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use anyocr_core::OcrEngine;
use anyocr_core::tbpu::Parser as Tbpu;
use anyocr_engines::Device;
use anyocr_pdf::PdfTool;
use anyocr_pdf::pipeline::{LayerMode, PageStatus, Progress, SearchableOptions, make_searchable};
use clap::Args;

use super::EngineArgs;

#[derive(Args)]
pub struct PdfArgs {
    /// 输入 PDF
    input: PathBuf,
    /// 输出 PDF；默认为输入同目录下的 `<文件名>.ocr.pdf`
    output: Option<PathBuf>,
    #[command(flatten)]
    engine: EngineArgs,
    /// 已有文字的页面：skip-text（跳过）/ force（全部识别）/ redo（重做 AnyOCR 文字层）
    #[arg(long, default_value = "skip-text")]
    mode: LayerMode,
    /// 页码范围（1 起始），如 1-20,25；默认全部
    #[arg(long, default_value = "")]
    pages: String,
    /// 渲染 DPI
    #[arg(long, default_value_t = 200.0)]
    dpi: f32,
    /// 并行识别线程数；默认 GPU 为 1，CPU 为 2
    #[arg(long)]
    workers: Option<usize>,
    /// 文档密码
    #[arg(long)]
    password: Option<String>,
    /// 文字层的排版解析方案（影响复制时的文字顺序）
    #[arg(long, default_value = "multi_para")]
    parser: Tbpu,
    /// 完成后重新打开输出文件，校验文字可提取、可搜索
    #[arg(long)]
    verify: bool,
    /// 将处理报告（JSON）写入该文件
    #[arg(long)]
    report: Option<PathBuf>,
}

fn default_output(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into());
    input.with_file_name(format!("{stem}.ocr.pdf"))
}

pub fn run(args: PdfArgs) -> Result<()> {
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| default_output(&args.input));
    if output == args.input {
        bail!("输出路径不能与输入相同");
    }
    let workers = args
        .workers
        .unwrap_or(if !args.engine.uses_local() {
            // 纯在线引擎：瓶颈在网络往返，适当并发
            4
        } else if args.engine.device == Device::Cpu {
            2
        } else {
            1
        })
        .max(1);
    let engines: Vec<Arc<dyn OcrEngine>> = args.engine.build_many(workers)?;
    let pdf = PdfTool::new()?;

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = Arc::clone(&cancel);
        let _ = ctrlc::set_handler(move || {
            eprintln!("\n正在取消…");
            cancel.store(true, Ordering::Relaxed);
        });
    }

    let opts = SearchableOptions {
        mode: args.mode,
        pages: args.pages.clone(),
        dpi: args.dpi,
        password: args.password.clone(),
        parser: args.parser,
        ..Default::default()
    };
    let mut total = 0usize;
    let mut finished = 0usize;
    let report = make_searchable(
        &pdf,
        &engines,
        &args.input,
        &output,
        &opts,
        &cancel,
        |p| match p {
            Progress::Planned { total: t, to_ocr } => {
                total = t;
                eprintln!("共选中 {t} 页，需识别 {to_ocr} 页，{workers} 个识别线程");
            }
            Progress::Page(r) => {
                finished += 1;
                let msg = match &r.status {
                    PageStatus::Skipped { text_chars } => {
                        format!("已有文字（{text_chars} 字），跳过")
                    }
                    PageStatus::Done {
                        blocks,
                        chars,
                        ocr_ms,
                        score,
                    } => {
                        format!("{blocks} 行 {chars} 字，{ocr_ms} ms，置信度 {score:.3}")
                    }
                    PageStatus::Failed { error } => format!("失败：{error}"),
                };
                eprintln!("[{finished:>4}/{total}] 第 {:>4} 页  {msg}", r.page);
            }
            Progress::Saving => eprintln!("正在保存 {}", output.display()),
            Progress::Recognized { .. } => {}
        },
    )
    .context("处理失败")?;

    let mb = |b: u64| b as f64 / 1024.0 / 1024.0;
    println!(
        "完成：识别 {} 页，跳过 {} 页，失败 {} 页；总耗时 {:.1} s（渲染 {:.1} s，OCR 累计 {:.1} s）；大小 {:.2} MB → {:.2} MB（+{:.1}%）",
        report.done(),
        report.skipped(),
        report.failed(),
        report.elapsed_ms as f64 / 1000.0,
        report.render_ms as f64 / 1000.0,
        report.ocr_ms as f64 / 1000.0,
        mb(report.input_bytes),
        mb(report.output_bytes),
        (report.output_bytes as f64 / report.input_bytes.max(1) as f64 - 1.0) * 100.0
    );
    println!("输出：{}", output.display());

    if args.verify {
        verify(&pdf, &output, args.password.as_deref(), &report)?;
    }
    if let Some(path) = &args.report {
        std::fs::write(path, serde_json::to_string_pretty(&report)?)?;
    }
    Ok(())
}

/// 校验：识别过的页面中，文字层可被 PDFium 提取，且与 OCR 字数相符；抽样子串可被搜索到。
fn verify(
    pdf: &PdfTool,
    output: &Path,
    password: Option<&str>,
    report: &anyocr_pdf::pipeline::Report,
) -> Result<()> {
    let mut checked = 0usize;
    let mut ratio_sum = 0f64;
    let mut probes = 0usize;
    let mut hits = 0usize;
    for r in &report.pages {
        let PageStatus::Done { chars, .. } = r.status else {
            continue;
        };
        if chars == 0 {
            continue;
        }
        let raw = pdf.page_text(output, password, r.page - 1)?;
        let n = raw.chars().filter(|c| !c.is_whitespace()).count();
        ratio_sum += (n.min(chars) as f64) / (n.max(chars) as f64);
        checked += 1;
        // 抽样：在首、中、末三行内各取 4 字（同一行内，跨行词组在任何阅读器里都搜不到），
        // 用 PDFium 的搜索（即 Chrome / Edge Ctrl+F 的实现）查找
        let lines: Vec<Vec<char>> = raw
            .split(['\r', '\n'])
            .map(|l| l.trim().chars().collect::<Vec<char>>())
            .filter(|l| l.len() >= 4)
            .collect();
        if !lines.is_empty() {
            for li in [0, lines.len() / 2, lines.len() - 1] {
                let line = &lines[li];
                let start = (line.len() - 4) / 2;
                let probe: String = line[start..start + 4].iter().collect();
                probes += 1;
                if pdf.search_count(output, password, r.page - 1, &probe)? > 0 {
                    hits += 1;
                }
            }
        }
    }
    if checked == 0 {
        println!("校验：没有识别过的页面");
        return Ok(());
    }
    let avg = ratio_sum / checked as f64;
    println!(
        "校验：{checked} 页可提取文字，字数一致率 {:.1}%，抽样搜索命中 {hits}/{probes}",
        avg * 100.0
    );
    if avg < 0.95 {
        bail!("校验未通过：文字层可提取字数与识别结果相差过大");
    }
    Ok(())
}
