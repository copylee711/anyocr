//! 腾讯云文字识别：通用印刷体识别（高精度版 / 标准版），TC3-HMAC-SHA256 签名。

use std::time::Instant;

use anyocr_core::{EngineError, EngineInfo, OcrEngine, PageResult, TextBlock};
use hmac::{Hmac, Mac};
use image::RgbImage;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;

const HOST: &str = "ocr.tencentcloudapi.com";
const CONTENT_TYPE: &str = "application/json; charset=utf-8";

pub fn preset() -> Preset {
    Preset {
        info: info(
            "tencent",
            "腾讯云 OCR",
            "https://console.cloud.tencent.com/cam/capi",
            "https://cloud.tencent.com/product/ocr",
            true,
        ),
        fields: vec![
            Field::secret("secret_id", "SecretId"),
            Field::secret("secret_key", "SecretKey"),
            Field::text("model", "接口", "GeneralAccurateOCR")
                .options(&["GeneralAccurateOCR", "GeneralBasicOCR"])
                .hint("GeneralAccurateOCR：通用印刷体高精度版；GeneralBasicOCR：标准版"),
            Field::text("region", "地域", "ap-guangzhou"),
        ],
        note: "需在控制台开通文字识别服务，有每月免费额度。",
        verified: false,
    }
}

pub struct Tencent {
    info: EngineInfo,
    secret_id: String,
    secret_key: String,
    action: String,
    region: String,
    agent: ureq::Agent,
}

impl Tencent {
    pub fn new(config: OnlineConfig) -> Result<Self, EngineError> {
        let action = match config.get("model") {
            Some("GeneralBasicOCR") => "GeneralBasicOCR",
            _ => "GeneralAccurateOCR",
        };
        Ok(Self {
            info: preset().info,
            secret_id: config.require("secret_id", "SecretId")?,
            secret_key: config.require("secret_key", "SecretKey")?,
            action: action.into(),
            region: config.get("region").unwrap_or("ap-guangzhou").into(),
            agent: agent(config.timeout()),
        })
    }
}

fn hmac(key: &[u8], msg: &str) -> Vec<u8> {
    // HMAC 接受任意长度的密钥，这里不会失败
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
    m.update(msg.as_bytes());
    m.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// 计算 TC3-HMAC-SHA256 的 Authorization 头。
pub(crate) fn authorization(
    secret_id: &str,
    secret_key: &str,
    service: &str,
    host: &str,
    timestamp: i64,
    payload: &str,
) -> String {
    let date = chrono::DateTime::from_timestamp(timestamp, 0)
        .unwrap_or_default()
        .format("%Y-%m-%d")
        .to_string();
    let signed_headers = "content-type;host";
    let canonical = format!(
        "POST\n/\n\ncontent-type:{CONTENT_TYPE}\nhost:{host}\n\n{signed_headers}\n{}",
        sha256_hex(payload.as_bytes())
    );
    let scope = format!("{date}/{service}/tc3_request");
    let to_sign = format!(
        "TC3-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
        sha256_hex(canonical.as_bytes())
    );
    let k_date = hmac(format!("TC3{secret_key}").as_bytes(), &date);
    let k_service = hmac(&k_date, service);
    let k_signing = hmac(&k_service, "tc3_request");
    let signature = hex::encode(hmac(&k_signing, &to_sign));
    format!(
        "TC3-HMAC-SHA256 Credential={secret_id}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    )
}

fn api_error(resp: &Value) -> Result<bool, EngineError> {
    let Some(err) = resp.get("Error") else {
        return Ok(true);
    };
    let code = err["Code"].as_str().unwrap_or_default();
    let msg = format!("{code} {}", err["Message"].as_str().unwrap_or_default());
    if code == "FailedOperation.ImageNoText" {
        return Ok(false);
    }
    Err(
        if code.starts_with("AuthFailure") || code == "UnauthorizedOperation" {
            EngineError::Unauthorized(msg)
        } else if code.contains("LimitExceeded") {
            EngineError::RateLimited(msg)
        } else if code.starts_with("ResourceUnavailable")
            || code.starts_with("ResourcesSoldOut")
            || code == "FailedOperation.UnOpenError"
        {
            EngineError::QuotaExceeded(msg)
        } else if code.starts_with("FailedOperation.Image") || code.starts_with("InvalidParameter")
        {
            EngineError::InvalidInput(msg)
        } else {
            EngineError::Inference(msg)
        },
    )
}

pub(crate) fn parse(resp: &Value, scale: f32) -> Vec<TextBlock> {
    let mut blocks = Vec::new();
    for d in resp["TextDetections"].as_array().into_iter().flatten() {
        let text = d["DetectedText"].as_str().unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let pts: Vec<[f32; 2]> = d["Polygon"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| [num(&p["X"]), num(&p["Y"])])
            .collect();
        let Some(q) = quad_from_points(&pts, scale) else {
            continue;
        };
        let score = (num(&d["Confidence"]) / 100.0).clamp(0.0, 1.0);
        blocks.push(TextBlock::new(text, q, score));
    }
    blocks
}

impl OcrEngine for Tencent {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let start = Instant::now();
        // 限制：base64 后不超过 7MB
        let (data, scale) = encode(image, Encoding::Jpeg(90), 4096)?;
        let payload = json!({ "ImageBase64": b64(&data) }).to_string();
        let ts = chrono::Utc::now().timestamp();
        let auth = authorization(&self.secret_id, &self.secret_key, "ocr", HOST, ts, &payload);
        let v = read_json(
            self.agent
                .post(format!("https://{HOST}/"))
                .header("Authorization", auth)
                .header("Content-Type", CONTENT_TYPE)
                .header("Host", HOST)
                .header("X-TC-Action", &self.action)
                .header("X-TC-Version", "2018-11-19")
                .header("X-TC-Region", &self.region)
                .header("X-TC-Timestamp", ts.to_string())
                .send(payload),
        )?;
        let resp = &v["Response"];
        let blocks = if api_error(resp)? {
            parse(resp, scale)
        } else {
            Vec::new()
        };
        Ok(finish_page(&self.info, image, blocks, start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_shape_and_determinism() {
        let a = authorization("AKIDtest", "secret", "ocr", HOST, 1_551_113_065, "{}");
        let b = authorization("AKIDtest", "secret", "ocr", HOST, 1_551_113_065, "{}");
        assert_eq!(a, b);
        assert!(a.starts_with("TC3-HMAC-SHA256 Credential=AKIDtest/2019-02-25/ocr/tc3_request, SignedHeaders=content-type;host, Signature="));
        let sig = a.rsplit('=').next().unwrap();
        assert_eq!(sig.len(), 64);
        assert_ne!(
            a,
            authorization("AKIDtest", "secret2", "ocr", HOST, 1_551_113_065, "{}")
        );
    }

    #[test]
    fn parses_and_classifies() {
        let ok: Value = serde_json::from_str(
            r#"{"TextDetections":[{"DetectedText":"腾讯","Confidence":99,"Polygon":[{"X":1,"Y":2},{"X":11,"Y":2},{"X":11,"Y":9},{"X":1,"Y":9}]}],"RequestId":"r"}"#,
        )
        .unwrap();
        assert!(api_error(&ok).unwrap());
        let b = parse(&ok, 1.0);
        assert_eq!(b[0].text, "腾讯");
        assert!((b[0].score - 0.99).abs() < 1e-6);

        let none: Value = serde_json::from_str(
            r#"{"Error":{"Code":"FailedOperation.ImageNoText","Message":""}}"#,
        )
        .unwrap();
        assert!(!api_error(&none).unwrap());
        let auth: Value = serde_json::from_str(
            r#"{"Error":{"Code":"AuthFailure.SignatureFailure","Message":""}}"#,
        )
        .unwrap();
        assert!(matches!(
            api_error(&auth),
            Err(EngineError::Unauthorized(_))
        ));
        let quota: Value = serde_json::from_str(
            r#"{"Error":{"Code":"ResourceUnavailable.InArrears","Message":""}}"#,
        )
        .unwrap();
        assert!(matches!(
            api_error(&quota),
            Err(EngineError::QuotaExceeded(_))
        ));
    }
}
