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
use std::sync::Arc;

use anyocr_engines::credentials;
use anyocr_engines::online::{self, mineru};
use anyocr_engines::vlm::{self, VlmClient, VlmConfig, VlmEngine};
use anyocr_engines::{ChainEngine, Device, LocalConfig, LocalEngine, LocalModel};
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
    /// 用 MinerU 在线服务把文档转成 Markdown（公式为 LaTeX），需要 ANYOCR_MINERU_TOKEN
    Md(MdArgs),
    /// 列出可用的在线引擎及其密钥环境变量
    Engines,
    /// 交互式配置在线引擎与视觉大模型的凭据（隐藏输入，存入系统凭据管理器，与桌面应用共用）
    Auth {
        #[command(subcommand)]
        cmd: Option<auth::AuthCmd>,
    },
    /// 以 stdio MCP 服务运行，供 Claude Code、Codex 等智能体调用（识别图片、读取与搜索扫描件 PDF）
    Mcp(McpArgs),
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
struct McpArgs {
    #[command(flatten)]
    engine: EngineArgs,
}

#[derive(Args)]
struct MdArgs {
    /// 输入文件（PDF、图片、doc/docx、ppt/pptx）
    input: PathBuf,
    /// 输出目录，结果放在 <输出目录>/<文件名>/ 下；默认与输入同目录
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// 页码范围，如 1-10,15
    #[arg(long)]
    pages: Option<String>,
    /// 模型：vlm（精度高）/ pipeline（速度快）
    #[arg(long, default_value = "vlm")]
    model: String,
}

fn run_md(args: MdArgs) -> Result<()> {
    let preset = online::mineru_preset();
    let mut config = credentials::online_config(&preset);
    config
        .fields
        .insert("model_version".into(), args.model.clone());
    let out = args
        .output
        .clone()
        .or_else(|| args.input.parent().map(PathBuf::from))
        .unwrap_or_default();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let cancel = Arc::clone(&cancel);
        let _ =
            ctrlc::set_handler(move || cancel.store(true, std::sync::atomic::Ordering::Relaxed));
    }
    let t = Instant::now();
    let result = mineru::convert(
        &args.input,
        &out,
        &mineru::Options {
            config,
            page_ranges: args.pages.clone(),
            deadline_secs: 0,
        },
        &cancel,
        |p| match p {
            mineru::Progress::Uploading => eprintln!("上传中…"),
            mineru::Progress::Running {
                state,
                extracted,
                total,
            } => {
                eprintln!(
                    "{state} {extracted}/{total}（{:.0} s）",
                    t.elapsed().as_secs_f64()
                )
            }
            mineru::Progress::Downloading => eprintln!("下载结果…"),
        },
    )?;
    eprintln!("完成，用时 {:.1} s", t.elapsed().as_secs_f64());
    println!("{}", result.markdown.display());
    Ok(())
}

/// 按服务预设构造 VLM 客户端。密钥、模型、地址都按“环境变量 → 凭据管理器（anyocr auth）”取值。
pub(crate) fn vlm_client(provider: &str, model: &str, base_url: &str) -> Result<VlmClient> {
    let preset = vlm::presets()
        .into_iter()
        .find(|p| p.id == provider)
        .with_context(|| format!("未知的 VLM 服务 {provider}，可用：deepseek、qwen、siliconflow、zhipu、volcengine、openai、gemini、ollama、custom"))?;
    let key = credentials::vlm_key(provider);
    if preset.needs_key && key.is_none() {
        anyhow::bail!(
            "缺少 {provider} 的 API Key：请在终端运行 `anyocr auth set vlm_{provider}`，或设置环境变量 ANYOCR_VLM_{}_API_KEY",
            provider.to_uppercase()
        );
    }
    let id = format!("vlm_{provider}");
    let saved = |field: &str, given: &str| {
        if given.trim().is_empty() {
            credentials::lookup(&id, field)
                .map(|(v, _)| v)
                .unwrap_or_default()
        } else {
            given.to_owned()
        }
    };
    let config = VlmConfig::from_preset(
        &preset,
        key,
        &saved("model", model),
        &saved("base_url", base_url),
    );
    Ok(VlmClient::new(config)?)
}

fn run_engines() {
    let mut all = online::presets();
    all.push(online::mineru_preset());
    for p in all {
        println!(
            "{:<16} {}{}",
            p.info.id,
            p.info.name,
            if p.verified { "" } else { "（未实测）" }
        );
        for f in &p.fields {
            println!(
                "    ANYOCR_{}_{:<14} {}{}",
                p.info.id.to_uppercase(),
                f.key.to_uppercase(),
                f.label,
                if f.default.is_empty() {
                    String::new()
                } else {
                    format!("（默认 {}）", f.default)
                }
            );
        }
        if let Some(u) = &p.info.key_url {
            println!("    获取 Key：{u}");
        }
    }
    println!("\n视觉大模型（--review <服务> 复核，或 --engine vlm_<服务> 识别）：");
    for p in vlm::presets() {
        println!(
            "{:<16} {}，默认模型 {}{}",
            p.id,
            p.name,
            if p.model.is_empty() {
                "（需填写）"
            } else {
                p.model
            },
            if p.verified { "" } else { "（未实测）" }
        );
        if p.needs_key {
            println!("    ANYOCR_VLM_{}_API_KEY", p.id.to_uppercase());
        }
        println!("    获取 Key：{}", p.key_url);
    }
}

#[derive(Args, Clone)]
pub(crate) struct EngineArgs {
    /// 引擎，逗号分隔即为 fallback 链，如 `paddle_aistudio,local`。local 为本地引擎；
    /// `vlm_<服务>`（如 vlm_deepseek）为视觉大模型识别（本地检测行框 + VLM 转写）；
    /// 密钥从环境变量读取（见 `anyocr engines`）
    #[arg(long, default_value = "local")]
    pub(crate) engine: String,
    /// `vlm_<服务>` 引擎使用的模型（默认用预设模型）
    #[arg(long, default_value = "")]
    vlm_model: String,
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
    /// 构造 `n` 个可并行使用的引擎。在线引擎在各实例间共享，本地引擎每个实例单独加载。
    pub(crate) fn build_many(&self, n: usize) -> Result<Vec<Arc<dyn OcrEngine>>> {
        let ids: Vec<&str> = self
            .engine
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if ids.is_empty() {
            anyhow::bail!("--engine 不能为空");
        }
        let presets = online::presets();
        let mut shared: Vec<Option<Arc<dyn OcrEngine>>> = Vec::new();
        for id in &ids {
            if *id == "local" || id.starts_with("vlm_") {
                shared.push(None);
                continue;
            }
            let p = presets.iter().find(|p| p.info.id == *id).with_context(|| {
                format!("未知引擎 {id}，可用：local 以及 `anyocr engines` 列出的 id")
            })?;
            let e = online::build(id, credentials::online_config(p))
                .with_context(|| format!("{id} 初始化失败"))?;
            shared.push(Some(e));
        }
        (0..n.max(1))
            .map(|_| {
                let list = shared
                    .iter()
                    .zip(&ids)
                    .map(|(s, id)| match s {
                        Some(e) => Ok(Arc::clone(e)),
                        None => {
                            let local = Arc::new(self.build_local()?) as Arc<dyn OcrEngine>;
                            match id.strip_prefix("vlm_") {
                                // 视觉大模型识别：本地引擎提供行框，VLM 给出文字
                                Some(provider) => {
                                    let client = vlm_client(provider, &self.vlm_model, "")?;
                                    let name = format!("VLM {}", client.model());
                                    Ok(Arc::new(VlmEngine::new(id, &name, client, Some(local)))
                                        as Arc<dyn OcrEngine>)
                                }
                                None => Ok(local),
                            }
                        }
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(if list.len() == 1 {
                    list.into_iter().next().unwrap_or_else(|| unreachable!())
                } else {
                    Arc::new(ChainEngine::new(list)) as Arc<dyn OcrEngine>
                })
            })
            .collect()
    }

    pub(crate) fn uses_local(&self) -> bool {
        self.engine
            .split(',')
            .any(|s| s.trim() == "local" || s.trim().starts_with("vlm_"))
    }

    fn build_local(&self) -> Result<LocalEngine> {
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
    let engine = args.engine.build_many(1)?.remove(0);
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
        Command::Md(a) => run_md(a),
        Command::Mcp(a) => mcp::run(a.engine),
        Command::Auth { cmd } => auth::run(cmd),
        Command::Engines => {
            run_engines();
            Ok(())
        }
        Command::Pdf(a) => pdf::run(a),
    }
}

mod auth;
mod index;
mod mcp;
mod pdf;

mod bench {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use anyhow::{Context, Result};
    use anyocr_core::PageResult;
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
        let engines = args.engine.build_many(workers)?;
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
