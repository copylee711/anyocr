//! OCR.space：有免费 Key 的通用 OCR，返回单词级坐标，按行合并。

use std::time::Instant;

use anyocr_core::{EngineError, EngineInfo, OcrEngine, PageResult, TextBlock};
use image::RgbImage;
use serde_json::Value;

use super::*;

pub fn preset() -> Preset {
    Preset {
        info: info(
            "ocr_space",
            "OCR.space",
            "https://ocr.space/ocrapi/freekey",
            "https://ocr.space/ocrapi",
            true,
        ),
        fields: vec![
            Field::secret("api_key", "API Key"),
            Field::text("engine", "识别引擎", "2")
                .options(&["1", "2", "3"])
                .hint("2 适合拉丁文字；中文请选 1 或 3，并把语言设为 chs"),
            Field::text("language", "语言", "auto").hint("auto、chs、cht、eng、jpn、kor 等"),
        ],
        note: "免费 Key 每月 25000 次，单张图片不超过 1MB（会自动压缩）。",
        verified: true,
    }
}

pub struct OcrSpace {
    info: EngineInfo,
    key: String,
    engine: String,
    language: String,
    agent: ureq::Agent,
}

impl OcrSpace {
    pub fn new(config: OnlineConfig) -> Result<Self, EngineError> {
        Ok(Self {
            info: preset().info,
            key: config.require("api_key", "API Key")?,
            engine: config.get("engine").unwrap_or("2").into(),
            language: config.get("language").unwrap_or("auto").into(),
            agent: agent(config.timeout()),
        })
    }
}

pub(crate) fn parse(v: &Value, scale: f32) -> Vec<TextBlock> {
    let mut out = Vec::new();
    let results = v["ParsedResults"].as_array();
    for line in results.into_iter().flatten().flat_map(|r| {
        r.pointer("/TextOverlay/Lines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }) {
        let text = line["LineText"].as_str().unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for w in line["Words"].as_array().into_iter().flatten() {
            let (x, y) = (num(&w["Left"]), num(&w["Top"]));
            r = [
                r[0].min(x),
                r[1].min(y),
                r[2].max(x + num(&w["Width"])),
                r[3].max(y + num(&w["Height"])),
            ];
        }
        if r[0] <= r[2] && r[1] <= r[3] {
            out.push(TextBlock::from_rect(text, r.map(|c| c * scale), 1.0));
        }
    }
    out
}

fn classify(status: u16, body: &str) -> Result<(), EngineError> {
    let lower = body.to_lowercase();
    if lower.contains("api key is invalid") || lower.contains("invalid api key") {
        return Err(EngineError::Unauthorized(truncate(body, 300)));
    }
    if lower.contains("maximum") && lower.contains("times within") {
        return Err(EngineError::RateLimited(truncate(body, 300)));
    }
    check_status(status, body)
}

impl OcrEngine for OcrSpace {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        // 免费 Key 限制 1MB：逐步降低尺寸与质量
        let mut attempt = (2400u32, 85u8);
        let (data, scale) = loop {
            let (d, s) = encode(image, Encoding::Jpeg(attempt.1), attempt.0)?;
            if d.len() < 950 * 1024 || attempt.0 <= 800 {
                break (d, s);
            }
            attempt = (attempt.0 * 4 / 5, attempt.1.saturating_sub(10).max(50));
        };
        let data_url = format!("data:image/jpeg;base64,{}", b64(&data));
        let (ct, body) = Multipart::new()
            .text("base64Image", &data_url)
            .text("language", &self.language)
            .text("OCREngine", &self.engine)
            .text("isOverlayRequired", "true")
            .text("scale", "true")
            .finish();
        let (status, text) = read_text(
            self.agent
                .post("https://api.ocr.space/parse/image")
                .header("apikey", &self.key)
                .header("Content-Type", ct)
                .send(&body[..]),
        )?;
        classify(status, &text)?;
        let v = parse_json(&text)?;
        if v["IsErroredOnProcessing"].as_bool() == Some(true) {
            let msg = match &v["ErrorMessage"] {
                Value::Array(a) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("；"),
                other => other.as_str().unwrap_or("识别失败").to_owned(),
            };
            classify(400, &msg)?;
            return Err(EngineError::Inference(msg));
        }
        Ok(finish_page(&self.info, image, parse(&v, scale), start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_words_into_line_box() {
        let v: Value = serde_json::from_str(
            r#"{"ParsedResults":[{"TextOverlay":{"Lines":[
                {"LineText":"Hello world","Words":[{"Left":10,"Top":5,"Width":20,"Height":8},{"Left":35,"Top":6,"Width":25,"Height":8}]},
                {"LineText":"One","Words":[{"Left":1,"Top":20,"Width":9,"Height":7}]}
            ]}}],"IsErroredOnProcessing":false}"#,
        )
        .unwrap();
        let b = parse(&v, 1.0);
        assert_eq!(b[0].bounds(), [10.0, 5.0, 60.0, 14.0]);
        assert_eq!(b[1].bounds(), [1.0, 20.0, 10.0, 27.0]);
    }

    #[test]
    fn rate_limit_message() {
        let r = classify(
            403,
            "You may only perform this action upto maximum 180 number of times within 3600 seconds",
        );
        assert!(matches!(r, Err(EngineError::RateLimited(_))));
    }

    /// 使用 OCR.space 公开的演示 Key（helloworld），只发送仓库里的英文样张。
    #[test]
    #[ignore]
    fn live_demo_key() {
        let mut cfg = OnlineConfig::default();
        cfg.fields.insert("api_key".into(), "helloworld".into());
        let engine = OcrSpace::new(cfg).unwrap();
        let page = engine.recognize(&super::super::test_image()).unwrap();
        assert!(page.text().contains("AnyOCR"), "{}", page.text());
        assert!(!page.blocks.is_empty());
    }
}
