//! PaddleOCR（百度 AI Studio）异步任务接口，模型 PP-OCRv5，返回行级四点框。
//!
//! 流程：`POST /api/v2/ocr/jobs`（multipart 上传图片）→ 轮询 `GET /api/v2/ocr/jobs/{id}` →
//! 下载 `resultUrl.jsonUrl`（JSONL，每行一页）。高峰期可能排队较久，超时后交给 fallback。

use std::time::{Duration, Instant};

use anyocr_core::{EngineError, EngineInfo, OcrEngine, PageResult, TextBlock};
use image::RgbImage;
use serde_json::Value;

use super::*;

const JOBS: &str = "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs";

pub fn preset() -> Preset {
    Preset {
        info: info(
            "paddle_aistudio",
            "PaddleOCR（AI Studio）",
            "https://aistudio.baidu.com/account/accessToken",
            "https://aistudio.baidu.com/paddleocr",
            true,
        ),
        fields: vec![Field::secret("token", "访问令牌")],
        note: "PP-OCRv5，行级坐标，有每日免费页数。异步排队，高峰期可能较慢。",
        verified: true,
    }
}

pub struct Paddle {
    info: EngineInfo,
    token: String,
    agent: ureq::Agent,
    deadline: Duration,
}

impl Paddle {
    pub fn new(config: OnlineConfig) -> Result<Self, EngineError> {
        Ok(Self {
            info: preset().info,
            token: config.require("token", "访问令牌")?,
            agent: agent(Duration::from_secs(60)),
            deadline: if config.timeout_secs == 0 {
                Duration::from_secs(180)
            } else {
                config.timeout()
            },
        })
    }

    fn auth(&self) -> String {
        format!("bearer {}", self.token)
    }

    fn api(&self, v: Value) -> Result<Value, EngineError> {
        api_error(&v)?;
        Ok(v)
    }
}

/// AI Studio 在 JSON 中用 `code` / `errorCode` 表示业务错误。
fn api_error(v: &Value) -> Result<(), EngineError> {
    let code = v
        .get("code")
        .or_else(|| v.get("errorCode"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if code == 0 {
        return Ok(());
    }
    let msg = format!(
        "{code} {}",
        v.get("msg")
            .or_else(|| v.get("errorMsg"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    );
    Err(match code {
        12001 => EngineError::QuotaExceeded(msg),
        12002 => EngineError::RateLimited(msg),
        401 | 403 | 11001..=11999 => EngineError::Unauthorized(msg),
        _ => EngineError::Inference(msg),
    })
}

/// 解析结果 JSONL 的第一页。
pub(crate) fn parse_page(line: &Value, scale: f32) -> Result<Vec<TextBlock>, EngineError> {
    let pruned = line
        .pointer("/result/ocrResults/0/prunedResult")
        .ok_or_else(|| EngineError::Inference("结果中没有 ocrResults".into()))?;
    let texts = pruned["rec_texts"].as_array().cloned().unwrap_or_default();
    let scores = pruned["rec_scores"].as_array().cloned().unwrap_or_default();
    let polys = pruned["rec_polys"].as_array().cloned().unwrap_or_default();
    let mut blocks = Vec::with_capacity(texts.len());
    for (i, t) in texts.iter().enumerate() {
        let text = t.as_str().unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let pts: Vec<[f32; 2]> = polys
            .get(i)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|p| Some([num(p.get(0)?), num(p.get(1)?)]))
                    .collect()
            })
            .unwrap_or_default();
        let Some(q) = quad_from_points(&pts, scale) else {
            continue;
        };
        let score = scores.get(i).map(num).unwrap_or(1.0);
        blocks.push(TextBlock::new(text, q, score));
    }
    Ok(blocks)
}

impl OcrEngine for Paddle {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        let (data, scale) = encode(image, Encoding::Jpeg(92), 4000)?;
        let payload = r#"{"useDocOrientationClassify":false,"useDocUnwarping":false,"useTextlineOrientation":false}"#;
        let (ct, body) = Multipart::new()
            .text("model", "PP-OCRv5")
            .text("optionalPayload", payload)
            .file("file", "page.jpg", Encoding::Jpeg(0).mime(), &data)
            .finish();
        let submit = self.api(read_json(
            self.agent
                .post(JOBS)
                .header("Authorization", self.auth())
                .header("Content-Type", ct)
                .send(&body[..]),
        )?)?;
        let job = submit
            .pointer("/data/jobId")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::Inference("提交任务后没有返回 jobId".into()))?
            .to_owned();

        let mut wait = Duration::from_millis(800);
        let json_url = loop {
            if start.elapsed() > self.deadline {
                return Err(EngineError::Timeout(format!(
                    "排队超过 {} 秒",
                    self.deadline.as_secs()
                )));
            }
            std::thread::sleep(wait);
            wait = (wait * 3 / 2).min(Duration::from_secs(4));
            let s = self.api(read_json(
                self.agent
                    .get(format!("{JOBS}/{job}"))
                    .header("Authorization", self.auth())
                    .call(),
            )?)?;
            match s.pointer("/data/state").and_then(Value::as_str) {
                Some("done") => {
                    break s
                        .pointer("/data/resultUrl/jsonUrl")
                        .and_then(Value::as_str)
                        .ok_or_else(|| EngineError::Inference("任务完成但没有结果地址".into()))?
                        .to_owned();
                }
                Some("failed") => {
                    let msg = s
                        .pointer("/data/errorMsg")
                        .and_then(Value::as_str)
                        .unwrap_or("任务失败");
                    return Err(EngineError::Inference(msg.to_owned()));
                }
                _ => {}
            }
        };
        let (status, text) = read_text(self.agent.get(&json_url).call())?;
        check_status(status, &text)?;
        let first = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .ok_or_else(|| EngineError::Inference("结果为空".into()))?;
        let line = parse_json(first)?;
        api_error(&line)?;
        let blocks = parse_page(&line, scale)?;
        Ok(finish_page(&self.info, image, blocks, start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jsonl_line() {
        let line: Value = serde_json::from_str(
            r#"{"logId":"x","result":{"ocrResults":[{"prunedResult":{
                "rec_texts":["第5章","","067"],
                "rec_scores":[0.99,0.5,0.98],
                "rec_polys":[[[788,128],[1041,128],[1041,150],[788,150]],[[0,0],[1,0],[1,1],[0,1]],[[1093,128],[1137,128],[1137,153],[1093,153]]]
            }}]},"errorCode":0,"errorMsg":"Success"}"#,
        )
        .unwrap();
        let b = parse_page(&line, 2.0).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].text, "第5章");
        assert_eq!(b[0].bbox[2], [2082.0, 300.0]);
        assert_eq!(b[1].text, "067");
    }

    #[test]
    fn quota_code_maps_to_quota() {
        let v: Value = serde_json::from_str(r#"{"code":12001,"msg":"quota"}"#).unwrap();
        assert!(matches!(api_error(&v), Err(EngineError::QuotaExceeded(_))));
    }

    /// 需要 `ANYOCR_PADDLE_TOKEN`，会消耗 1 页免费额度。
    #[test]
    #[ignore]
    fn live_recognize() {
        let cfg = config_from_env(&preset());
        let Ok(engine) = Paddle::new(cfg) else { return };
        let img = super::super::test_image();
        let page = engine.recognize(&img).unwrap();
        assert!(page.text().contains("AnyOCR"), "{}", page.text());
    }
}
