//! 本地 PP-OCR 引擎，基于 [oar-ocr](https://github.com/greatv/oar-ocr)（Apache-2.0）。
//!
//! 模型文件首次使用时从 ModelScope 自动下载到模型目录（`OAR_HOME`），并做 SHA-256 校验。

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Once;
use std::time::Instant;

use anyocr_core::{EngineError, EngineInfo, EngineKind, OcrEngine, PageResult, TextBlock};
use image::RgbImage;
use oar_ocr::core::config::{OrtExecutionProvider, OrtSessionConfig};
use oar_ocr::domain::tasks::TextDetectionConfig;
use oar_ocr::oarocr::{OAROCR, OAROCRBuilder, PpOcrV6Size};
use serde::{Deserialize, Serialize};

/// 可选的本地模型组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LocalModel {
    /// PP-OCRv6 tiny：最快，约 6 MB。
    PpOcrV6Tiny,
    /// PP-OCRv6 small：速度与精度均衡（默认），约 30 MB。
    #[default]
    PpOcrV6Small,
    /// PP-OCRv6 medium：精度最高，约 130 MB。
    PpOcrV6Medium,
    /// PP-OCRv5 mobile。
    PpOcrV5Mobile,
    /// PP-OCRv5 server。
    PpOcrV5Server,
}

impl LocalModel {
    pub const ALL: [LocalModel; 5] = [
        LocalModel::PpOcrV6Tiny,
        LocalModel::PpOcrV6Small,
        LocalModel::PpOcrV6Medium,
        LocalModel::PpOcrV5Mobile,
        LocalModel::PpOcrV5Server,
    ];

    pub fn key(self) -> &'static str {
        match self {
            LocalModel::PpOcrV6Tiny => "pp-ocrv6-tiny",
            LocalModel::PpOcrV6Small => "pp-ocrv6-small",
            LocalModel::PpOcrV6Medium => "pp-ocrv6-medium",
            LocalModel::PpOcrV5Mobile => "pp-ocrv5-mobile",
            LocalModel::PpOcrV5Server => "pp-ocrv5-server",
        }
    }

    fn builder(self) -> OAROCRBuilder {
        match self {
            LocalModel::PpOcrV6Tiny => OAROCRBuilder::pp_ocrv6(PpOcrV6Size::Tiny),
            LocalModel::PpOcrV6Small => OAROCRBuilder::pp_ocrv6(PpOcrV6Size::Small),
            LocalModel::PpOcrV6Medium => OAROCRBuilder::pp_ocrv6(PpOcrV6Size::Medium),
            LocalModel::PpOcrV5Mobile => OAROCRBuilder::new(
                "pp-ocrv5_mobile_det.onnx",
                "pp-ocrv5_mobile_rec.onnx",
                "ppocrv5_dict.txt",
            ),
            LocalModel::PpOcrV5Server => OAROCRBuilder::new(
                "pp-ocrv5_server_det.onnx",
                "pp-ocrv5_server_rec.onnx",
                "ppocrv5_dict.txt",
            ),
        }
    }
}

impl FromStr for LocalModel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        LocalModel::ALL
            .into_iter()
            .find(|m| m.key() == s)
            .ok_or_else(|| {
                format!(
                    "未知模型：{s}，可选：{}",
                    LocalModel::ALL.map(|m| m.key()).join(", ")
                )
            })
    }
}

/// 推理设备。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Device {
    /// 自动选择：可用的 GPU 后端优先，CPU 兜底。
    #[default]
    Auto,
    Cpu,
    /// DirectML（Windows，任意 GPU），参数为设备序号。
    DirectMl(i32),
    /// NVIDIA CUDA，参数为设备序号。
    Cuda(i32),
}

impl FromStr for Device {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, id) = match s.split_once(':') {
            Some((n, i)) => (
                n,
                i.parse::<i32>().map_err(|e| format!("设备序号无效：{e}"))?,
            ),
            None => (s, 0),
        };
        match name {
            "auto" => Ok(Device::Auto),
            "cpu" => Ok(Device::Cpu),
            "directml" | "dml" => Ok(Device::DirectMl(id)),
            "cuda" => Ok(Device::Cuda(id)),
            _ => Err(format!(
                "未知设备：{s}，可选：auto、cpu、directml[:N]、cuda[:N]"
            )),
        }
    }
}

/// 本地引擎配置。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalConfig {
    pub model: LocalModel,
    pub device: Device,
    /// 模型目录；为空时使用 oar-ocr 默认目录（`~/.oar`）。
    pub models_dir: Option<PathBuf>,
    /// CPU 推理线程数；为空时由 ONNX Runtime 决定。
    pub threads: Option<usize>,
    /// 启用文本行方向分类（识别倒置的文字行）。
    pub textline_orientation: bool,
    /// 启用整页方向分类（自动纠正 90°/180°/270° 旋转的页面）。
    pub doc_orientation: bool,
    /// 检测输入的边长限制；为空时使用模型预设。
    pub limit_side_len: Option<u32>,
}

static MODELS_DIR_ONCE: Once = Once::new();

/// 设置模型下载与缓存目录。只在第一次调用时生效，应在创建任何引擎前、程序启动早期调用。
pub fn set_models_dir(dir: &Path) {
    MODELS_DIR_ONCE.call_once(|| {
        // SAFETY: 仅在程序启动、尚未创建引擎线程时调用一次。
        unsafe { std::env::set_var(oar_ocr::download::OAR_HOME_ENV, dir) };
    });
}

fn session_config(cfg: &LocalConfig) -> Option<OrtSessionConfig> {
    let base = match cfg.device {
        Device::Auto => OrtSessionConfig::auto(),
        Device::Cpu => {
            OrtSessionConfig::new().with_execution_providers(vec![OrtExecutionProvider::CPU])
        }
        Device::DirectMl(id) => OrtSessionConfig::new().with_execution_providers(vec![
            OrtExecutionProvider::DirectML {
                device_id: Some(id),
            },
            OrtExecutionProvider::CPU,
        ]),
        Device::Cuda(id) => OrtSessionConfig::new().with_execution_providers(vec![
            OrtExecutionProvider::CUDA {
                device_id: Some(id),
                gpu_mem_limit: None,
                arena_extend_strategy: None,
                cudnn_conv_algo_search: None,
                cudnn_conv_use_max_workspace: None,
            },
            OrtExecutionProvider::CPU,
        ]),
    };
    Some(match cfg.threads {
        Some(t) if t > 0 => base.with_intra_threads(t),
        _ => base,
    })
}

/// 本地 PP-OCR 引擎。
pub struct LocalEngine {
    info: EngineInfo,
    ocr: OAROCR,
    config: LocalConfig,
}

impl LocalEngine {
    /// 加载模型（必要时下载）。耗时操作，应在后台线程调用。
    pub fn new(config: LocalConfig) -> Result<Self, EngineError> {
        if let Some(dir) = &config.models_dir {
            std::fs::create_dir_all(dir)
                .map_err(|e| EngineError::Model(format!("无法创建模型目录：{e}")))?;
            set_models_dir(dir);
        }
        let mut b = config.model.builder();
        if let Some(sc) = session_config(&config) {
            b = b.ort_session(sc);
        }
        if config.textline_orientation {
            b = b.with_text_line_orientation_classification("pp-lcnet_x0_25_textline_ori.onnx");
        }
        if config.doc_orientation {
            b = b.with_document_image_orientation_classification("pp-lcnet_x1_0_doc_ori.onnx");
        }
        if let Some(len) = config.limit_side_len {
            let det = TextDetectionConfig {
                limit_side_len: Some(len),
                ..preset_detection(config.model)
            };
            b = b.text_detection_config(det);
        }
        let ocr = b.build().map_err(|e| EngineError::Model(e.to_string()))?;
        let info = EngineInfo {
            id: "local".into(),
            name: format!("本地 {}", config.model.key()),
            kind: EngineKind::Local,
            key_url: None,
            homepage: Some("https://github.com/PaddlePaddle/PaddleOCR".into()),
            has_boxes: true,
        };
        Ok(Self { info, ocr, config })
    }

    pub fn config(&self) -> &LocalConfig {
        &self.config
    }
}

fn preset_detection(model: LocalModel) -> TextDetectionConfig {
    match model {
        LocalModel::PpOcrV6Tiny | LocalModel::PpOcrV6Small | LocalModel::PpOcrV6Medium => {
            TextDetectionConfig {
                score_threshold: 0.2,
                box_threshold: if model == LocalModel::PpOcrV6Tiny {
                    0.4
                } else {
                    0.45
                },
                unclip_ratio: 1.4,
                max_candidates: 3000,
                ..TextDetectionConfig::default()
            }
        }
        _ => TextDetectionConfig::default(),
    }
}

impl OcrEngine for LocalEngine {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        if image.width() == 0 || image.height() == 0 {
            return Err(EngineError::InvalidInput("图片尺寸为 0".into()));
        }
        let start = Instant::now();
        let mut results = self
            .ocr
            .predict(vec![image.clone()])
            .map_err(|e| EngineError::Inference(e.to_string()))?;
        let page = results
            .pop()
            .ok_or_else(|| EngineError::Inference("引擎未返回结果".into()))?;
        let blocks = page
            .text_regions
            .iter()
            .filter_map(|r| {
                let text = r.text.as_deref()?.trim();
                if text.is_empty() {
                    return None;
                }
                let pts = &r.bounding_box.points;
                let quad = if pts.len() == 4 {
                    [
                        [pts[0].x, pts[0].y],
                        [pts[1].x, pts[1].y],
                        [pts[2].x, pts[2].y],
                        [pts[3].x, pts[3].y],
                    ]
                } else {
                    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                    for p in pts {
                        x0 = x0.min(p.x);
                        y0 = y0.min(p.y);
                        x1 = x1.max(p.x);
                        y1 = y1.max(p.y);
                    }
                    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
                };
                Some(TextBlock::new(text, quad, r.confidence.unwrap_or(0.0)))
            })
            .collect();
        Ok(PageResult {
            width: image.width(),
            height: image.height(),
            blocks,
            elapsed_ms: start.elapsed().as_millis() as u64,
            engine: self.info.id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_device() {
        assert_eq!("cpu".parse::<Device>().unwrap(), Device::Cpu);
        assert_eq!("directml:1".parse::<Device>().unwrap(), Device::DirectMl(1));
        assert_eq!("cuda".parse::<Device>().unwrap(), Device::Cuda(0));
        assert!("tpu".parse::<Device>().is_err());
    }

    #[test]
    fn parse_model() {
        for m in LocalModel::ALL {
            assert_eq!(m.key().parse::<LocalModel>().unwrap(), m);
        }
    }
}
