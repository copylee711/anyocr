//! Mistral OCR：返回整页 Markdown，没有行级坐标。
//!
//! 每个非空行生成一个文本块，框沿页面高度均匀排布，只是占位，不代表真实位置；
//! 因此 `has_boxes = false`，生成 PDF 文字层时调用方应优先选用有坐标的引擎。

use std::time::Instant;

use anyocr_core::{EngineError, EngineInfo, OcrEngine, PageResult, TextBlock};
use image::RgbImage;
use serde_json::json;

use super::*;

pub fn preset() -> Preset {
    Preset {
        info: info(
            "mistral",
            "Mistral OCR",
            "https://console.mistral.ai/api-keys",
            "https://docs.mistral.ai/capabilities/document_ai/basic_ocr/",
            false,
        ),
        fields: vec![
            Field::secret("api_key", "API Key"),
            Field::text("model", "模型", "mistral-ocr-latest"),
        ],
        note: "输出 Markdown（含表格、公式），没有行级坐标，不适合生成 PDF 文字层。",
        verified: false,
    }
}

pub struct Mistral {
    info: EngineInfo,
    key: String,
    model: String,
    agent: ureq::Agent,
}

impl Mistral {
    pub fn new(config: OnlineConfig) -> Result<Self, EngineError> {
        Ok(Self {
            info: preset().info,
            key: config.require("api_key", "API Key")?,
            model: config.get("model").unwrap_or("mistral-ocr-latest").into(),
            agent: agent(config.timeout()),
        })
    }
}

/// 把整页文字按行切开，框沿页面高度均匀排布。
pub(crate) fn lines_to_blocks(text: &str, width: u32, height: u32) -> Vec<TextBlock> {
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
            TextBlock::from_rect(*l, [0.0, y0, w, y0 + h / n], 1.0)
        })
        .collect()
}

impl OcrEngine for Mistral {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        let (data, _) = encode(image, Encoding::Jpeg(90), 4096)?;
        let v = read_json(
            self.agent
                .post("https://api.mistral.ai/v1/ocr")
                .header("Authorization", format!("Bearer {}", self.key))
                .send_json(json!({
                    "model": self.model,
                    "document": {
                        "type": "image_url",
                        "image_url": format!("data:image/jpeg;base64,{}", b64(&data)),
                    },
                })),
        )?;
        let md: Vec<&str> = v["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["markdown"].as_str())
            .collect();
        let blocks = lines_to_blocks(&md.join("\n"), image.width(), image.height());
        Ok(finish_page(&self.info, image, blocks, start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreads_lines() {
        let b = lines_to_blocks("# 标题\n\n正文\n", 100, 200);
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].bounds(), [0.0, 100.0, 100.0, 200.0]);
    }
}
