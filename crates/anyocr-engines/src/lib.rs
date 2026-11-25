//! AnyOCR 引擎实现。
//!
//! - [`local`]：本地离线 PP-OCR 引擎（oar-ocr + ONNX Runtime）。
//! - 在线 OCR 服务与视觉大模型引擎在后续里程碑加入。

pub mod local;

pub use local::{Device, LocalConfig, LocalEngine, LocalModel};
