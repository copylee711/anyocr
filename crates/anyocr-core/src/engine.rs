//! OCR 引擎接口。本地、在线、VLM 引擎都实现 [`OcrEngine`]，输出统一的 [`PageResult`]。

use image::RgbImage;
use serde::{Deserialize, Serialize};

use crate::types::PageResult;

/// 引擎类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    /// 本机离线推理。
    Local,
    /// 在线 OCR 服务。
    Online,
    /// 视觉大模型。
    Vlm,
}

/// 引擎元数据，供设置页展示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInfo {
    /// 稳定的标识，如 `local`、`paddle_aistudio`、`mineru`。
    pub id: String,
    /// 显示名称。
    pub name: String,
    pub kind: EngineKind,
    /// 官方 API Key 获取页，在线引擎与 VLM 提供，设置页显示“获取 API Key”跳转按钮。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_url: Option<String>,
    /// 官方主页或文档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// 是否输出行级坐标（决定能否用于生成 PDF 文字层）。
    pub has_boxes: bool,
}

/// 引擎错误。分类是为了 fallback：遇到可恢复的错误时切换到下一个引擎。
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("引擎未配置：{0}")]
    NotConfigured(String),
    #[error("鉴权失败：{0}")]
    Unauthorized(String),
    #[error("配额已用尽：{0}")]
    QuotaExceeded(String),
    #[error("请求被限流：{0}")]
    RateLimited(String),
    #[error("超时：{0}")]
    Timeout(String),
    #[error("网络错误：{0}")]
    Network(String),
    #[error("模型加载失败：{0}")]
    Model(String),
    #[error("输入无效：{0}")]
    InvalidInput(String),
    #[error("推理失败：{0}")]
    Inference(String),
    #[error("已取消")]
    Cancelled,
}

impl EngineError {
    /// 是否应切换到 fallback 链中的下一个引擎。输入无效、主动取消不切换。
    pub fn should_fallback(&self) -> bool {
        !matches!(self, EngineError::InvalidInput(_) | EngineError::Cancelled)
    }
}

/// 同步 OCR 引擎接口。实现须线程安全，调用方可在多个线程并行调用。
pub trait OcrEngine: Send + Sync {
    fn info(&self) -> &EngineInfo;

    /// 识别一张图片，返回未经排版解析的文本块（`end` 默认为换行）。
    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError>;
}
