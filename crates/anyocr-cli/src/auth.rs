//! `anyocr auth`：在用户自己的终端里交互式配置在线引擎与视觉大模型的凭据。
//!
//! 密钥用隐藏输入读取，存进系统凭据管理器（与桌面应用共用），不经过智能体对话，也不写明文文件。
//! 智能体需要在线引擎时，应引导用户运行这个命令，而不是让用户把密钥发到对话里。

use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Context, Result, bail};
use anyocr_engines::credentials::{self, Source};
use anyocr_engines::online::{self, OnlineConfig};
use anyocr_engines::vlm;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum AuthCmd {
    /// 列出全部引擎及凭据配置情况（只显示末 4 位）
    List,
    /// 配置某个引擎，如 `anyocr auth set baidu`、`anyocr auth set vlm_deepseek`
    Set {
        /// 引擎 id，见 `anyocr auth list`
        engine: String,
    },
    /// 删除某个引擎保存的全部凭据与参数
    Remove { engine: String },
    /// 用内置样张测试某个引擎能否正常调用（消耗一次调用额度）
    Test { engine: String },
}

/// 一个输入项。
struct Field {
    key: String,
    label: String,
    secret: bool,
    default: String,
    hint: String,
    options: Vec<String>,
}

/// 可配置的目标：在线引擎或视觉大模型。
struct Target {
    id: String,
    name: String,
    key_url: String,
    note: String,
    fields: Vec<Field>,
}

fn targets() -> Vec<Target> {
    let mut out = Vec::new();
    let mut presets = online::presets();
    presets.push(online::mineru_preset());
    for p in presets {
        out.push(Target {
            id: p.info.id.clone(),
            name: p.info.name.clone(),
            key_url: p.info.key_url.clone().unwrap_or_default(),
            note: p.note.to_owned(),
            fields: p
                .fields
                .iter()
                .map(|f| Field {
                    key: f.key.to_owned(),
                    label: f.label.to_owned(),
                    secret: f.secret,
                    default: f.default.to_owned(),
                    hint: f.hint.to_owned(),
                    options: f.options.iter().map(|s| (*s).to_owned()).collect(),
                })
                .collect(),
        });
    }
    for p in vlm::presets() {
        let mut fields = Vec::new();
        if p.needs_key {
            fields.push(Field {
                key: "api_key".into(),
                label: "API Key".into(),
                secret: true,
                default: String::new(),
                hint: String::new(),
                options: vec![],
            });
        }
        fields.push(Field {
            key: "model".into(),
            label: "模型".into(),
            secret: false,
            default: p.model.to_owned(),
            hint: "需支持图片输入".into(),
            options: p.models.iter().map(|s| (*s).to_owned()).collect(),
        });
        fields.push(Field {
            key: "base_url".into(),
            label: "服务地址".into(),
            secret: false,
            default: p.base_url.to_owned(),
            hint: "OpenAI 兼容地址".into(),
            options: vec![],
        });
        out.push(Target {
            id: format!("vlm_{}", p.id),
            name: format!("视觉大模型：{}", p.name),
            key_url: p.key_url.to_owned(),
            note: p.note.to_owned(),
            fields,
        });
    }
    out
}

fn find(id: &str) -> Result<Target> {
    targets()
        .into_iter()
        .find(|t| t.id == id)
        .with_context(|| format!("未知引擎 {id}，用 `anyocr auth list` 查看可用的 id"))
}

/// 凭据是否齐全（必填的密钥字段都有值），以及显示用的状态文字。
fn state(t: &Target) -> (bool, String) {
    let mut parts = Vec::new();
    let mut ready = true;
    for f in t.fields.iter().filter(|f| f.secret) {
        match credentials::lookup(&t.id, &f.key) {
            Some((v, Source::Env)) => parts.push(format!(
                "{} …{}（环境变量）",
                f.label,
                credentials::tail(&v)
            )),
            Some((v, Source::Keyring)) => {
                parts.push(format!("{} …{}", f.label, credentials::tail(&v)))
            }
            None => {
                ready = false;
                parts.push(format!("{} 未配置", f.label));
            }
        }
    }
    if t.fields.iter().all(|f| !f.secret) {
        parts.push("无需密钥".into());
    }
    (ready, parts.join("，"))
}

fn print_list() {
    println!("凭据保存在系统凭据管理器，与 AnyOCR 桌面应用共用。\n");
    for t in targets() {
        let (ready, s) = state(&t);
        println!(
            "{} {:<16} {}  {}",
            if ready { "✓" } else { "·" },
            t.id,
            t.name,
            s
        );
    }
    println!("\n配置：anyocr auth set <id>（或直接运行 anyocr auth 按提示选择）");
}

fn read_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s)?;
    Ok(s.trim().to_owned())
}

fn ensure_terminal() -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!(
            "这个命令需要在你自己的终端里交互运行（密钥用隐藏输入，不经过其他程序）。\n\
             请打开终端执行：npx -y anyocr auth"
        );
    }
    Ok(())
}

fn configure(t: &Target) -> Result<()> {
    println!("\n配置 {}（{}）", t.name, t.id);
    if !t.note.is_empty() {
        println!("  {}", t.note);
    }
    if !t.key_url.is_empty() {
        println!("  获取 Key：{}", t.key_url);
    }
    println!("  直接回车保留当前值，输入 - 清除。密钥输入时不显示。\n");
    for f in &t.fields {
        let current = credentials::get(&t.id, &f.key);
        let hint = if f.hint.is_empty() {
            String::new()
        } else {
            format!("（{}）", f.hint)
        };
        let value = if f.secret {
            let shown = current
                .as_deref()
                .map(|v| format!("已保存 …{}", credentials::tail(v)))
                .unwrap_or_else(|| "未保存".into());
            rpassword::prompt_password(format!("  {}{hint} [{shown}]：", f.label))
                .context("读取输入失败")?
        } else {
            let shown = current.clone().unwrap_or_else(|| f.default.clone());
            if !f.options.is_empty() {
                println!("  可选：{}", f.options.join(" / "));
            }
            read_line(&format!(
                "  {}{hint} [{}]：",
                f.label,
                if shown.is_empty() { "无" } else { &shown }
            ))?
        };
        let value = value.trim();
        if value == "-" {
            credentials::set(&t.id, &f.key, "").map_err(anyhow::Error::msg)?;
        } else if !value.is_empty() {
            credentials::set(&t.id, &f.key, value).map_err(anyhow::Error::msg)?;
        }
    }
    let (ready, s) = state(t);
    println!(
        "\n{} {s}",
        if ready {
            "已保存。"
        } else {
            "尚未配置完整："
        }
    );
    if ready {
        let ans = read_line("现在测试连接吗？会消耗一次调用额度 [Y/n]：")?;
        if !ans.eq_ignore_ascii_case("n") {
            test(t)?;
        }
    }
    Ok(())
}

fn wizard() -> Result<()> {
    ensure_terminal()?;
    let all = targets();
    println!("选择要配置的引擎（凭据保存在系统凭据管理器，与桌面应用共用）：\n");
    for (i, t) in all.iter().enumerate() {
        let (ready, _) = state(t);
        println!(
            "  {:>2}. {} {:<16} {}",
            i + 1,
            if ready { "✓" } else { " " },
            t.id,
            t.name
        );
    }
    let pick = read_line("\n输入序号或 id（回车退出）：")?;
    if pick.is_empty() {
        return Ok(());
    }
    let t = match pick.parse::<usize>() {
        Ok(n) if (1..=all.len()).contains(&n) => all.into_iter().nth(n - 1),
        _ => all.into_iter().find(|t| t.id == pick),
    }
    .with_context(|| format!("没有找到 {pick}"))?;
    configure(&t)
}

const SAMPLE: &[u8] = include_bytes!("../../../tests/fixtures/hello.png");

fn test(t: &Target) -> Result<()> {
    let img = image::load_from_memory(SAMPLE)?.to_rgb8();
    let start = std::time::Instant::now();
    let r: Result<String, String> = if let Some(provider) = t.id.strip_prefix("vlm_") {
        crate::vlm_client(provider, "", "")
            .map_err(|e| format!("{e:#}"))
            .and_then(|c| {
                let page = anyocr_core::PageResult {
                    width: img.width(),
                    height: img.height(),
                    blocks: vec![anyocr_core::TextBlock::from_rect(
                        "识别任何文宇 2026",
                        [30., 150., 360., 200.],
                        0.7,
                    )],
                    ..Default::default()
                };
                use anyocr_core::Reviewer;
                vlm::VlmReviewer::new(c, anyocr_core::Guard::default())
                    .review(&img, &page)
                    .map(|o| format!("模型改正了 {} 处故意写错的字", o.accepted()))
                    .map_err(|e| e.to_string())
            })
    } else if t.id == "mineru" {
        online::mineru::check_token(&credentials::online_config(&online::mineru_preset()))
            .map(|_| "令牌有效".to_owned())
            .map_err(|e| e.to_string())
    } else {
        let preset = online::presets()
            .into_iter()
            .find(|p| p.info.id == t.id)
            .context("未知引擎")?;
        let cfg: OnlineConfig = credentials::online_config(&preset);
        online::build(&t.id, cfg)
            .and_then(|e| e.recognize(&img))
            .map(|p| p.text().split_whitespace().collect::<Vec<_>>().join(" "))
            .map_err(|e| e.to_string())
    };
    match r {
        Ok(s) => println!("✓ 连接正常（{} ms）：{s}", start.elapsed().as_millis()),
        Err(e) => println!("✗ 测试失败：{e}"),
    }
    Ok(())
}

pub fn run(cmd: Option<AuthCmd>) -> Result<()> {
    match cmd {
        None => wizard(),
        Some(AuthCmd::List) => {
            print_list();
            Ok(())
        }
        Some(AuthCmd::Set { engine }) => {
            ensure_terminal()?;
            configure(&find(&engine)?)
        }
        Some(AuthCmd::Remove { engine }) => {
            let t = find(&engine)?;
            for f in &t.fields {
                credentials::set(&t.id, &f.key, "").map_err(anyhow::Error::msg)?;
            }
            println!("已删除 {} 的全部凭据与参数。", t.id);
            Ok(())
        }
        Some(AuthCmd::Test { engine }) => test(&find(&engine)?),
    }
}

/// MCP `engine_status` 使用：各引擎是否已配置（不含任何密钥内容）。
pub fn status_report() -> String {
    let mut out = String::from("本地引擎（local）：始终可用，无需配置。\n\n");
    for t in targets() {
        let (ready, _) = state(&t);
        let s = if t.fields.iter().all(|f| !f.secret) {
            "无需密钥（本机服务或自定义地址，可用 anyocr auth set 设置模型与地址）"
        } else if ready {
            "已配置"
        } else {
            "未配置"
        };
        out.push_str(&format!("- {}（{}）：{s}\n", t.id, t.name));
    }
    out.push_str(
        "\n需要使用未配置的在线引擎时，请让用户在自己的终端运行 `npx -y anyocr auth set <id>` 完成配置\
（密钥隐藏输入，存入系统凭据管理器）。不要让用户把 API Key 粘贴到对话中。",
    );
    out
}
