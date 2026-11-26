//! 引擎 fallback 链：按顺序尝试，遇到可恢复错误时切到下一个引擎。
//!
//! 未配置、鉴权失败、配额用尽的引擎在本链的剩余生命周期内直接跳过（每个任务新建一条链）。
//! 限流、超时、网络错误只影响当前这一页。

use std::sync::{Arc, Mutex};

use anyocr_core::{EngineError, EngineInfo, EngineKind, OcrEngine, PageResult};
use image::RgbImage;

pub struct ChainEngine {
    info: EngineInfo,
    engines: Vec<Arc<dyn OcrEngine>>,
    disabled: Mutex<Vec<bool>>,
}

impl ChainEngine {
    /// `engines` 不能为空。
    pub fn new(engines: Vec<Arc<dyn OcrEngine>>) -> Self {
        let names: Vec<_> = engines.iter().map(|e| e.info().name.clone()).collect();
        let info = EngineInfo {
            id: "chain".into(),
            name: names.join(" → "),
            kind: engines
                .first()
                .map(|e| e.info().kind)
                .unwrap_or(EngineKind::Local),
            key_url: None,
            homepage: None,
            has_boxes: engines.iter().all(|e| e.info().has_boxes),
        };
        let n = engines.len();
        Self {
            info,
            engines,
            disabled: Mutex::new(vec![false; n]),
        }
    }

    fn is_disabled(&self, i: usize) -> bool {
        self.disabled.lock().map(|d| d[i]).unwrap_or(false)
    }

    fn disable(&self, i: usize) {
        if let Ok(mut d) = self.disabled.lock() {
            d[i] = true;
        }
    }
}

/// 这些错误在同一任务里不会自行恢复，之后直接跳过该引擎。
fn sticky(e: &EngineError) -> bool {
    matches!(
        e,
        EngineError::NotConfigured(_)
            | EngineError::Unauthorized(_)
            | EngineError::QuotaExceeded(_)
    )
}

impl OcrEngine for ChainEngine {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn recognize(&self, image: &RgbImage) -> Result<PageResult, EngineError> {
        let mut errors = Vec::new();
        let mut last = None;
        for (i, engine) in self.engines.iter().enumerate() {
            if self.is_disabled(i) {
                continue;
            }
            match engine.recognize(image) {
                Ok(page) => return Ok(page),
                Err(e) if !e.should_fallback() => return Err(e),
                Err(e) => {
                    tracing::warn!(engine = %engine.info().id, error = %e, "引擎失败，尝试下一个");
                    if sticky(&e) {
                        self.disable(i);
                    }
                    errors.push(format!("{}：{e}", engine.info().name));
                    last = Some(e);
                }
            }
        }
        let summary = if errors.is_empty() {
            "没有可用的引擎".to_owned()
        } else {
            errors.join("；")
        };
        // 保留最后一个错误的类别，消息里列出每个引擎的失败原因
        Err(match last {
            Some(EngineError::Timeout(_)) => EngineError::Timeout(summary),
            Some(EngineError::RateLimited(_)) => EngineError::RateLimited(summary),
            Some(EngineError::QuotaExceeded(_)) => EngineError::QuotaExceeded(summary),
            Some(EngineError::Unauthorized(_)) => EngineError::Unauthorized(summary),
            Some(EngineError::Network(_)) => EngineError::Network(summary),
            Some(EngineError::Inference(_)) | Some(EngineError::Model(_)) => {
                EngineError::Inference(summary)
            }
            _ => EngineError::NotConfigured(summary),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Fake {
        info: EngineInfo,
        calls: AtomicUsize,
        result: fn() -> Result<PageResult, EngineError>,
    }

    fn fake(id: &str, result: fn() -> Result<PageResult, EngineError>) -> Arc<Fake> {
        Arc::new(Fake {
            info: EngineInfo {
                id: id.into(),
                name: id.into(),
                kind: EngineKind::Online,
                key_url: None,
                homepage: None,
                has_boxes: true,
            },
            calls: AtomicUsize::new(0),
            result,
        })
    }

    impl OcrEngine for Fake {
        fn info(&self) -> &EngineInfo {
            &self.info
        }
        fn recognize(&self, _: &RgbImage) -> Result<PageResult, EngineError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            (self.result)().map(|mut p| {
                p.engine = self.info.id.clone();
                p
            })
        }
    }

    fn ok() -> Result<PageResult, EngineError> {
        Ok(PageResult::default())
    }

    #[test]
    fn falls_back_and_skips_sticky_failures() {
        let a = fake("a", || Err(EngineError::QuotaExceeded("q".into())));
        let b = fake("b", || Err(EngineError::Timeout("t".into())));
        let c = fake("c", ok);
        let chain = ChainEngine::new(vec![a.clone(), b.clone(), c.clone()]);
        let img = RgbImage::new(2, 2);
        assert_eq!(chain.recognize(&img).unwrap().engine, "c");
        assert_eq!(chain.recognize(&img).unwrap().engine, "c");
        // 配额用尽的 a 只调用一次；超时的 b 每页都会重试
        assert_eq!(a.calls.load(Ordering::SeqCst), 1);
        assert_eq!(b.calls.load(Ordering::SeqCst), 2);
        assert_eq!(chain.info().name, "a → b → c");
    }

    #[test]
    fn invalid_input_stops_chain() {
        let a = fake("a", || Err(EngineError::InvalidInput("bad".into())));
        let b = fake("b", ok);
        let chain = ChainEngine::new(vec![a, b.clone()]);
        assert!(matches!(
            chain.recognize(&RgbImage::new(2, 2)),
            Err(EngineError::InvalidInput(_))
        ));
        assert_eq!(b.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn all_failed_lists_reasons() {
        let a = fake("a", || Err(EngineError::Unauthorized("401".into())));
        let b = fake("b", || Err(EngineError::Network("down".into())));
        let chain = ChainEngine::new(vec![a, b]);
        let e = chain
            .recognize(&RgbImage::new(2, 2))
            .unwrap_err()
            .to_string();
        assert!(e.contains("a：") && e.contains("b："), "{e}");
    }
}
