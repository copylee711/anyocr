//! 在线 OCR 引擎。
//!
//! 每个服务是一个预设（[`Preset`]）：元数据、需要用户填写的字段、构造函数。设置页按字段渲染表单，
//! 密钥由调用方（桌面端用系统凭据管理器，CLI 用环境变量）放进 [`OnlineConfig`]。
//!
//! 所有在线引擎都把 HTTP 错误归类成 [`EngineError`]，以便 fallback 链判断是否切换到下一个引擎。

mod azure;
mod baidu;
mod google;
mod mistral;
mod ocrspace;
mod paddle;
mod tencent;
mod youdao;

pub mod mineru;

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use anyocr_core::{EngineError, EngineInfo, EngineKind, OcrEngine};
use base64::Engine as _;
use image::RgbImage;
use serde::Serialize;

/// 设置页上的一个输入项。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// 字段键，如 `api_key`、`secret_key`、`endpoint`。
    pub key: &'static str,
    /// 显示名称。
    pub label: &'static str,
    /// 是否是密钥（存进凭据管理器，界面上用密码框）。
    pub secret: bool,
    pub required: bool,
    /// 默认值（非密钥字段）。
    #[serde(skip_serializing_if = "str::is_empty")]
    pub default: &'static str,
    /// 可选值（下拉框）；为空时是自由输入。
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub options: &'static [&'static str],
    /// 输入提示。
    #[serde(skip_serializing_if = "str::is_empty")]
    pub hint: &'static str,
}

impl Field {
    const fn secret(key: &'static str, label: &'static str) -> Self {
        Self {
            key,
            label,
            secret: true,
            required: true,
            default: "",
            options: &[],
            hint: "",
        }
    }
    const fn text(key: &'static str, label: &'static str, default: &'static str) -> Self {
        Self {
            key,
            label,
            secret: false,
            required: false,
            default,
            options: &[],
            hint: "",
        }
    }
    const fn options(mut self, options: &'static [&'static str]) -> Self {
        self.options = options;
        self
    }
    const fn required(mut self) -> Self {
        self.required = true;
        self
    }
    const fn hint(mut self, hint: &'static str) -> Self {
        self.hint = hint;
        self
    }
}

/// 一个在线引擎预设。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub info: EngineInfo,
    pub fields: Vec<Field>,
    /// 说明（计费、限制、是否适合截图等）。
    pub note: &'static str,
    /// 是否已用真实密钥验证过（界面上标注“未实测”）。
    pub verified: bool,
}

/// 构造在线引擎所需的配置：字段值（含密钥）与超时。
#[derive(Debug, Clone, Default)]
pub struct OnlineConfig {
    pub fields: BTreeMap<String, String>,
    /// 单次识别的总超时；为 0 时用 60 秒。
    pub timeout_secs: u64,
}

impl OnlineConfig {
    /// 取字段值；未填写时用预设默认值。
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .get(key)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
    }

    fn require(&self, key: &str, label: &str) -> Result<String, EngineError> {
        self.get(key)
            .map(str::to_owned)
            .ok_or_else(|| EngineError::NotConfigured(format!("缺少 {label}")))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(if self.timeout_secs == 0 {
            60
        } else {
            self.timeout_secs
        })
    }
}

fn info(id: &str, name: &str, key_url: &str, homepage: &str, has_boxes: bool) -> EngineInfo {
    EngineInfo {
        id: id.into(),
        name: name.into(),
        kind: EngineKind::Online,
        key_url: Some(key_url.into()),
        homepage: Some(homepage.into()),
        has_boxes,
    }
}

/// 全部在线引擎预设，顺序即设置页的默认顺序。
pub fn presets() -> Vec<Preset> {
    vec![
        paddle::preset(),
        baidu::preset(),
        tencent::preset(),
        youdao::preset(),
        google::preset(),
        azure::preset(),
        ocrspace::preset(),
        mistral::preset(),
    ]
}

/// MinerU 文档解析（PDF → Markdown）的预设。它不是逐页识别引擎，单独列出。
pub fn mineru_preset() -> Preset {
    mineru::preset()
}

/// 按预设 id 构造在线引擎。
pub fn build(id: &str, config: OnlineConfig) -> Result<Arc<dyn OcrEngine>, EngineError> {
    Ok(match id {
        "paddle_aistudio" => Arc::new(paddle::Paddle::new(config)?),
        "baidu" => Arc::new(baidu::Baidu::new(config)?),
        "tencent" => Arc::new(tencent::Tencent::new(config)?),
        "youdao" => Arc::new(youdao::Youdao::new(config)?),
        "google_vision" => Arc::new(google::Google::new(config)?),
        "azure_vision" => Arc::new(azure::Azure::new(config)?),
        "ocr_space" => Arc::new(ocrspace::OcrSpace::new(config)?),
        "mistral" => Arc::new(mistral::Mistral::new(config)?),
        _ => return Err(EngineError::NotConfigured(format!("未知的在线引擎：{id}"))),
    })
}

/// 读取 CLI / 测试用的环境变量：`ANYOCR_<ID>_<FIELD>`，例如 `ANYOCR_BAIDU_API_KEY`。
/// PaddleOCR、MinerU 兼容旧的 `ANYOCR_PADDLE_TOKEN`、`ANYOCR_MINERU_TOKEN`。
pub fn config_from_env(preset: &Preset) -> OnlineConfig {
    let mut fields = BTreeMap::new();
    for f in &preset.fields {
        let name = format!(
            "ANYOCR_{}_{}",
            preset.info.id.to_uppercase(),
            f.key.to_uppercase()
        );
        let legacy = match (preset.info.id.as_str(), f.key) {
            ("paddle_aistudio", "token") => Some("ANYOCR_PADDLE_TOKEN"),
            ("mineru", "token") => Some("ANYOCR_MINERU_TOKEN"),
            _ => None,
        };
        let v = std::env::var(&name)
            .ok()
            .or_else(|| legacy.and_then(|l| std::env::var(l).ok()));
        if let Some(v) = v {
            fields.insert(f.key.to_owned(), v);
        }
    }
    OnlineConfig {
        fields,
        timeout_secs: 0,
    }
}

// ---------------------------------------------------------------------------
// HTTP 与编码工具
// ---------------------------------------------------------------------------

pub(crate) fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into()
}

/// 把传输层错误归类。
pub(crate) fn transport(e: ureq::Error) -> EngineError {
    match e {
        ureq::Error::Timeout(_) => EngineError::Timeout(e.to_string()),
        other => EngineError::Network(other.to_string()),
    }
}

/// 按 HTTP 状态码归类错误；2xx 返回 `Ok(())`。
pub(crate) fn check_status(status: u16, body: &str) -> Result<(), EngineError> {
    let msg = || format!("HTTP {status}：{}", truncate(body, 300));
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(EngineError::Unauthorized(msg())),
        402 => Err(EngineError::QuotaExceeded(msg())),
        429 => Err(EngineError::RateLimited(msg())),
        408 | 504 => Err(EngineError::Timeout(msg())),
        400 | 413 | 415 | 422 => Err(EngineError::InvalidInput(msg())),
        _ => Err(EngineError::Network(msg())),
    }
}

/// 读取响应体为字符串并检查状态码。
pub(crate) fn read_text(
    resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<(u16, String), EngineError> {
    let mut resp = resp.map_err(transport)?;
    let status = resp.status().as_u16();
    let body = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .map_err(transport)?;
    Ok((status, body))
}

/// 读取 JSON 响应；非 2xx 时按状态码归类。
pub(crate) fn read_json(
    resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<serde_json::Value, EngineError> {
    let (status, body) = read_text(resp)?;
    check_status(status, &body)?;
    parse_json(&body)
}

pub(crate) fn parse_json(body: &str) -> Result<serde_json::Value, EngineError> {
    serde_json::from_str(body).map_err(|e| {
        EngineError::Inference(format!("无法解析返回的 JSON：{e}；{}", truncate(body, 200)))
    })
}

pub(crate) fn truncate(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// 图片编码格式。
#[derive(Clone, Copy)]
pub(crate) enum Encoding {
    /// JPEG，质量 0~100。用于有文件大小限制的服务。
    Jpeg(u8),
}

impl Encoding {
    pub(crate) fn mime(self) -> &'static str {
        match self {
            Encoding::Jpeg(_) => "image/jpeg",
        }
    }
}

/// 编码图片；`max_side` 大于 0 时先等比缩小到长边不超过该值，返回编码结果与缩放比例（原图 / 编码图）。
pub(crate) fn encode(
    image: &RgbImage,
    enc: Encoding,
    max_side: u32,
) -> Result<(Vec<u8>, f32), EngineError> {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return Err(EngineError::InvalidInput("图片尺寸为 0".into()));
    }
    let long = w.max(h);
    let (img, scale) = if max_side > 0 && long > max_side {
        let s = max_side as f32 / long as f32;
        let nw = ((w as f32 * s).round() as u32).max(1);
        let nh = ((h as f32 * s).round() as u32).max(1);
        (
            std::borrow::Cow::Owned(image::imageops::resize(
                image,
                nw,
                nh,
                image::imageops::FilterType::Triangle,
            )),
            w as f32 / nw as f32,
        )
    } else {
        (std::borrow::Cow::Borrowed(image), 1.0)
    };
    let mut buf = Cursor::new(Vec::new());
    let r = match enc {
        Encoding::Jpeg(q) => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q).encode_image(&*img)
        }
    };
    r.map_err(|e| EngineError::InvalidInput(format!("图片编码失败：{e}")))?;
    Ok((buf.into_inner(), scale))
}

pub(crate) fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// 简单的 multipart/form-data 构造器。
pub(crate) struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    pub(crate) fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        Self {
            boundary: format!("----anyocr{nanos:x}"),
            body: Vec::new(),
        }
    }

    pub(crate) fn text(mut self, name: &str, value: &str) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self
    }

    pub(crate) fn file(mut self, name: &str, filename: &str, mime: &str, data: &[u8]) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self.body.extend_from_slice(data);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    /// 返回 (Content-Type, body)。
    pub(crate) fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}

/// 把 JSON 数值数组 `[[x,y],...]` 读成四点框。点数不是 4 时取外接矩形。
pub(crate) fn quad_from_points(points: &[[f32; 2]], scale: f32) -> Option<anyocr_core::Quad> {
    if points.is_empty() {
        return None;
    }
    let s = |p: [f32; 2]| [p[0] * scale, p[1] * scale];
    if points.len() == 4 {
        return Some([s(points[0]), s(points[1]), s(points[2]), s(points[3])]);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in points {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    Some([s([x0, y0]), s([x1, y0]), s([x1, y1]), s([x0, y1])])
}

/// 从 JSON 读 `f32`，兼容整数与浮点。
pub(crate) fn num(v: &serde_json::Value) -> f32 {
    v.as_f64().unwrap_or_default() as f32
}

/// 把整页文字按行切开，框沿页面高度均匀排布（没有坐标的引擎用，框只是占位）。
pub(crate) fn spread_lines(text: &str, width: u32, height: u32) -> Vec<anyocr_core::TextBlock> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let n = lines.len().max(1) as f32;
    let (w, h) = (width as f32, height as f32);
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let y0 = h * i as f32 / n;
            anyocr_core::TextBlock::from_rect(*l, [0.0, y0, w, y0 + h / n], 1.0)
        })
        .collect()
}

/// 统计耗时并补齐 PageResult 的通用字段。
pub(crate) fn finish_page(
    info: &EngineInfo,
    image: &RgbImage,
    blocks: Vec<anyocr_core::TextBlock>,
    start: std::time::Instant,
) -> anyocr_core::PageResult {
    anyocr_core::PageResult {
        width: image.width(),
        height: image.height(),
        blocks,
        elapsed_ms: start.elapsed().as_millis() as u64,
        engine: info.id.clone(),
    }
}

/// 仓库内的公开样张（英文 + 中文），供联网测试使用。
#[cfg(test)]
pub(crate) fn test_image() -> RgbImage {
    image::load_from_memory(include_bytes!("../../../../tests/fixtures/hello.png"))
        .map(|i| i.to_rgb8())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_key_url_and_unique_id() {
        let mut all = presets();
        all.push(mineru_preset());
        let mut ids: Vec<_> = all.iter().map(|p| p.info.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        for p in &all {
            assert!(
                p.info
                    .key_url
                    .as_deref()
                    .is_some_and(|u| u.starts_with("https://"))
            );
            assert!(
                p.fields.iter().any(|f| f.secret),
                "{} 没有密钥字段",
                p.info.id
            );
        }
    }

    #[test]
    fn missing_key_is_not_configured() {
        for p in presets() {
            let r = build(&p.info.id, OnlineConfig::default());
            assert!(
                matches!(r, Err(EngineError::NotConfigured(_))),
                "{} 缺少密钥时应返回 NotConfigured",
                p.info.id
            );
        }
    }

    #[test]
    fn status_classification() {
        assert!(matches!(
            check_status(401, ""),
            Err(EngineError::Unauthorized(_))
        ));
        assert!(matches!(
            check_status(429, ""),
            Err(EngineError::RateLimited(_))
        ));
        assert!(matches!(
            check_status(500, ""),
            Err(EngineError::Network(_))
        ));
        assert!(check_status(200, "").is_ok());
    }

    #[test]
    fn multipart_layout() {
        let (ct, body) = Multipart::new()
            .text("a", "1")
            .file("f", "x.png", "image/png", b"PNG")
            .finish();
        let boundary = ct.split("boundary=").nth(1).unwrap();
        let s = String::from_utf8(body).unwrap();
        assert!(s.starts_with(&format!("--{boundary}\r\n")));
        assert!(s.ends_with(&format!("--{boundary}--\r\n")));
        assert!(
            s.contains("name=\"f\"; filename=\"x.png\"\r\nContent-Type: image/png\r\n\r\nPNG\r\n")
        );
    }

    #[test]
    fn encode_downscales() {
        let img = RgbImage::new(4000, 1000);
        let (data, scale) = encode(&img, Encoding::Jpeg(80), 2000).unwrap();
        assert!(!data.is_empty());
        assert!((scale - 2.0).abs() < 1e-3);
    }
}
