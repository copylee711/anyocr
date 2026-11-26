//! 引擎池：按当前引擎设置懒加载并复用本地引擎实例；设置变化时重建。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyocr_core::Guard;
use anyocr_core::OcrEngine;
use anyocr_core::review::ReviewSetup;
use anyocr_engines::online::{self, OnlineConfig, Preset};
use anyocr_engines::vlm::{self, VlmClient, VlmConfig, VlmEngine, VlmReviewer};
use anyocr_engines::{ChainEngine, LocalConfig, LocalEngine};

use crate::secrets;
use crate::settings::{EngineSettings, OnlineSettings, VlmSettings};

/// 凭据管理器中 VLM 密钥的引擎名。
pub fn vlm_secret_id(provider: &str) -> String {
    format!("vlm_{provider}")
}

/// 按设置构造 VLM 客户端。
pub fn vlm_client(v: &VlmSettings) -> Result<VlmClient, String> {
    let preset = vlm::presets()
        .into_iter()
        .find(|p| p.id == v.provider)
        .ok_or_else(|| format!("未知的视觉大模型服务：{}", v.provider))?;
    let key = secrets::get(&vlm_secret_id(&v.provider), "api_key");
    if preset.needs_key && key.is_none() {
        return Err(format!("请先在设置中填写 {} 的 API Key", preset.name));
    }
    let mut config = VlmConfig::from_preset(&preset, key, &v.model, &v.base_url);
    config.timeout_secs = v.timeout.max(5);
    VlmClient::new(config).map_err(|e| e.to_string())
}

/// 文档任务的复核设置；未开启时为 `None`。
pub fn review_setup(v: &VlmSettings) -> Result<Option<ReviewSetup>, String> {
    if !v.review.enabled {
        return Ok(None);
    }
    Ok(Some(ReviewSetup {
        reviewer: Arc::new(VlmReviewer::new(vlm_client(v)?, Guard::default())),
        concurrency: v.review.concurrency.clamp(1, 16),
        grace: std::time::Duration::from_secs_f32(v.review.grace.clamp(0.0, 600.0)),
        low_conf: (v.review.low_conf > 0.0).then_some(v.review.low_conf),
    }))
}

/// 在线引擎配置：设置中的非密钥字段 + 凭据管理器中的密钥。
pub fn online_config(preset: &Preset, online: &OnlineSettings) -> OnlineConfig {
    let mut fields = online
        .engines
        .get(&preset.info.id)
        .cloned()
        .unwrap_or_default();
    for f in preset.fields.iter().filter(|f| f.secret) {
        if let Some(v) = secrets::get(&preset.info.id, f.key) {
            fields.insert(f.key.to_owned(), v);
        }
    }
    OnlineConfig {
        fields,
        timeout_secs: online.timeout,
    }
}

/// 构造单个在线引擎。
pub fn build_online(id: &str, online: &OnlineSettings) -> Result<Arc<dyn OcrEngine>, String> {
    let preset = online::presets()
        .into_iter()
        .find(|p| p.info.id == id)
        .ok_or_else(|| format!("未知的在线引擎：{id}"))?;
    online::build(id, online_config(&preset, online)).map_err(|e| e.to_string())
}

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

    /// 按设置的引擎链构造任务用的引擎，返回 (引擎列表, 链说明)。
    /// `n` 为空时自动决定并行数：链中有本地引擎时按本地设置，否则按在线并发数。
    /// 没有密钥等无法构造的在线引擎直接跳过；全部不可用时报错。
    pub fn task_engines(
        &self,
        es: &EngineSettings,
        online: &OnlineSettings,
        vlm_settings: &VlmSettings,
        n: Option<usize>,
    ) -> Result<Vec<Arc<dyn OcrEngine>>, String> {
        let chain: Vec<&str> = if online.chain.is_empty() {
            vec!["local"]
        } else {
            online.chain.iter().map(String::as_str).collect()
        };
        // “vlm”（视觉大模型识别）也需要本地引擎提供行框
        let has_local = chain.contains(&"local") || chain.contains(&"vlm");
        let n = n
            .unwrap_or(if has_local {
                es.worker_count()
            } else {
                online.concurrency.clamp(1, 16)
            })
            .max(1);
        let locals = if has_local {
            self.engines(es, n)?
        } else {
            Vec::new()
        };
        let mut shared: Vec<Option<Arc<dyn OcrEngine>>> = Vec::new();
        let mut errors = Vec::new();
        // 链中的“vlm”：每个并行实例一个 VlmEngine，行框来自同序号的本地引擎
        let mut vlm_client_ok: Option<Result<(), String>> = None;
        for id in &chain {
            if *id == "local" {
                shared.push(None);
                continue;
            }
            if *id == "vlm" {
                let r = vlm_client(vlm_settings).map(|_| ());
                if let Err(e) = &r {
                    errors.push(format!("视觉大模型：{e}"));
                } else {
                    shared.push(None);
                }
                vlm_client_ok = Some(r);
                continue;
            }
            match build_online(id, online) {
                Ok(e) => shared.push(Some(e)),
                Err(e) => {
                    tracing::warn!(engine = id, error = %e, "在线引擎不可用，已跳过");
                    errors.push(format!("{id}：{e}"));
                }
            }
        }
        if shared.is_empty() {
            return Err(format!("没有可用的引擎（{}）", errors.join("；")));
        }
        let ids: Vec<&str> = chain
            .iter()
            .copied()
            .filter(|id| *id != "vlm" || matches!(vlm_client_ok, Some(Ok(()))))
            .filter(|id| {
                *id == "local"
                    || *id == "vlm"
                    || !errors.iter().any(|e| e.starts_with(&format!("{id}：")))
            })
            .collect();
        (0..n)
            .map(|i| {
                let list = shared
                    .iter()
                    .zip(&ids)
                    .map(|(s, id)| -> Result<Arc<dyn OcrEngine>, String> {
                        Ok(match s {
                            Some(e) => Arc::clone(e),
                            None if *id == "vlm" => {
                                let client = vlm_client(vlm_settings)?;
                                let name = format!("视觉大模型 {}", client.model());
                                Arc::new(VlmEngine::new(
                                    "vlm",
                                    &name,
                                    client,
                                    Some(Arc::clone(&locals[i])),
                                ))
                            }
                            None => Arc::clone(&locals[i]),
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(if list.len() == 1 {
                    Arc::clone(&list[0])
                } else {
                    Arc::new(ChainEngine::new(list)) as Arc<dyn OcrEngine>
                })
            })
            .collect()
    }

    pub fn models_dir(&self) -> &PathBuf {
        &self.models_dir
    }
}
