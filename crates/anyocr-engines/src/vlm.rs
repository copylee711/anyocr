//! 视觉大模型（OpenAI Chat Completions 兼容协议）。
//!
//! - [`VlmReviewer`]：模式 B，对照页面图片校对 OCR 行，只返回需要修改的行。
//! - [`VlmEngine`]：模式 A，VLM 作为识别引擎。有检测引擎时坐标来自检测框、文字由 VLM 给出；
//!   没有时只输出文字（框为占位，不能用于文字层）。
//!
//! 实测（2026-11-26）：DeepSeek `deepseek-flash` 默认先思考，一页要 12~40 s；
//! 传 `thinking: {type: disabled}` 后约 1.3 s，校对结果相同，所以预设里默认关闭思考。

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyocr_core::review::{self, Guard, ReviewOutcome, Reviewer};
use anyocr_core::{EngineError, EngineInfo, EngineKind, OcrEngine, PageResult};
use image::RgbImage;
use serde::Serialize;
use serde_json::{Value, json};

use crate::online::{Encoding, agent, b64, check_status, encode, parse_json, read_text, truncate};

/// VLM 服务预设。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VlmPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    /// 默认模型（需支持图片输入）。
    pub model: &'static str,
    /// 可选的其他模型。
    pub models: &'static [&'static str],
    pub key_url: &'static str,
    /// 是否需要 API Key（本地 Ollama 不需要）。
    pub needs_key: bool,
    /// 附加到请求体的参数，如关闭思考。
    pub extra: &'static str,
    pub note: &'static str,
    pub verified: bool,
}

/// 全部 VLM 预设。`custom` 由用户填写地址与模型。
pub fn presets() -> Vec<VlmPreset> {
    vec![
        VlmPreset {
            id: "deepseek",
            name: "DeepSeek",
            base_url: "https://api.deepseek.com",
            model: "deepseek-flash",
            models: &["deepseek-flash"],
            key_url: "https://platform.deepseek.com/api_keys",
            needs_key: true,
            extra: r#"{"thinking":{"type":"disabled"}}"#,
            note: "deepseek-flash 支持图片；已默认关闭思考，单页复核约 1~3 秒。",
            verified: true,
        },
        VlmPreset {
            id: "qwen",
            name: "阿里云百炼（通义千问 VL）",
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            model: "qwen-vl-plus",
            models: &["qwen-vl-plus", "qwen-vl-max", "qwen-vl-ocr"],
            key_url: "https://bailian.console.aliyun.com/?tab=model#/api-key",
            needs_key: true,
            extra: "",
            note: "qwen-vl-ocr 专为文字识别优化。",
            verified: false,
        },
        VlmPreset {
            id: "siliconflow",
            name: "硅基流动",
            base_url: "https://api.siliconflow.cn/v1",
            model: "Qwen/Qwen2.5-VL-72B-Instruct",
            models: &[
                "Qwen/Qwen2.5-VL-72B-Instruct",
                "Qwen/Qwen2.5-VL-32B-Instruct",
            ],
            key_url: "https://cloud.siliconflow.cn/account/ak",
            needs_key: true,
            extra: "",
            note: "开源视觉模型的托管服务。",
            verified: false,
        },
        VlmPreset {
            id: "zhipu",
            name: "智谱 GLM",
            base_url: "https://open.bigmodel.cn/api/paas/v4",
            model: "glm-4v-flash",
            models: &["glm-4v-flash", "glm-4v-plus"],
            key_url: "https://open.bigmodel.cn/usercenter/apikeys",
            needs_key: true,
            extra: "",
            note: "glm-4v-flash 免费。",
            verified: false,
        },
        VlmPreset {
            id: "volcengine",
            name: "火山方舟（豆包）",
            base_url: "https://ark.cn-beijing.volces.com/api/v3",
            model: "",
            models: &[],
            key_url: "https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey",
            needs_key: true,
            extra: r#"{"thinking":{"type":"disabled"}}"#,
            note: "模型处填写控制台里支持视觉的模型名或推理接入点 ID。",
            verified: false,
        },
        VlmPreset {
            id: "openai",
            name: "OpenAI",
            base_url: "https://api.openai.com/v1",
            model: "gpt-4.1-mini",
            models: &["gpt-4.1-mini", "gpt-4.1", "gpt-4o"],
            key_url: "https://platform.openai.com/api-keys",
            needs_key: true,
            extra: "",
            note: "",
            verified: false,
        },
        VlmPreset {
            id: "gemini",
            name: "Google Gemini",
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            model: "gemini-2.5-flash",
            models: &["gemini-2.5-flash", "gemini-2.5-flash-lite"],
            key_url: "https://aistudio.google.com/apikey",
            needs_key: true,
            extra: "",
            note: "使用 Gemini 的 OpenAI 兼容端点。",
            verified: false,
        },
        VlmPreset {
            id: "ollama",
            name: "Ollama（本机）",
            base_url: "http://localhost:11434/v1",
            model: "qwen2.5vl:7b",
            models: &["qwen2.5vl:7b", "qwen2.5vl:3b"],
            key_url: "https://ollama.com/library/qwen2.5vl",
            needs_key: false,
            extra: "",
            note: "本机运行，不上传图片；需要先 `ollama pull` 对应模型。",
            verified: false,
        },
        VlmPreset {
            id: "custom",
            name: "自定义（OpenAI 兼容）",
            base_url: "",
            model: "",
            models: &[],
            key_url: "https://platform.openai.com/docs/api-reference/chat",
            needs_key: false,
            extra: "",
            note: "任何兼容 OpenAI Chat Completions、支持图片输入的服务。",
            verified: false,
        },
    ]
}

/// VLM 客户端配置。
#[derive(Debug, Clone)]
pub struct VlmConfig {
    /// 如 `https://api.deepseek.com`，会拼上 `/chat/completions`。
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    /// 合并进请求体的附加参数（JSON 对象）。
    pub extra: Value,
    pub timeout_secs: u64,
    /// 图片长边上限（像素）。
    pub max_side: u32,
}

impl VlmConfig {
    /// 由预设构造；`model`、`base_url` 为空时用预设默认值。
    pub fn from_preset(
        p: &VlmPreset,
        api_key: Option<String>,
        model: &str,
        base_url: &str,
    ) -> Self {
        let pick = |a: &str, b: &str| {
            if a.trim().is_empty() {
                b.to_owned()
            } else {
                a.trim().to_owned()
            }
        };
        Self {
            base_url: pick(base_url, p.base_url),
            model: pick(model, p.model),
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            extra: serde_json::from_str(p.extra).unwrap_or(Value::Null),
            timeout_secs: 60,
            max_side: 1600,
        }
    }
}

/// OpenAI 兼容的对话客户端（单图 + 文本）。
pub struct VlmClient {
    config: VlmConfig,
    agent: ureq::Agent,
}

impl VlmClient {
    pub fn new(config: VlmConfig) -> Result<Self, EngineError> {
        if config.base_url.trim().is_empty() {
            return Err(EngineError::NotConfigured("缺少服务地址".into()));
        }
        if config.model.trim().is_empty() {
            return Err(EngineError::NotConfigured("缺少模型名".into()));
        }
        let timeout = Duration::from_secs(config.timeout_secs.max(5));
        Ok(Self {
            agent: agent(timeout),
            config,
        })
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// 发送一张图片和提示，要求返回 JSON 对象。
    pub fn ask_json(&self, image: &RgbImage, prompt: &str) -> Result<Value, EngineError> {
        let (data, _) = encode(image, Encoding::Jpeg(85), self.config.max_side)?;
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let mut body = json!({
            "model": self.config.model,
            "temperature": 0,
            "response_format": { "type": "json_object" },
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", b64(&data)) } },
                    { "type": "text", "text": prompt },
                ],
            }],
        });
        if let (Some(obj), Some(extra)) = (body.as_object_mut(), self.config.extra.as_object()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
        let send = |body: &Value| {
            let mut req = self.agent.post(&url);
            if let Some(k) = &self.config.api_key {
                req = req.header("Authorization", format!("Bearer {k}"));
            }
            read_text(req.send_json(body))
        };
        let (mut status, mut text) = send(&body)?;
        // 部分服务不支持 response_format，去掉后重试一次
        if status == 400 && text.contains("response_format") {
            if let Some(obj) = body.as_object_mut() {
                obj.remove("response_format");
            }
            (status, text) = send(&body)?;
        }
        if status == 402 {
            return Err(EngineError::QuotaExceeded(truncate(&text, 300)));
        }
        check_status(status, &text)?;
        let v = parse_json(&text)?;
        let content = v
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                EngineError::Inference(format!("返回中没有内容：{}", truncate(&text, 200)))
            })?;
        extract_json(content)
    }
}

/// 从模型回复中取出 JSON 对象（兼容 ```json 代码块和前后多余文字）。
pub(crate) fn extract_json(content: &str) -> Result<Value, EngineError> {
    let s = content.trim();
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Ok(v);
    }
    let (Some(a), Some(b)) = (s.find('{'), s.rfind('}')) else {
        return Err(EngineError::Inference(format!(
            "回复不是 JSON：{}",
            truncate(s, 200)
        )));
    };
    serde_json::from_str(&s[a..=b])
        .map_err(|e| EngineError::Inference(format!("回复 JSON 无效：{e}；{}", truncate(s, 200))))
}

/// 读取 `{"<key>":[{"id":0,"text":"..."}]}`。
pub(crate) fn id_texts(v: &Value, key: &str) -> Vec<(usize, String)> {
    v[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let id = x["id"]
                .as_u64()
                .or_else(|| x["id"].as_str().and_then(|s| s.trim().parse().ok()))?;
            Some((id as usize, x["text"].as_str()?.to_owned()))
        })
        .collect()
}

fn numbered(page: &PageResult) -> String {
    page.blocks
        .iter()
        .enumerate()
        .map(|(i, b)| format!("{i}: {}", b.text))
        .collect::<Vec<_>>()
        .join("\n")
}

const REVIEW_PROMPT: &str = "下面是这张图片的 OCR 结果，每行格式为“编号: 文本”。请对照图片逐行校对，\
只返回识别有误、需要修改的行；没有错误的行不要返回。\
规则：每行只改错字、漏字、多字和错误的标点符号；不要改空格和排版；不要合并、拆分、新增或删除行；\
不要使用 LaTeX、Markdown 等标记（不要出现 $、\\、_、^、{、}），公式和上下标保持原 OCR 的纯文本写法，只改其中认错的字符；\
表格中每个编号对应固定位置的单元格，不要把相邻单元格的内容挪过来；\
看不清的地方保持原样；只纠正 OCR 认错的字，原书的用字和写法即使你认为不规范也要保留；看不清的上下标不要猜。\
只输出 JSON：{\"fixes\":[{\"id\":编号,\"text\":\"改正后的整行文字\"}]}，没有错误时输出 {\"fixes\":[]}。\n\nOCR 结果：\n";

const TRANSCRIBE_LINES_PROMPT: &str = "下面是检测到的文字行，每行格式为“编号: 初步识别结果”，按图片中从上到下的顺序排列。\
请对照图片给出每一行的准确文字（初步结果可能有错）。不要合并、拆分或新增行。\
只输出 JSON：{\"lines\":[{\"id\":编号,\"text\":\"该行文字\"}]}。\n\n";

const TRANSCRIBE_PROMPT: &str = "请按阅读顺序转写图片中的全部文字，每个视觉上的文本行作为一项，不要遗漏、不要添加。\
数学公式用 LaTeX 表示。只输出 JSON：{\"lines\":[\"第一行\",\"第二行\"]}。";

/// 模式 B：VLM 复核器。
pub struct VlmReviewer {
    client: VlmClient,
    guard: Guard,
}

impl VlmReviewer {
    pub fn new(client: VlmClient, guard: Guard) -> Self {
        Self { client, guard }
    }
}

impl Reviewer for VlmReviewer {
    fn name(&self) -> &str {
        self.client.model()
    }

    fn review(&self, image: &RgbImage, page: &PageResult) -> Result<ReviewOutcome, EngineError> {
        let start = Instant::now();
        if page.blocks.is_empty() {
            return Ok(ReviewOutcome {
                model: self.client.model().to_owned(),
                ..Default::default()
            });
        }
        let v = self
            .client
            .ask_json(image, &format!("{REVIEW_PROMPT}{}", numbered(page)))?;
        let fixes = id_texts(&v, "fixes");
        Ok(ReviewOutcome {
            changes: review::evaluate(page, &fixes, &self.guard),
            elapsed_ms: start.elapsed().as_millis() as u64,
            model: self.client.model().to_owned(),
        })
    }
}

/// 模式 A：VLM 作为识别引擎。
pub struct VlmEngine {
    info: EngineInfo,
    client: VlmClient,
    detector: Option<Arc<dyn OcrEngine>>,
}

impl VlmEngine {
    /// `id` 如 `vlm_deepseek`。`detector` 提供行框（通常是本地引擎）。
    pub fn new(
        id: &str,
        name: &str,
        client: VlmClient,
        detector: Option<Arc<dyn OcrEngine>>,
    ) -> Self {
        Self {
            info: EngineInfo {
                id: id.into(),
                name: name.into(),
                kind: EngineKind::Vlm,
                key_url: None,
                homepage: None,
                has_boxes: detector.is_some(),
            },
            client,
            detector,
        }
    }
}

impl OcrEngine for VlmEngine {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        let mut page = match &self.detector {
            Some(det) => {
                let mut page = det.recognize(image)?;
                if !page.blocks.is_empty() {
                    let v = self.client.ask_json(
                        image,
                        &format!("{TRANSCRIBE_LINES_PROMPT}{}", numbered(&page)),
                    )?;
                    // 转写允许比复核更大的改动，但仍拒绝整行换成无关内容
                    let guard = Guard {
                        max_edit_ratio: 0.8,
                        min_edits_allowed: 3,
                        ..Guard::default()
                    };
                    let outcome = ReviewOutcome {
                        changes: review::evaluate(&page, &id_texts(&v, "lines"), &guard),
                        ..Default::default()
                    };
                    review::apply(&mut page, &outcome);
                }
                page
            }
            None => {
                let v = self.client.ask_json(image, TRANSCRIBE_PROMPT)?;
                let lines: Vec<String> = v["lines"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| l.as_str().map(str::to_owned))
                    .collect();
                PageResult {
                    width: image.width(),
                    height: image.height(),
                    blocks: crate::online::spread_lines(
                        &lines.join("\n"),
                        image.width(),
                        image.height(),
                    ),
                    ..Default::default()
                }
            }
        };
        page.elapsed_ms = start.elapsed().as_millis() as u64;
        page.engine = self.info.id.clone();
        Ok(page)
    }
}

/// 一页中最低的置信度，用于“只复核含低置信度行的页”。
pub fn min_score(page: &PageResult) -> f32 {
    page.blocks.iter().map(|b| b.score).fold(1.0, f32::min)
}

#[cfg(test)]
mod tests {
    use anyocr_core::TextBlock;

    use super::*;

    #[test]
    fn extracts_json_from_fenced_reply() {
        let v =
            extract_json("好的：\n```json\n{\"fixes\":[{\"id\":\"2\",\"text\":\"干涉\"}]}\n```")
                .unwrap();
        assert_eq!(id_texts(&v, "fixes"), vec![(2, "干涉".to_owned())]);
        assert!(extract_json("没有").is_err());
    }

    #[test]
    fn presets_are_consistent() {
        for p in presets() {
            assert!(
                p.extra.is_empty() || serde_json::from_str::<Value>(p.extra).is_ok(),
                "{}",
                p.id
            );
            assert!(p.key_url.starts_with("http"));
        }
        let ds = presets().into_iter().find(|p| p.id == "deepseek").unwrap();
        let cfg = VlmConfig::from_preset(&ds, Some("k".into()), "", "");
        assert_eq!(cfg.model, "deepseek-flash");
        assert_eq!(cfg.extra["thinking"]["type"], "disabled");
    }

    #[test]
    fn missing_model_is_not_configured() {
        let custom = presets().into_iter().find(|p| p.id == "custom").unwrap();
        let cfg = VlmConfig::from_preset(&custom, None, "", "");
        assert!(matches!(
            VlmClient::new(cfg),
            Err(EngineError::NotConfigured(_))
        ));
    }

    /// 需要 `DEEPSEEK_API_KEY`：用样张的 OCR 结果故意改错两处，看能否改回。
    #[test]
    #[ignore]
    fn live_deepseek_review() {
        let Ok(key) = std::env::var("DEEPSEEK_API_KEY") else {
            return;
        };
        let ds = presets().into_iter().find(|p| p.id == "deepseek").unwrap();
        let reviewer = VlmReviewer::new(
            VlmClient::new(VlmConfig::from_preset(&ds, Some(key), "", "")).unwrap(),
            Guard::default(),
        );
        let img = crate::online::test_image();
        let page = PageResult {
            width: img.width(),
            height: img.height(),
            blocks: vec![
                TextBlock::from_rect("AnyOCR online engine test", [30., 25., 560., 75.], 0.99),
                TextBlock::from_rect(
                    "Recognize anyth1ng. Own your dafa.",
                    [30., 90., 620., 130.],
                    0.8,
                ),
                TextBlock::from_rect("识别任何文宇 2026", [30., 150., 360., 200.], 0.7),
            ],
            ..Default::default()
        };
        let out = reviewer.review(&img, &page).unwrap();
        eprintln!("{out:#?}");
        let mut p = page.clone();
        review::apply(&mut p, &out);
        assert_eq!(p.blocks[1].text, "Recognize anything. Own your data.");
        assert_eq!(p.blocks[2].text, "识别任何文字 2026");
    }
}
