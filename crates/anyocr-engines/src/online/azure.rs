//! Azure AI Vision Image Analysis 4.0 的 Read 功能（同步接口）。

use std::time::Instant;

use anyocr_core::{EngineError, EngineInfo, OcrEngine, PageResult, TextBlock};
use image::RgbImage;
use serde_json::Value;

use super::*;

pub fn preset() -> Preset {
    Preset {
        info: info(
            "azure_vision",
            "Azure AI Vision（Read）",
            "https://portal.azure.com/#create/Microsoft.CognitiveServicesComputerVision",
            "https://learn.microsoft.com/azure/ai-services/computer-vision/overview-ocr",
            true,
        ),
        fields: vec![
            Field::text("endpoint", "终结点", "")
                .required()
                .hint("如 https://<资源名>.cognitiveservices.azure.com"),
            Field::secret("api_key", "密钥"),
        ],
        note: "在 Azure 门户创建“计算机视觉”资源后，在“密钥和终结点”页复制。F0 免费层每月 5000 次。",
        verified: false,
    }
}

pub struct Azure {
    info: EngineInfo,
    endpoint: String,
    key: String,
    agent: ureq::Agent,
}

impl Azure {
    pub fn new(config: OnlineConfig) -> Result<Self, EngineError> {
        let endpoint = config.require("endpoint", "终结点")?;
        Ok(Self {
            info: preset().info,
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            key: config.require("api_key", "密钥")?,
            agent: agent(config.timeout()),
        })
    }
}

pub(crate) fn parse(v: &Value, scale: f32) -> Vec<TextBlock> {
    let mut out = Vec::new();
    let blocks = v.pointer("/readResult/blocks").and_then(Value::as_array);
    for line in blocks
        .into_iter()
        .flatten()
        .flat_map(|b| b["lines"].as_array().into_iter().flatten())
    {
        let text = line["text"].as_str().unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let pts: Vec<[f32; 2]> = line["boundingPolygon"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| [num(&p["x"]), num(&p["y"])])
            .collect();
        let confs: Vec<f32> = line["words"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| w["confidence"].as_f64().map(|c| c as f32))
            .collect();
        let score = if confs.is_empty() {
            1.0
        } else {
            confs.iter().sum::<f32>() / confs.len() as f32
        };
        if let Some(q) = quad_from_points(&pts, scale) {
            out.push(TextBlock::new(text, q, score));
        }
    }
    out
}

impl OcrEngine for Azure {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        // 限制：图片不超过 20MB（F0 为 4MB），边长 50~16000
        let (data, scale) = encode(image, Encoding::Jpeg(90), 4096)?;
        let (status, body) = read_text(
            self.agent
                .post(format!(
                    "{}/computervision/imageanalysis:analyze",
                    self.endpoint
                ))
                .query("api-version", "2024-02-01")
                .query("features", "read")
                .header("Ocp-Apim-Subscription-Key", &self.key)
                .header("Content-Type", "application/octet-stream")
                .send(&data[..]),
        )?;
        if status == 403 && body.to_lowercase().contains("quota") {
            return Err(EngineError::QuotaExceeded(truncate(&body, 300)));
        }
        check_status(status, &body)?;
        let v = parse_json(&body)?;
        Ok(finish_page(&self.info, image, parse(&v, scale), start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_read_result() {
        let v: Value = serde_json::from_str(
            r#"{"readResult":{"blocks":[{"lines":[{"text":"Azure","boundingPolygon":[{"x":1,"y":1},{"x":9,"y":1},{"x":9,"y":5},{"x":1,"y":5}],"words":[{"confidence":0.5},{"confidence":0.7}]}]}]}}"#,
        )
        .unwrap();
        let b = parse(&v, 2.0);
        assert_eq!(b[0].bounds(), [2.0, 2.0, 18.0, 10.0]);
        assert!((b[0].score - 0.6).abs() < 1e-6);
    }
}
