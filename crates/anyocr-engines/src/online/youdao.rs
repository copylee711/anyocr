//! 有道智云通用文字识别（`/ocrapi`），v3 签名（SHA-256）。

use std::time::Instant;

use anyocr_core::{EngineError, EngineInfo, OcrEngine, PageResult, TextBlock};
use image::RgbImage;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::*;

pub fn preset() -> Preset {
    Preset {
        info: info(
            "youdao",
            "有道智云 OCR",
            "https://ai.youdao.com/console/",
            "https://ai.youdao.com/product-ocr.s",
            true,
        ),
        fields: vec![
            Field::secret("app_key", "应用 ID"),
            Field::secret("app_secret", "应用密钥"),
            Field::text("lang", "语言", "auto").hint("auto 为自动识别，也可填 zh-CHS、en 等"),
        ],
        note: "需在控制台创建应用并绑定“通用 OCR”服务。",
        verified: false,
    }
}

pub struct Youdao {
    info: EngineInfo,
    app_key: String,
    app_secret: String,
    lang: String,
    agent: ureq::Agent,
}

impl Youdao {
    pub fn new(config: OnlineConfig) -> Result<Self, EngineError> {
        Ok(Self {
            info: preset().info,
            app_key: config.require("app_key", "应用 ID")?,
            app_secret: config.require("app_secret", "应用密钥")?,
            lang: config.get("lang").unwrap_or("auto").into(),
            agent: agent(config.timeout()),
        })
    }
}

/// v3 签名：`sha256(appKey + input + salt + curtime + appSecret)`，input 为截断后的待识别内容。
pub(crate) fn sign(app_key: &str, q: &str, salt: &str, curtime: &str, app_secret: &str) -> String {
    let chars: Vec<char> = q.chars().collect();
    let input = if chars.len() <= 20 {
        q.to_owned()
    } else {
        let head: String = chars[..10].iter().collect();
        let tail: String = chars[chars.len() - 10..].iter().collect();
        format!("{head}{}{tail}", chars.len())
    };
    hex::encode(Sha256::digest(
        format!("{app_key}{input}{salt}{curtime}{app_secret}").as_bytes(),
    ))
}

fn api_error(v: &Value) -> Result<(), EngineError> {
    let code = v["errorCode"].as_str().unwrap_or("0");
    if code == "0" {
        return Ok(());
    }
    let msg = format!("错误码 {code}");
    Err(match code {
        "101" | "102" | "103" | "113" | "1001" | "1002" | "1003" => EngineError::InvalidInput(msg),
        "108" | "110" | "111" | "202" | "206" => EngineError::Unauthorized(msg),
        "401" | "402" => EngineError::QuotaExceeded(msg),
        "411" | "412" => EngineError::RateLimited(msg),
        _ => EngineError::Inference(msg),
    })
}

pub(crate) fn parse(v: &Value, scale: f32) -> Vec<TextBlock> {
    let mut blocks = Vec::new();
    for region in v
        .pointer("/Result/regions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for line in region["lines"].as_array().into_iter().flatten() {
            let text = line["text"].as_str().unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            let nums: Vec<f32> = line["boundingBox"]
                .as_str()
                .unwrap_or_default()
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            let pts: Vec<[f32; 2]> = nums.as_chunks::<2>().0.to_vec();
            if let Some(q) = quad_from_points(&pts, scale) {
                blocks.push(TextBlock::new(text, q, 1.0));
            }
        }
    }
    blocks
}

impl OcrEngine for Youdao {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        let (data, scale) = encode(image, Encoding::Jpeg(90), 4096)?;
        let img = b64(&data);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let curtime = now.as_secs().to_string();
        let salt = format!("{:x}", now.as_nanos());
        let sign = sign(&self.app_key, &img, &salt, &curtime, &self.app_secret);
        let v = read_json(
            self.agent
                .post("https://openapi.youdao.com/ocrapi")
                .send_form([
                    ("img", img.as_str()),
                    ("langType", self.lang.as_str()),
                    ("detectType", "10012"),
                    ("imageType", "1"),
                    ("appKey", self.app_key.as_str()),
                    ("salt", salt.as_str()),
                    ("sign", sign.as_str()),
                    ("signType", "v3"),
                    ("curtime", curtime.as_str()),
                    ("docType", "json"),
                ]),
        )?;
        api_error(&v)?;
        Ok(finish_page(&self.info, image, parse(&v, scale), start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_truncates_long_input() {
        let long = "a".repeat(10) + &"b".repeat(5) + &"c".repeat(10);
        assert_eq!(
            sign("k", &long, "s", "1", "x"),
            hex::encode(Sha256::digest(
                format!("k{}25{}s1x", "a".repeat(10), "c".repeat(10)).as_bytes()
            ))
        );
        assert_eq!(
            sign("k", "short", "s", "1", "x"),
            hex::encode(Sha256::digest(b"kshorts1x"))
        );
    }

    #[test]
    fn parses_regions() {
        let v: Value = serde_json::from_str(
            r#"{"errorCode":"0","Result":{"regions":[{"lines":[{"boundingBox":"1,2,11,2,11,9,1,9","text":"有道"}]}]}}"#,
        )
        .unwrap();
        let b = parse(&v, 1.0);
        assert_eq!(b[0].bounds(), [1.0, 2.0, 11.0, 9.0]);
        let e: Value = serde_json::from_str(r#"{"errorCode":"401"}"#).unwrap();
        assert!(matches!(api_error(&e), Err(EngineError::QuotaExceeded(_))));
    }
}
