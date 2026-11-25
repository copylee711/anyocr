//! 引擎池：按当前引擎设置懒加载并复用本地引擎实例；设置变化时重建。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyocr_core::OcrEngine;
use anyocr_engines::{LocalConfig, LocalEngine};

use crate::settings::EngineSettings;

struct Pool {
    key: Option<EngineSettings>,
    engines: Vec<Arc<LocalEngine>>,
}

pub struct EngineState {
    models_dir: PathBuf,
    pool: Mutex<Pool>,
}

impl EngineState {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            models_dir,
            pool: Mutex::new(Pool {
                key: None,
                engines: Vec::new(),
            }),
        }
    }

    /// 取得 `n` 个引擎实例（不足时加载；首次使用可能需要下载模型，耗时较长）。
    pub fn engines(
        &self,
        settings: &EngineSettings,
        n: usize,
    ) -> Result<Vec<Arc<dyn OcrEngine>>, String> {
        let mut pool = self.pool.lock().map_err(|_| "引擎状态已损坏".to_string())?;
        if pool.key.as_ref() != Some(settings) {
            pool.engines.clear();
            pool.key = Some(settings.clone());
        }
        while pool.engines.len() < n.max(1) {
            let engine = LocalEngine::new(LocalConfig {
                model: settings.model(),
                device: settings.device(),
                models_dir: Some(self.models_dir.clone()),
                textline_orientation: settings.textline_orientation,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
            pool.engines.push(Arc::new(engine));
        }
        Ok(pool
            .engines
            .iter()
            .take(n.max(1))
            .map(|e| Arc::clone(e) as Arc<dyn OcrEngine>)
            .collect())
    }

    pub fn models_dir(&self) -> &PathBuf {
        &self.models_dir
    }
}
