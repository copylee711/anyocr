//! AnyOCR 核心库。
//!
//! - [`types`]：所有引擎统一输出的数据模型（[`TextBlock`]、[`PageResult`]）。
//! - [`engine`]：OCR 引擎接口与引擎元数据。
//! - [`tbpu`]：文本块后处理（排版解析），移植自 Umi-OCR。
//! - [`review`]：视觉大模型复核的数据结构与防幻觉校验。

pub mod engine;
pub mod review;
pub mod tbpu;
pub mod types;

pub use engine::{EngineError, EngineInfo, EngineKind, OcrEngine};
pub use review::{Guard, LineChange, ReviewOutcome, Reviewer};
pub use types::{BlockSource, PageResult, Point, Quad, TextBlock};
