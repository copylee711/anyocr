//! 在线引擎相关命令：预设列表、密钥读写、测试连接、MinerU 文档转 Markdown 任务。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyocr_engines::online::{self, Preset, mineru};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::engines;
use crate::jobs::{JobEvent, Jobs};
use crate::secrets;
use crate::settings::SettingsState;

/// 测试连接用的样张（英文 + 中文）。
const SAMPLE: &[u8] = include_bytes!("../../tests/fixtures/hello.png");

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Presets {
    engines: Vec<Preset>,
    mineru: Preset,
}

#[tauri::command]
pub fn online_presets() -> Presets {
    Presets {
        engines: online::presets(),
        mineru: online::mineru_preset(),
    }
}

fn find_preset(id: &str) -> Result<Preset, String> {
    let mut all = online::presets();
    all.push(online::mineru_preset());
    all.into_iter()
        .find(|p| p.info.id == id)
        .ok_or_else(|| format!("未知的引擎：{id}"))
}

/// 各密钥字段的保存情况：字段 → 末 4 位（未保存为 null）。
#[tauri::command]
pub async fn secret_status(engine: String) -> Result<BTreeMap<String, Option<String>>, String> {
    let preset = find_preset(&engine)?;
    tauri::async_runtime::spawn_blocking(move || {
        let fields: Vec<&str> = preset
            .fields
            .iter()
            .filter(|f| f.secret)
            .map(|f| f.key)
            .collect();
        secrets::status(&engine, &fields)
    })
    .await
    .map_err(|e| e.to_string())
}

/// 保存或清除（空值）某个密钥字段。
#[tauri::command]
pub async fn secret_set(engine: String, field: String, value: String) -> Result<(), String> {
    let preset = find_preset(&engine)?;
    if !preset.fields.iter().any(|f| f.secret && f.key == field) {
        return Err(format!("{engine} 没有密钥字段 {field}"));
    }
    tauri::async_runtime::spawn_blocking(move || secrets::set(&engine, &field, &value))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    ok: bool,
    elapsed_ms: u64,
    text: String,
    error: Option<String>,
}

/// 用内置样张调用一次引擎，验证密钥与网络。会消耗一次调用额度。
#[tauri::command]
pub async fn online_test(
    settings: State<'_, SettingsState>,
    engine: String,
) -> Result<TestResult, String> {
    let online_settings = settings.online();
    tauri::async_runtime::spawn_blocking(move || {
        let start = Instant::now();
        let sample = image::load_from_memory(SAMPLE)
            .map_err(|e| e.to_string())?
            .to_rgb8();
        let r = if engine == "mineru" {
            mineru_check(&online_settings)
        } else {
            engines::build_online(&engine, &online_settings)
                .and_then(|e| e.recognize(&sample).map_err(|e| e.to_string()))
                .map(|p| p.text())
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        Ok(match r {
            Ok(text) => TestResult {
                ok: true,
                elapsed_ms,
                text,
                error: None,
            },
            Err(e) => TestResult {
                ok: false,
                elapsed_ms,
                text: String::new(),
                error: Some(e),
            },
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// MinerU 解析一份文档要几十秒，测试连接只验证令牌：申请上传地址（不上传文件）。
fn mineru_check(online_settings: &crate::settings::OnlineSettings) -> Result<String, String> {
    let preset = online::mineru_preset();
    let cfg = engines::online_config(&preset, online_settings);
    mineru::check_token(&cfg).map_err(|e| e.to_string())?;
    Ok("令牌有效".into())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MineruJob {
    pub files: Vec<String>,
    #[serde(default)]
    pub pages: String,
    /// 为空时输出到源文件所在目录。
    #[serde(default)]
    pub out_dir: String,
}

/// 启动 MinerU 转 Markdown 任务，逐个文件串行处理（服务端排队，本地并发意义不大）。
#[tauri::command]
pub fn mineru_start(
    app: AppHandle,
    settings: State<'_, SettingsState>,
    jobs_state: State<'_, Arc<Jobs>>,
    args: MineruJob,
) -> Result<u64, String> {
    let preset = online::mineru_preset();
    let config = engines::online_config(&preset, &settings.online());
    if config.get("token").is_none() {
        return Err("请先在设置 → 在线引擎中填写 MinerU 的 API Token".into());
    }
    let jobs_state = Arc::clone(&jobs_state);
    let (job, cancel) = jobs_state.register();
    std::thread::spawn(move || {
        let emit = |e: JobEvent| {
            let _ = app.emit("job", e);
        };
        let mut outputs = Vec::new();
        for (i, f) in args.files.iter().enumerate() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            emit(JobEvent::FileStart { job, file: i });
            let input = Path::new(f);
            let out = if args.out_dir.trim().is_empty() {
                input.parent().map(Path::to_path_buf).unwrap_or_default()
            } else {
                PathBuf::from(args.out_dir.trim())
            };
            let start = Instant::now();
            let opts = mineru::Options {
                config: config.clone(),
                page_ranges: Some(args.pages.clone()),
                deadline_secs: 0,
            };
            let r = mineru::convert(input, &out, &opts, &cancel, |p| {
                let (stage, done, total) = match p {
                    mineru::Progress::Uploading => ("uploading".to_owned(), 0, 0),
                    mineru::Progress::Running {
                        state,
                        extracted,
                        total,
                    } => (state, extracted as usize, total as usize),
                    mineru::Progress::Downloading => ("downloading".to_owned(), 0, 0),
                };
                emit(JobEvent::Progress {
                    job,
                    file: i,
                    stage,
                    done,
                    total,
                });
            });
            let elapsed_ms = start.elapsed().as_millis() as u64;
            match r {
                Ok(o) => {
                    let md = o.markdown.display().to_string();
                    outputs.push(md.clone());
                    emit(JobEvent::FileDone {
                        job,
                        file: i,
                        ok: true,
                        error: None,
                        elapsed_ms,
                        outputs: vec![md],
                        report: None,
                        text: None,
                        blocks: 0,
                    });
                }
                Err(e) => emit(JobEvent::FileDone {
                    job,
                    file: i,
                    ok: false,
                    error: Some(e.to_string()),
                    elapsed_ms,
                    outputs: vec![],
                    report: None,
                    text: None,
                    blocks: 0,
                }),
            }
        }
        emit(JobEvent::JobDone {
            job,
            cancelled: cancel.load(std::sync::atomic::Ordering::Relaxed),
            outputs,
            error: None,
        });
        jobs_state.finish(job);
    });
    Ok(job)
}
