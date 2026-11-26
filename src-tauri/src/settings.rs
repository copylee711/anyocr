//! 应用设置：引擎相关字段由 Rust 解析；其余字段（界面、各页默认值等）原样保存，交给前端管理。
//! 存储位置：Tauri app_config_dir/settings.json。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use anyocr_engines::{Device, LocalModel};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EngineSettings {
    /// 本地模型，如 `pp-ocrv6-small`。
    pub model: String,
    /// `auto` / `cpu` / `directml` / `cuda`。
    pub device: String,
    /// 并行识别实例数；0 为自动（GPU 1，CPU 2）。
    pub workers: usize,
    pub textline_orientation: bool,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            model: "pp-ocrv6-small".into(),
            device: "auto".into(),
            workers: 0,
            textline_orientation: false,
        }
    }
}

impl EngineSettings {
    pub fn model(&self) -> LocalModel {
        self.model.parse().unwrap_or_default()
    }
    pub fn device(&self) -> Device {
        self.device.parse().unwrap_or_default()
    }
    pub fn worker_count(&self) -> usize {
        match self.workers {
            0 if self.device() == Device::Cpu => 2,
            0 => 1,
            n => n.min(8),
        }
    }
}

/// 在线引擎设置（密钥不在这里，见 `secrets`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OnlineSettings {
    /// 引擎优先级链，`local` 为本地引擎，其余为在线引擎 id。
    pub chain: Vec<String>,
    /// 单页识别超时（秒）。
    pub timeout: u64,
    /// 并行请求数（链中没有本地引擎时使用）。
    pub concurrency: usize,
    /// 各引擎的非密钥字段，如模型、地域、终结点。
    pub engines: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for OnlineSettings {
    fn default() -> Self {
        Self {
            chain: vec!["local".into()],
            timeout: 60,
            concurrency: 4,
            engines: BTreeMap::new(),
        }
    }
}

/// 视觉大模型设置（密钥在凭据管理器，条目 `vlm_<服务>/api_key`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VlmSettings {
    /// 服务预设 id，如 `deepseek`。
    pub provider: String,
    /// 为空时用预设默认模型。
    pub model: String,
    /// 为空时用预设地址（自定义 / Ollama 需要填写或修改）。
    pub base_url: String,
    pub timeout: u64,
    pub review: ReviewSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReviewSettings {
    /// 文档与批量任务开启并行复核。
    pub enabled: bool,
    /// 截图识别后复核，结果就地更新。
    pub screenshot: bool,
    pub concurrency: usize,
    /// OCR 完成后最多再等的秒数。
    pub grace: f32,
    /// 大于 0 时只复核含低于该置信度的行的页。
    pub low_conf: f32,
}

impl Default for VlmSettings {
    fn default() -> Self {
        Self {
            provider: "deepseek".into(),
            model: String::new(),
            base_url: String::new(),
            timeout: 60,
            review: ReviewSettings::default(),
        }
    }
}

impl Default for ReviewSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            screenshot: false,
            concurrency: 4,
            grace: 10.0,
            low_conf: 0.0,
        }
    }
}

pub struct SettingsState {
    path: PathBuf,
    value: Mutex<Value>,
}

impl SettingsState {
    pub fn load(path: PathBuf) -> Self {
        let value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Map::new()));
        Self {
            path,
            value: Mutex::new(value),
        }
    }

    pub fn get(&self) -> Value {
        self.value.lock().map(|v| v.clone()).unwrap_or(Value::Null)
    }

    pub fn engine(&self) -> EngineSettings {
        self.get()
            .get("engine")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    pub fn online(&self) -> OnlineSettings {
        self.get()
            .get("online")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    pub fn vlm(&self) -> VlmSettings {
        self.get()
            .get("vlm")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, value: Value) -> Result<(), String> {
        if !value.is_object() {
            return Err("设置必须是 JSON 对象".into());
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        if let Ok(mut v) = self.value.lock() {
            *v = value;
        }
        Ok(())
    }
}
