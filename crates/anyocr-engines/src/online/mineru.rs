//! MinerU 精准解析 API v4：整份文档（PDF / 图片 / Office）转 Markdown，公式输出 LaTeX，表格输出 HTML。
//!
//! 流程：`POST /api/v4/file-urls/batch` 申请上传地址 → `PUT` 上传文件（不带 Content-Type）→
//! 轮询 `GET /api/v4/extract-results/batch/{batch_id}` → 下载结果 zip，解出 `full.md` 与 `images/`。

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyocr_core::EngineError;
use serde_json::{Value, json};

use super::*;

const BASE: &str = "https://mineru.net/api/v4";

pub fn preset() -> Preset {
    Preset {
        info: info(
            "mineru",
            "MinerU 文档解析",
            "https://mineru.net/apiManage",
            "https://mineru.net/",
            false,
        ),
        fields: vec![
            Field::secret("token", "API Token"),
            Field::text("model_version", "模型", "vlm")
                .options(&["vlm", "pipeline"])
                .hint("vlm 精度更高；pipeline 速度更快"),
            Field::text("language", "文档语言", "ch").hint("pipeline 模型使用，如 ch、en"),
        ],
        note: "整份文档转 Markdown（公式为 LaTeX，表格为 HTML，图片单独导出）。单文件 ≤ 200MB / 600 页，每天 2000 页高优先级额度。",
        verified: true,
    }
}

/// MinerU 任务进度。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Uploading,
    /// 排队或解析中：状态、已解析页数、总页数（未知时为 0）。
    Running {
        state: String,
        extracted: u32,
        total: u32,
    },
    Downloading,
}

/// 转换选项。
#[derive(Debug, Clone)]
pub struct Options {
    pub config: OnlineConfig,
    /// 页码范围，如 `1-10,15`；为空表示全部。
    pub page_ranges: Option<String>,
    /// 等待解析完成的最长时间；为 0 时 30 分钟。
    pub deadline_secs: u64,
}

/// 转换结果。
#[derive(Debug, Clone)]
pub struct Output {
    /// 生成的 Markdown 文件。
    pub markdown: PathBuf,
    /// Markdown 与图片所在目录。
    pub dir: PathBuf,
}

fn api_error(v: &Value) -> Result<(), EngineError> {
    let code = &v["code"];
    if code.as_i64() == Some(0) {
        return Ok(());
    }
    let c = match code {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let msg = format!("{c} {}", v["msg"].as_str().unwrap_or_default());
    Err(match c.as_str() {
        "A0202" | "A0211" | "401" | "403" => EngineError::Unauthorized(msg),
        "-60018" | "-60019" => EngineError::QuotaExceeded(msg),
        "-60002" | "-60005" | "-60006" | "-60007" => EngineError::InvalidInput(msg),
        _ => EngineError::Inference(msg),
    })
}

/// 验证令牌：申请一个上传地址（不上传文件，不消耗解析额度）。
pub fn check_token(config: &OnlineConfig) -> Result<(), EngineError> {
    let token = config.require("token", "API Token")?;
    let v = read_json(
        agent(Duration::from_secs(30))
            .post(format!("{BASE}/file-urls/batch"))
            .header("Authorization", format!("Bearer {token}"))
            .send_json(json!({
                "files": [{ "name": "check.pdf", "data_id": "anyocr-check" }],
                "model_version": "pipeline",
            })),
    )?;
    api_error(&v)
}

/// 把 `input` 转成 Markdown，结果放在 `out_dir/<文件名>/` 下：`<文件名>.md` 与 `images/`。
pub fn convert(
    input: &Path,
    out_dir: &Path,
    opts: &Options,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(Progress),
) -> Result<Output, EngineError> {
    let token = opts.config.require("token", "API Token")?;
    let auth = format!("Bearer {token}");
    let http = agent(Duration::from_secs(120));
    let file = std::fs::read(input)
        .map_err(|e| EngineError::InvalidInput(format!("无法读取 {}：{e}", input.display())))?;
    let name = input
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document.pdf")
        .to_owned();
    let stem = input
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("document")
        .to_owned();

    on_progress(Progress::Uploading);
    let mut entry = json!({ "name": name, "data_id": "anyocr", "is_ocr": true });
    if let Some(r) = opts.page_ranges.as_deref().filter(|r| !r.trim().is_empty()) {
        entry["page_ranges"] = json!(r.trim());
    }
    let mut req = json!({
        "files": [entry],
        "model_version": opts.config.get("model_version").unwrap_or("vlm"),
        "enable_formula": true,
        "enable_table": true,
    });
    if let Some(lang) = opts.config.get("language") {
        req["language"] = json!(lang);
    }
    let v = read_json(
        http.post(format!("{BASE}/file-urls/batch"))
            .header("Authorization", &auth)
            .send_json(req),
    )?;
    api_error(&v)?;
    let batch = v
        .pointer("/data/batch_id")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::Inference("没有返回 batch_id".into()))?
        .to_owned();
    let upload = v
        .pointer("/data/file_urls/0")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::Inference("没有返回上传地址".into()))?
        .to_owned();
    let (status, body) = read_text(agent(Duration::from_secs(900)).put(&upload).send(&file[..]))?;
    check_status(status, &body)?;

    let deadline = Duration::from_secs(if opts.deadline_secs == 0 {
        1800
    } else {
        opts.deadline_secs
    });
    let start = Instant::now();
    let zip_url = loop {
        for _ in 0..6 {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        if start.elapsed() > deadline {
            return Err(EngineError::Timeout(format!(
                "解析超过 {} 秒",
                deadline.as_secs()
            )));
        }
        let s = read_json(
            http.get(format!("{BASE}/extract-results/batch/{batch}"))
                .header("Authorization", &auth)
                .call(),
        )?;
        api_error(&s)?;
        let r = &s["data"]["extract_result"][0];
        let state = r["state"].as_str().unwrap_or("pending");
        match state {
            "done" => {
                break r["full_zip_url"]
                    .as_str()
                    .ok_or_else(|| EngineError::Inference("解析完成但没有结果地址".into()))?
                    .to_owned();
            }
            "failed" => {
                return Err(EngineError::Inference(
                    r["err_msg"].as_str().unwrap_or("解析失败").to_owned(),
                ));
            }
            _ => on_progress(Progress::Running {
                state: state.to_owned(),
                extracted: r
                    .pointer("/extract_progress/extracted_pages")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                total: r
                    .pointer("/extract_progress/total_pages")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
            }),
        }
    };

    on_progress(Progress::Downloading);
    let mut resp = agent(Duration::from_secs(900))
        .get(&zip_url)
        .call()
        .map_err(transport)?;
    let status = resp.status().as_u16();
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(1024 * 1024 * 1024)
        .read_to_vec()
        .map_err(transport)?;
    check_status(status, "")?;
    extract(&bytes, &out_dir.join(&stem), &stem)
}

/// 解出 `full.md`（改名为 `<stem>.md`）和 `images/`，其余中间文件丢弃。
pub(crate) fn extract(zip: &[u8], dir: &Path, stem: &str) -> Result<Output, EngineError> {
    let io = |e: std::io::Error| EngineError::Inference(format!("写入结果失败：{e}"));
    let mut archive = zip::ZipArchive::new(Cursor::new(zip))
        .map_err(|e| EngineError::Inference(format!("结果 zip 无效：{e}")))?;
    std::fs::create_dir_all(dir).map_err(io)?;
    let markdown = dir.join(format!("{stem}.md"));
    let mut found = false;
    for i in 0..archive.len() {
        let mut f = archive
            .by_index(i)
            .map_err(|e| EngineError::Inference(format!("结果 zip 无效：{e}")))?;
        let Some(rel) = f.enclosed_name() else {
            continue;
        };
        if f.is_dir() {
            continue;
        }
        let target = if rel == Path::new("full.md") {
            found = true;
            markdown.clone()
        } else if rel.starts_with("images") {
            dir.join(&rel)
        } else {
            continue;
        };
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p).map_err(io)?;
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(io)?;
        std::fs::write(&target, buf).map_err(io)?;
    }
    if !found {
        return Err(EngineError::Inference("结果中没有 full.md".into()));
    }
    Ok(Output {
        markdown,
        dir: dir.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn extracts_markdown_and_images_only() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("full.md", o).unwrap();
            z.write_all(b"# T\n![](images/a.jpg)\n").unwrap();
            z.start_file("images/a.jpg", o).unwrap();
            z.write_all(b"JPG").unwrap();
            z.start_file("layout.json", o).unwrap();
            z.write_all(b"{}").unwrap();
            z.finish().unwrap();
        }
        let tmp = std::env::temp_dir().join(format!("anyocr-mineru-{}", std::process::id()));
        let out = extract(buf.get_ref(), &tmp.join("书"), "书").unwrap();
        assert_eq!(
            std::fs::read_to_string(&out.markdown).unwrap(),
            "# T\n![](images/a.jpg)\n"
        );
        assert!(tmp.join("书/images/a.jpg").exists());
        assert!(!tmp.join("书/layout.json").exists());
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn token_error() {
        let v: Value = serde_json::from_str(r#"{"code":"A0202","msg":"token error"}"#).unwrap();
        assert!(matches!(api_error(&v), Err(EngineError::Unauthorized(_))));
        let ok: Value = serde_json::from_str(r#"{"code":0,"msg":"ok"}"#).unwrap();
        assert!(api_error(&ok).is_ok());
    }
}
