//! 百度智能云文字识别：通用文字识别（含位置信息版 / 高精度含位置版）。
//!
//! 鉴权：用 API Key + Secret Key 换取 access_token（有效期 30 天，进程内缓存）。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyocr_core::{EngineError, EngineInfo, OcrEngine, PageResult, TextBlock};
use image::RgbImage;
use serde_json::Value;

use super::*;

pub fn preset() -> Preset {
    Preset {
        info: info(
            "baidu",
            "百度智能云 OCR",
            "https://console.bce.baidu.com/ai/#/ai/ocr/overview/index",
            "https://cloud.baidu.com/product/ocr",
            true,
        ),
        fields: vec![
            Field::secret("api_key", "API Key"),
            Field::secret("secret_key", "Secret Key"),
            Field::text("model", "接口", "accurate")
                .options(&["accurate", "general"])
                .hint("accurate：高精度含位置版；general：标准含位置版"),
        ],
        note: "需在控制台开通“通用文字识别（含位置）”，有每月免费额度。",
        verified: false,
    }
}

pub struct Baidu {
    info: EngineInfo,
    api_key: String,
    secret_key: String,
    model: String,
    agent: ureq::Agent,
    token: Mutex<Option<(String, Instant)>>,
}

impl Baidu {
    pub fn new(config: OnlineConfig) -> Result<Self, EngineError> {
        let model = match config.get("model") {
            Some("general") => "general",
            _ => "accurate",
        };
        Ok(Self {
            info: preset().info,
            api_key: config.require("api_key", "API Key")?,
            secret_key: config.require("secret_key", "Secret Key")?,
            model: model.into(),
            agent: agent(config.timeout()),
            token: Mutex::new(None),
        })
    }

    fn access_token(&self) -> Result<String, EngineError> {
        if let Ok(g) = self.token.lock()
            && let Some((t, exp)) = g.as_ref()
            && Instant::now() < *exp
        {
            return Ok(t.clone());
        }
        let v = read_json(
            self.agent
                .post("https://aip.baidubce.com/oauth/2.0/token")
                .query("grant_type", "client_credentials")
                .query("client_id", &self.api_key)
                .query("client_secret", &self.secret_key)
                .send_empty(),
        )?;
        let Some(token) = v["access_token"].as_str() else {
            return Err(EngineError::Unauthorized(format!(
                "获取 access_token 失败：{}",
                v["error_description"].as_str().unwrap_or("未知错误")
            )));
        };
        let ttl = v["expires_in"]
            .as_u64()
            .unwrap_or(86400)
            .saturating_sub(600);
        if let Ok(mut g) = self.token.lock() {
            *g = Some((token.to_owned(), Instant::now() + Duration::from_secs(ttl)));
        }
        Ok(token.to_owned())
    }
}

fn api_error(v: &Value) -> Result<(), EngineError> {
    let Some(code) = v.get("error_code").and_then(Value::as_i64) else {
        return Ok(());
    };
    let msg = format!("{code} {}", v["error_msg"].as_str().unwrap_or_default());
    Err(match code {
        // 17 每日请求量超限，19 请求总量超限，18 QPS 超限
        17 | 19 => EngineError::QuotaExceeded(msg),
        18 => EngineError::RateLimited(msg),
        // 6 无权限，14 IAM 鉴权失败，110/111 token 无效或过期
        6 | 14 | 110 | 111 => EngineError::Unauthorized(msg),
        216200..=216202 => EngineError::InvalidInput(msg),
        _ => EngineError::Inference(msg),
    })
}

pub(crate) fn parse(v: &Value, scale: f32) -> Vec<TextBlock> {
    let mut blocks = Vec::new();
    for w in v["words_result"].as_array().into_iter().flatten() {
        let text = w["words"].as_str().unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let l = &w["location"];
        let (x, y, ww, hh) = (
            num(&l["left"]),
            num(&l["top"]),
            num(&l["width"]),
            num(&l["height"]),
        );
        let score = w.pointer("/probability/average").map(num).unwrap_or(1.0);
        blocks.push(TextBlock::from_rect(
            text,
            [x * scale, y * scale, (x + ww) * scale, (y + hh) * scale],
            score,
        ));
    }
    blocks
}

impl OcrEngine for Baidu {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        // 限制：base64 后不超过 10MB，最长边不超过 8192
        let (data, scale) = encode(image, Encoding::Jpeg(92), 4096)?;
        let token = self.access_token()?;
        let url = format!("https://aip.baidubce.com/rest/2.0/ocr/v1/{}", self.model);
        let img = b64(&data);
        let v = read_json(
            self.agent
                .post(&url)
                .query("access_token", &token)
                .send_form([("image", img.as_str()), ("probability", "true")]),
        )?;
        if matches!(v.get("error_code").and_then(Value::as_i64), Some(110 | 111))
            && let Ok(mut g) = self.token.lock()
        {
            *g = None;
        }
        api_error(&v)?;
        Ok(finish_page(&self.info, image, parse(&v, scale), start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_words_result() {
        let v: Value = serde_json::from_str(
            r#"{"words_result":[{"words":"你好","location":{"left":10,"top":20,"width":30,"height":12},"probability":{"average":0.9}}],"words_result_num":1}"#,
        )
        .unwrap();
        let b = parse(&v, 1.0);
        assert_eq!(b[0].bounds(), [10.0, 20.0, 40.0, 32.0]);
        assert!((b[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn error_codes() {
        let q: Value = serde_json::from_str(
            r#"{"error_code":17,"error_msg":"Open api daily request limit reached"}"#,
        )
        .unwrap();
        assert!(matches!(api_error(&q), Err(EngineError::QuotaExceeded(_))));
        let a: Value =
            serde_json::from_str(r#"{"error_code":110,"error_msg":"Access token invalid"}"#)
                .unwrap();
        assert!(matches!(api_error(&a), Err(EngineError::Unauthorized(_))));
    }
}
