//! AnyOCR 引擎实现。
//!
//! - [`local`]：本地离线 PP-OCR 引擎（oar-ocr + ONNX Runtime）。
//! - [`online`]：在线 OCR 服务（PaddleOCR、百度、腾讯、有道、Google、Azure、OCR.space、Mistral）与 MinerU 文档解析。
//! - [`chain`]：按优先级依次尝试的 fallback 链。
//! - [`credentials`]：凭据（系统凭据管理器 + 环境变量），桌面端与命令行共用。
//! - [`vlm`]：视觉大模型（OpenAI 兼容）：识别引擎与并行复核。

pub mod chain;
pub mod credentials;
pub mod local;
pub mod online;
pub mod vlm;

pub use chain::ChainEngine;
pub use local::{Device, LocalConfig, LocalEngine, LocalModel};
