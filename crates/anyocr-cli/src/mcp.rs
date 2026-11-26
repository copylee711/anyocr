//! `anyocr mcp`：stdio MCP 服务，供 Claude Code、Codex 等智能体识别图片和扫描件 PDF。
//!
//! 工具：
//! - `ocr_image`：识别图片。
//! - `ocr_pdf`：按页读取 PDF 文字。有文字层的页直接提取，扫描页现场 OCR（结果缓存）。
//! - `search_pdf`：关键词 → 页码与上下文，用来在整本书里定位，再用 `ocr_pdf` 读相关页。
//! - `make_searchable_pdf`：后台生成带文字层的 PDF，同时把全书文字写入缓存；用 `job_status` 查进度。
//!
//! 大结果按字数截断并提示下一次从哪页继续，避免撑爆智能体上下文。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use anyocr_core::OcrEngine;
use anyocr_core::tbpu::Parser as Tbpu;
use anyocr_pdf::pipeline::{LayerMode, Progress, SearchableOptions, make_searchable};
use anyocr_pdf::{PdfTool, parse_page_ranges};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::EngineArgs;
use crate::index::{PageCache, search};

/// 页面已有不少于该数量的非空白字符时视为有文字层。
const TEXT_THRESHOLD: usize = 10;
/// 单次 `ocr_pdf` 最多处理的页数（扫描页每页约 1 秒）。
const MAX_PAGES_PER_CALL: usize = 30;

const INSTRUCTIONS: &str = "AnyOCR：本地 OCR，用于读取图片和扫描件 PDF（没有文字层、无法直接复制文字的 PDF）。\
推荐流程：长文档先用 search_pdf 按关键词定位页码，再用 ocr_pdf 只读取相关页；\
若 search_pdf 提示有很多扫描页未建立索引，调用 make_searchable_pdf 在后台识别全书（约 1 秒/页），用 job_status 查看进度，完成后即可全文搜索。\
所有识别都在本机完成，不上传文件。";

enum JobState {
    Running {
        done: usize,
        total: usize,
        started: Instant,
    },
    Done {
        summary: String,
    },
    Failed {
        error: String,
    },
}

struct State {
    engine_args: EngineArgs,
    engine: Mutex<Option<Arc<dyn OcrEngine>>>,
    pdf: Mutex<Option<Arc<PdfTool>>>,
    jobs: Mutex<HashMap<u64, JobState>>,
    next_job: AtomicU64,
}

impl State {
    fn engine(&self) -> Result<Arc<dyn OcrEngine>, String> {
        let mut g = self
            .engine
            .lock()
            .map_err(|_| "引擎状态已损坏".to_string())?;
        if let Some(e) = g.as_ref() {
            return Ok(Arc::clone(e));
        }
        let e = self
            .engine_args
            .build_many(1)
            .map_err(|e| format!("{e:#}"))?
            .remove(0);
        *g = Some(Arc::clone(&e));
        Ok(e)
    }

    fn pdf(&self) -> Result<Arc<PdfTool>, String> {
        let mut g = self.pdf.lock().map_err(|_| "PDF 状态已损坏".to_string())?;
        if let Some(p) = g.as_ref() {
            return Ok(Arc::clone(p));
        }
        let p = Arc::new(PdfTool::new().map_err(|e| e.to_string())?);
        *g = Some(Arc::clone(&p));
        Ok(p)
    }

    fn set_job(&self, id: u64, s: JobState) {
        if let Ok(mut g) = self.jobs.lock() {
            g.insert(id, s);
        }
    }
}

#[derive(Clone)]
pub struct AnyOcrMcp {
    state: Arc<State>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OcrImageArgs {
    /// 图片的绝对路径（png、jpg、bmp、webp、tif 等）
    pub path: String,
    /// 排版解析：multi_para（默认，多栏按段落）、multi_line、single_para、single_line、single_code（代码）、none
    #[serde(default)]
    pub parser: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OcrPdfArgs {
    /// PDF 的绝对路径
    pub path: String,
    /// 页码范围（1 起始），如 "1-5" 或 "12,15-18"。默认 "1-10"，单次最多 30 页
    #[serde(default)]
    pub pages: Option<String>,
    /// 返回的最大字符数，默认 20000；超出时在页边界截断并提示下一页
    #[serde(default)]
    pub max_chars: Option<usize>,
    /// 文档密码
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchPdfArgs {
    /// PDF 的绝对路径
    pub path: String,
    /// 关键词（忽略空格、换行与大小写）
    pub query: String,
    /// 最多返回的命中数，默认 20
    #[serde(default)]
    pub max_results: Option<usize>,
    /// 每处命中前后显示的字符数，默认 60
    #[serde(default)]
    pub context: Option<usize>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MakeSearchableArgs {
    /// 扫描件 PDF 的绝对路径
    pub path: String,
    /// 输出路径，默认在原文件旁生成 <文件名>.ocr.pdf
    #[serde(default)]
    pub output: Option<String>,
    /// 页码范围（1 起始），默认全部
    #[serde(default)]
    pub pages: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct JobStatusArgs {
    /// make_searchable_pdf 返回的任务编号
    pub job_id: u64,
}

fn ok(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        text.into(),
    )]))
}

fn fail(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(text.into())]))
}

/// 在阻塞线程中执行，结果（含错误）以文本返回给智能体。
async fn blocking(
    f: impl FnOnce() -> Result<String, String> + Send + 'static,
) -> Result<CallToolResult, ErrorData> {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(s)) => ok(s),
        Ok(Err(e)) => fail(e),
        Err(e) => fail(format!("内部错误：{e}")),
    }
}

fn existing(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.is_file() {
        return Err(format!("文件不存在：{path}（请使用绝对路径）"));
    }
    Ok(p)
}

#[tool_router]
impl AnyOcrMcp {
    pub fn new(engine_args: EngineArgs) -> Self {
        Self {
            state: Arc::new(State {
                engine_args,
                engine: Mutex::new(None),
                pdf: Mutex::new(None),
                jobs: Mutex::new(HashMap::new()),
                next_job: AtomicU64::new(1),
            }),
        }
    }

    #[tool(
        description = "识别图片中的文字（本地 OCR）。返回按阅读顺序排列的纯文本。首次调用会加载模型，可能需要几秒。"
    )]
    async fn ocr_image(
        &self,
        Parameters(args): Parameters<OcrImageArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let st = Arc::clone(&self.state);
        blocking(move || {
            let path = existing(&args.path)?;
            let parser: Tbpu = args
                .parser
                .as_deref()
                .unwrap_or("multi_para")
                .parse()
                .map_err(|e: String| e)?;
            let img = image::open(&path)
                .map_err(|e| format!("无法打开图片：{e}"))?
                .to_rgb8();
            let mut page = st.engine()?.recognize(&img).map_err(|e| e.to_string())?;
            page.blocks = parser.run(page.blocks);
            let text = page.text();
            Ok(if text.trim().is_empty() {
                "（没有识别到文字）".to_owned()
            } else {
                text
            })
        })
        .await
    }

    #[tool(
        description = "按页读取 PDF 的文字。有文字层的页直接提取，扫描页现场 OCR（约 1 秒/页，结果会缓存）。每页以 <!-- page N --> 开头。单次最多 30 页；长文档请先用 search_pdf 定位页码。"
    )]
    async fn ocr_pdf(
        &self,
        Parameters(args): Parameters<OcrPdfArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let st = Arc::clone(&self.state);
        blocking(move || read_pages(&st, &args)).await
    }

    #[tool(
        description = "在 PDF 中搜索关键词，返回页码与上下文。会搜索文字层和已识别过的扫描页；结果会说明有多少扫描页尚未识别，必要时先调用 make_searchable_pdf。"
    )]
    async fn search_pdf(
        &self,
        Parameters(args): Parameters<SearchPdfArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let st = Arc::clone(&self.state);
        blocking(move || {
            let path = existing(&args.path)?;
            let pdf = st.pdf()?;
            let texts = pdf
                .pages_text(&path, args.password.as_deref())
                .map_err(|e| e.to_string())?;
            let cache = PageCache::load(&path);
            let mut missing = Vec::new();
            let pages: Vec<(usize, &str)> = texts
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    if t.chars().filter(|c| !c.is_whitespace()).count() >= TEXT_THRESHOLD {
                        Some((i, t.as_str()))
                    } else if let Some(c) = cache.pages.get(&i) {
                        Some((i, c.as_str()))
                    } else {
                        missing.push(i + 1);
                        None
                    }
                })
                .collect();
            let limit = args.max_results.unwrap_or(20).clamp(1, 200);
            let (hits, total) = search(pages, &args.query, args.context.unwrap_or(60).min(400), limit);
            let mut out = format!(
                "“{}”共命中 {total} 处（文档共 {} 页，已搜索 {} 页）。\n",
                args.query,
                texts.len(),
                texts.len() - missing.len()
            );
            for h in &hits {
                out.push_str(&format!("- 第 {} 页：{}\n", h.page, h.context));
            }
            if total > hits.len() {
                out.push_str(&format!("（只列出前 {} 处，可增大 max_results）\n", hits.len()));
            }
            if !missing.is_empty() {
                out.push_str(&format!(
                    "\n注意：{} 页是扫描页且尚未识别，没有参与搜索（{}）。可调用 make_searchable_pdf 在后台识别全书（约 1 秒/页），或用 ocr_pdf 读取指定页。\n",
                    missing.len(),
                    summarize_pages(&missing)
                ));
            }
            Ok(out)
        })
        .await
    }

    #[tool(
        description = "在后台为扫描件 PDF 添加可搜索的文字层（原图不变，另存为新文件），并把全书文字写入缓存，之后 search_pdf / ocr_pdf 无需再识别。立即返回任务编号，用 job_status 查询进度。"
    )]
    async fn make_searchable_pdf(
        &self,
        Parameters(args): Parameters<MakeSearchableArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let path = match existing(&args.path) {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        let output = args
            .output
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| default_output(&path));
        if output == path {
            return fail("输出路径不能与输入相同");
        }
        let st = Arc::clone(&self.state);
        let id = st.next_job.fetch_add(1, Ordering::Relaxed);
        st.set_job(
            id,
            JobState::Running {
                done: 0,
                total: 0,
                started: Instant::now(),
            },
        );
        let out_display = output.display().to_string();
        std::thread::spawn(move || {
            let r = run_searchable(&st, id, &path, &output, &args);
            st.set_job(
                id,
                match r {
                    Ok(summary) => JobState::Done { summary },
                    Err(error) => JobState::Failed { error },
                },
            );
        });
        ok(format!(
            "已开始，任务编号 {id}。输出：{out_display}\n识别速度约 1 秒/页，请稍后用 job_status 查询进度。完成后可直接对原文件调用 search_pdf / ocr_pdf。"
        ))
    }

    #[tool(description = "查询 make_searchable_pdf 后台任务的进度或结果。")]
    async fn job_status(
        &self,
        Parameters(args): Parameters<JobStatusArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let g = match self.state.jobs.lock() {
            Ok(g) => g,
            Err(_) => return fail("任务状态已损坏"),
        };
        match g.get(&args.job_id) {
            None => fail(format!("没有编号为 {} 的任务", args.job_id)),
            Some(JobState::Running {
                done,
                total,
                started,
            }) => {
                let secs = started.elapsed().as_secs_f64();
                let eta = if *done > 0 && *total > *done {
                    format!(
                        "，预计还需 {:.0} 秒",
                        secs / *done as f64 * (*total - *done) as f64
                    )
                } else {
                    String::new()
                };
                ok(format!(
                    "进行中：已识别 {done}/{total} 页，已用 {secs:.0} 秒{eta}。"
                ))
            }
            Some(JobState::Done { summary }) => ok(format!("已完成。{summary}")),
            Some(JobState::Failed { error }) => fail(format!("失败：{error}")),
        }
    }
}

#[tool_handler]
impl ServerHandler for AnyOcrMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("anyocr", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

fn default_output(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into());
    input.with_file_name(format!("{stem}.ocr.pdf"))
}

/// 把页码列表压缩成“1-5, 8, 10-12”，过长时省略。
fn summarize_pages(pages: &[usize]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let mut j = i;
        while j + 1 < pages.len() && pages[j + 1] == pages[j] + 1 {
            j += 1;
        }
        parts.push(if i == j {
            format!("{}", pages[i])
        } else {
            format!("{}-{}", pages[i], pages[j])
        });
        i = j + 1;
    }
    if parts.len() > 8 {
        format!("第 {} 等", parts[..8].join(", "))
    } else {
        format!("第 {} 页", parts.join(", "))
    }
}

fn read_pages(st: &State, args: &OcrPdfArgs) -> Result<String, String> {
    let path = existing(&args.path)?;
    let pw = args.password.as_deref();
    let pdf = st.pdf()?;
    let texts = pdf.pages_text(&path, pw).map_err(|e| e.to_string())?;
    let count = texts.len();
    let spec = args.pages.clone().unwrap_or_else(|| "1-10".into());
    let mut selected = parse_page_ranges(&spec, count)?;
    if selected.is_empty() {
        return Err(format!("页码范围为空（文档共 {count} 页）"));
    }
    let mut note = String::new();
    if selected.len() > MAX_PAGES_PER_CALL {
        let next = selected[MAX_PAGES_PER_CALL] + 1;
        selected.truncate(MAX_PAGES_PER_CALL);
        note = format!("\n（单次最多 {MAX_PAGES_PER_CALL} 页，后续请从第 {next} 页继续）");
    }

    // 需要 OCR 的扫描页：先查缓存
    let mut cache = PageCache::load(&path);
    let need: Vec<usize> = selected
        .iter()
        .copied()
        .filter(|i| {
            texts[*i].chars().filter(|c| !c.is_whitespace()).count() < TEXT_THRESHOLD
                && !cache.pages.contains_key(i)
        })
        .collect();
    if !need.is_empty() {
        let engine = st.engine()?;
        let mut err = None;
        pdf.render_pages(&path, pw, &need, 200.0, 1080, |p| {
            match engine.recognize(&p.image) {
                Ok(mut r) => {
                    r.blocks = Tbpu::MultiPara.run(r.blocks);
                    cache.pages.insert(p.index, r.text());
                }
                Err(e) => {
                    err.get_or_insert(format!("第 {} 页识别失败：{e}", p.index + 1));
                }
            }
            true
        })
        .map_err(|e| e.to_string())?;
        cache.save();
        if let Some(e) = err {
            return Err(e);
        }
    }

    let max = args.max_chars.unwrap_or(20_000).clamp(1000, 200_000);
    let mut out = String::new();
    for (k, &i) in selected.iter().enumerate() {
        let (text, src) =
            if texts[i].chars().filter(|c| !c.is_whitespace()).count() >= TEXT_THRESHOLD {
                (texts[i].as_str(), "文字层")
            } else {
                (cache.pages.get(&i).map(String::as_str).unwrap_or(""), "OCR")
            };
        let block = format!("<!-- page {} ({src}) -->\n{}\n\n", i + 1, text.trim());
        if k > 0 && out.chars().count() + block.chars().count() > max {
            note = format!(
                "\n（已达 max_chars={max} 上限，后续请从第 {} 页继续）",
                i + 1
            );
            break;
        }
        out.push_str(&block);
    }
    out.push_str(&format!("（文档共 {count} 页）{note}"));
    Ok(out)
}

fn run_searchable(
    st: &State,
    id: u64,
    path: &Path,
    output: &Path,
    args: &MakeSearchableArgs,
) -> Result<String, String> {
    let pdf = st.pdf()?;
    let engine = st.engine()?;
    let opts = SearchableOptions {
        mode: LayerMode::SkipText,
        pages: args.pages.clone().unwrap_or_default(),
        password: args.password.clone(),
        ..Default::default()
    };
    let mut cache = PageCache::load(path);
    let started = Instant::now();
    let mut done = 0;
    let mut total = 0;
    let report = make_searchable(
        &pdf,
        &[engine],
        path,
        output,
        &opts,
        &AtomicBool::new(false),
        |p| match p {
            Progress::Planned { to_ocr, .. } => {
                total = to_ocr;
                st.set_job(
                    id,
                    JobState::Running {
                        done,
                        total,
                        started,
                    },
                );
            }
            Progress::Page(_) => {
                done += 1;
                st.set_job(
                    id,
                    JobState::Running {
                        done: done.min(total),
                        total,
                        started,
                    },
                );
            }
            Progress::Recognized { index, result } => {
                cache.pages.insert(index, result.text());
            }
            _ => {}
        },
    )
    .map_err(|e| e.to_string())?;
    cache.save();
    // 输出文件也记一份：空白页的文字层为空，否则搜索输出文件时会被当成“尚未识别”
    let mut out_cache = PageCache::load(output);
    out_cache.pages = cache.pages.clone();
    out_cache.save();
    Ok(format!(
        "识别 {} 页，跳过 {} 页（已有文字），失败 {} 页，用时 {:.0} 秒。输出：{}",
        report.done(),
        report.skipped(),
        report.failed(),
        report.elapsed_ms as f64 / 1000.0,
        output.display()
    ))
}

/// 运行 stdio MCP 服务，直到客户端断开。
pub fn run(engine_args: EngineArgs) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let service = AnyOcrMcp::new(engine_args)
            .serve(rmcp::transport::stdio())
            .await?;
        service.waiting().await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_summary() {
        assert_eq!(summarize_pages(&[1, 2, 3, 5, 7, 8]), "第 1-3, 5, 7-8 页");
        let many: Vec<usize> = (1..=40).step_by(2).collect();
        assert!(summarize_pages(&many).ends_with("等"));
    }
}
