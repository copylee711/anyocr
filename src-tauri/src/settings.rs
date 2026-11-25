//! 应用设置：引擎相关字段由 Rust 解析；其余字段（界面、各页默认值等）原样保存，交给前端管理。
//! 存储位置：Tauri app_config_dir/settings.json。

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
