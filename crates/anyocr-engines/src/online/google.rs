//! Google Cloud Vision：`DOCUMENT_TEXT_DETECTION`，按符号的换行标记拼成行。

use std::time::Instant;

use anyocr_core::{EngineError, EngineInfo, OcrEngine, PageResult, TextBlock};
use image::RgbImage;
use serde_json::{Value, json};

use super::*;

pub fn preset() -> Preset {
    Preset {
        info: info(
            "google_vision",
            "Google Cloud Vision",
            "https://console.cloud.google.com/apis/credentials",
            "https://cloud.google.com/vision/docs/ocr",
            true,
        ),
        fields: vec![
            Field::secret("api_key", "API Key"),
            Field::text("language", "语言提示", "").hint("可留空，或填 zh、en 等，用逗号分隔"),
        ],
        note: "需在项目中启用 Cloud Vision API 并绑定结算账号，每月有免费额度。",
        verified: false,
    }
}

pub struct Google {
    info: EngineInfo,
    key: String,
    languages: Vec<String>,
    agent: ureq::Agent,
}

impl Google {
    pub fn new(config: OnlineConfig) -> Result<Self, EngineError> {
        Ok(Self {
            info: preset().info,
            key: config.require("api_key", "API Key")?,
            languages: config
                .get("language")
                .unwrap_or_default()
                .split([',', '，', ' '])
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            agent: agent(config.timeout()),
        })
    }
}

fn vertices(v: &Value) -> Vec<[f32; 2]> {
    v.pointer("/boundingBox/vertices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|p| [num(&p["x"]), num(&p["y"])])
        .collect()
}

/// 遍历 page → block → paragraph → word → symbol，遇到 LINE_BREAK / EOL_SURE_SPACE 结束一行。
pub(crate) fn parse(resp: &Value, scale: f32) -> Vec<TextBlock> {
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut pts: Vec<[f32; 2]> = Vec::new();
    let mut conf = Vec::new();
    let mut flush = |text: &mut String, pts: &mut Vec<[f32; 2]>, conf: &mut Vec<f32>| {
        let t = text.trim();
        if !t.is_empty()
            && let Some(q) = quad_from_points(pts, scale)
        {
            let score = if conf.is_empty() {
                1.0
            } else {
                conf.iter().sum::<f32>() / conf.len() as f32
            };
            // quad_from_points 对多于 4 个点取外接矩形
            blocks.push(TextBlock::new(t, q, score));
        }
        text.clear();
        pts.clear();
        conf.clear();
    };
    let pages = resp
        .pointer("/fullTextAnnotation/pages")
        .and_then(Value::as_array);
    for page in pages.into_iter().flatten() {
        for block in page["blocks"].as_array().into_iter().flatten() {
            for para in block["paragraphs"].as_array().into_iter().flatten() {
                for word in para["words"].as_array().into_iter().flatten() {
                    pts.extend(vertices(word));
                    if let Some(c) = word["confidence"].as_f64() {
                        conf.push(c as f32);
                    }
                    for sym in word["symbols"].as_array().into_iter().flatten() {
                        text.push_str(sym["text"].as_str().unwrap_or_default());
                        match sym
                            .pointer("/property/detectedBreak/type")
                            .and_then(Value::as_str)
                        {
                            Some("SPACE" | "SURE_SPACE") => text.push(' '),
                            Some("EOL_SURE_SPACE" | "LINE_BREAK") => {
                                flush(&mut text, &mut pts, &mut conf)
                            }
                            _ => {}
                        }
                    }
                }
                flush(&mut text, &mut pts, &mut conf);
            }
        }
    }
    blocks
}

impl OcrEngine for Google {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        // 限制：请求 JSON 不超过 10MB
        let (data, scale) = encode(image, Encoding::Jpeg(90), 4096)?;
        let mut req = json!({
            "image": { "content": b64(&data) },
            "features": [{ "type": "DOCUMENT_TEXT_DETECTION" }],
        });
        if !self.languages.is_empty() {
            req["imageContext"] = json!({ "languageHints": self.languages });
        }
        let (status, body) = read_text(
            self.agent
                .post("https://vision.googleapis.com/v1/images:annotate")
                .query("key", &self.key)
                .send_json(json!({ "requests": [req] })),
        )?;
        if status == 400 && body.contains("API key not valid") {
            return Err(EngineError::Unauthorized(truncate(&body, 300)));
        }
        if status == 403 && body.contains("billing") {
            return Err(EngineError::QuotaExceeded(truncate(&body, 300)));
        }
        check_status(status, &body)?;
        let v = parse_json(&body)?;
        let resp = &v["responses"][0];
        if let Some(err) = resp.get("error") {
            let msg = err["message"].as_str().unwrap_or_default().to_owned();
            return Err(match err["code"].as_i64() {
                Some(7 | 16) => EngineError::Unauthorized(msg),
                Some(8) => EngineError::QuotaExceeded(msg),
                Some(3) => EngineError::InvalidInput(msg),
                _ => EngineError::Inference(msg),
            });
        }
        Ok(finish_page(&self.info, image, parse(resp, scale), start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines_on_breaks() {
        let v: Value = serde_json::from_str(
            r#"{"fullTextAnnotation":{"pages":[{"blocks":[{"paragraphs":[{"words":[
              {"boundingBox":{"vertices":[{"x":0,"y":0},{"x":10,"y":0},{"x":10,"y":8},{"x":0,"y":8}]},"confidence":0.9,
               "symbols":[{"text":"H"},{"text":"i","property":{"detectedBreak":{"type":"SPACE"}}}]},
              {"boundingBox":{"vertices":[{"x":12,"y":0},{"x":30,"y":0},{"x":30,"y":8},{"x":12,"y":8}]},"confidence":0.8,
               "symbols":[{"text":"you","property":{"detectedBreak":{"type":"LINE_BREAK"}}}]},
              {"boundingBox":{"vertices":[{"x":0,"y":10},{"x":10,"y":10},{"x":10,"y":18},{"x":0,"y":18}]},
               "symbols":[{"text":"第"},{"text":"二"}]}
            ]}]}]}]}}"#,
        )
        .unwrap();
        let b = parse(&v, 1.0);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].text, "Hi you");
        assert_eq!(b[0].bounds(), [0.0, 0.0, 30.0, 8.0]);
        assert_eq!(b[1].text, "第二");
    }
}
