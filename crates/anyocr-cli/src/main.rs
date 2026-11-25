//! AnyOCR 命令行。
//!
//! ```text
//! anyocr ocr a.png b.jpg --parser multi_para --format json
//! ```

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use anyocr_core::OcrEngine;
use anyocr_core::tbpu::Parser as Tbpu;
use anyocr_engines::{Device, LocalConfig, LocalEngine, LocalModel};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "anyocr", version, about = "AnyOCR：本地优先的 OCR 工具")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 识别图片中的文字
    Ocr(OcrArgs),
    /// 对 PDF 页面做识别测速（渲染 + OCR），输出每页用时与汇总
    Bench(BenchArgs),
    /// 为扫描件 PDF 添加不可见文字层，生成可搜索 PDF（类似 ocrmypdf）
    Pdf(pdf::PdfArgs),
}

#[derive(Args)]
struct BenchArgs {
    /// PDF 路径
    pdf: PathBuf,
    #[command(flatten)]
    engine: EngineArgs,
    /// 页码范围（1 起始），如 1-20；默认全部
    #[arg(long, default_value = "")]
    pages: String,
    /// 渲染 DPI
    #[arg(long, default_value_t = 200.0)]
    dpi: f32,
    /// 并行识别的引擎实例数
    #[arg(long, default_value_t = 1)]
    workers: usize,
    /// 将每页识别文本写入该目录（page-0001.txt …）
    #[arg(long)]
    dump: Option<PathBuf>,
}

#[derive(Args)]
pub(crate) struct EngineArgs {
    /// 本地模型：pp-ocrv6-tiny / pp-ocrv6-small / pp-ocrv6-medium / pp-ocrv5-mobile / pp-ocrv5-server
    #[arg(long, default_value = "pp-ocrv6-small")]
    pub(crate) model: LocalModel,
    /// 推理设备：auto / cpu / directml[:N] / cuda[:N]
    #[arg(long, default_value = "auto")]
    pub(crate) device: Device,
    /// CPU 线程数
    #[arg(long)]
    threads: Option<usize>,
    /// 模型目录（默认 %APPDATA%/anyocr/models）
    #[arg(long)]
    models_dir: Option<PathBuf>,
    /// 启用文本行方向分类
    #[arg(long)]
    textline_orientation: bool,
}

impl EngineArgs {
    pub(crate) fn build(&self) -> Result<LocalEngine> {
        let models_dir = self
            .models_dir
            .clone()
            .or_else(|| dirs::data_dir().map(|d| d.join("anyocr").join("models")));
        let t = Instant::now();
        let engine = LocalEngine::new(LocalConfig {
            model: self.model,
            device: self.device,
            models_dir,
            threads: self.threads,
            textline_orientation: self.textline_orientation,
            ..Default::default()
        })
        .context("加载本地引擎失败")?;
        tracing::info!("引擎加载耗时 {:?}", t.elapsed());
        Ok(engine)
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// 纯文本
    Text,
    /// 每张图一行 JSON（与 Umi-OCR 结果字段兼容）
    Json,
}

#[derive(Args)]
struct OcrArgs {
    /// 图片路径
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    #[command(flatten)]
    engine: EngineArgs,
    /// 排版解析方案：multi_para / multi_line / multi_none / single_para / single_line / single_none / single_code / none
    #[arg(long, default_value = "multi_para")]
    parser: Tbpu,
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
}

fn run_ocr(args: OcrArgs) -> Result<()> {
    let engine = args.engine.build()?;
    for path in &args.inputs {
        let img = image::open(path)
            .with_context(|| format!("无法打开图片 {}", path.display()))?
            .to_rgb8();
        let mut page = engine.recognize(&img)?;
        page.blocks = args.parser.run(page.blocks);
        match args.format {
            Format::Text => {
                if args.inputs.len() > 1 {
                    println!("===== {} =====", path.display());
                }
                print!("{}", page.text());
            }
            Format::Json => {
                let mut v = serde_json::to_value(&page)?;
                v["path"] = serde_json::Value::String(path.display().to_string());
                println!("{}", serde_json::to_string(&v)?);
            }
        }
        tracing::info!(
            "{}：{} 个文本块，识别 {} ms",
            path.display(),
            page.blocks.len(),
            page.elapsed_ms
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "anyocr=info,warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::Ocr(a) => run_ocr(a),
        Command::Bench(a) => bench::run(a),
        Command::Pdf(a) => pdf::run(a),
    }
}

mod pdf;

mod bench {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use anyhow::{Context, Result};
    use anyocr_core::{OcrEngine, PageResult};
    use anyocr_pdf::{PdfTool, RenderedPage, parse_page_ranges};

    use super::BenchArgs;

    pub fn run(args: BenchArgs) -> Result<()> {
        let pdf = PdfTool::new()?;
        let count = pdf.page_count(&args.pdf, None)?;
        let pages = parse_page_ranges(&args.pages, count).map_err(anyhow::Error::msg)?;
        if let Some(d) = &args.dump {
            std::fs::create_dir_all(d).context("无法创建输出目录")?;
        }
        let workers = args.workers.max(1);
        let engines = (0..workers)
            .map(|_| args.engine.build())
            .collect::<Result<Vec<_>>>()?;
        eprintln!(
            "文档 {} 页，测试 {} 页，DPI {}，{} 个引擎实例",
            count,
            pages.len(),
            args.dpi,
            workers
        );

        let total = Instant::now();
        let (tx, rx) = mpsc::sync_channel::<RenderedPage>(workers * 2);
        let rx = Arc::new(Mutex::new(rx));
        let (rtx, rrx) = mpsc::channel::<(usize, PageResult, u128)>();
        let handles: Vec<_> = engines
            .into_iter()
            .map(|engine| {
                let rx = Arc::clone(&rx);
                let rtx = rtx.clone();
                std::thread::spawn(move || {
                    loop {
                        let job = rx.lock().ok().and_then(|r| r.recv().ok());
                        let Some(page) = job else { break };
                        let t = Instant::now();
                        match engine.recognize(&page.image) {
                            Ok(mut res) => {
                                res.blocks = anyocr_core::tbpu::Parser::MultiPara.run(res.blocks);
                                let _ = rtx.send((page.index, res, t.elapsed().as_millis()));
                            }
                            Err(e) => eprintln!("第 {} 页识别失败：{e}", page.index + 1),
                        }
                    }
                })
            })
            .collect();
        drop(rtx);

        let mut render_ms = 0u128;
        let mut t = Instant::now();
        pdf.render_pages(&args.pdf, None, &pages, args.dpi, 1080, |p| {
            render_ms += t.elapsed().as_millis();
            let ok = tx.send(p).is_ok();
            t = Instant::now();
            ok
        })?;
        drop(tx);

        let mut results: Vec<(usize, PageResult, u128)> = rrx.iter().collect();
        for h in handles {
            let _ = h.join();
        }
        let wall = total.elapsed();
        results.sort_by_key(|r| r.0);

        let mut ocr_ms = 0u128;
        let mut chars = 0usize;
        let mut score = 0f32;
        for (i, res, ms) in &results {
            ocr_ms += ms;
            let text = res.text();
            chars += text.chars().filter(|c| !c.is_whitespace()).count();
            score += res.mean_score();
            eprintln!(
                "第 {:>4} 页  {:>5} ms  {:>4} 块  均分 {:.3}  {}x{}",
                i + 1,
                ms,
                res.blocks.len(),
                res.mean_score(),
                res.width,
                res.height
            );
            if let Some(d) = &args.dump {
                std::fs::write(d.join(format!("page-{:04}.txt", i + 1)), text)?;
            }
        }
        let n = results.len().max(1) as f64;
        println!(
            "汇总：{} 页，总耗时 {:.1} s（{:.2} s/页），渲染累计 {:.1} s，OCR 累计 {:.1} s（单页均 {:.0} ms），字符 {}，平均置信度 {:.3}",
            results.len(),
            wall.as_secs_f64(),
            wall.as_secs_f64() / n,
            render_ms as f64 / 1000.0,
            ocr_ms as f64 / 1000.0,
            ocr_ms as f64 / n,
            chars,
            score as f64 / n
        );
        Ok(())
    }
}
