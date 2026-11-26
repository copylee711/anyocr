//! 在线引擎与视觉大模型的凭据：保存在系统凭据管理器（Windows 凭据管理器 / macOS 钥匙串）。
//!
//! 桌面应用和命令行共用同一套条目（服务名 `anyocr`，用户名 `<引擎 id>/<字段>`，视觉大模型为
//! `vlm_<服务>/api_key`），在任一处配置后另一处都能直接使用。
//!
//! 命令行取值顺序：环境变量 → 凭据管理器。环境变量便于 CI 与临时覆盖。

use std::collections::BTreeMap;

use crate::online::{OnlineConfig, Preset};

const SERVICE: &str = "anyocr";

fn entry(engine: &str, field: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("{engine}/{field}")).map_err(|e| e.to_string())
}

/// 读取；不存在时返回 `None`。
pub fn get(engine: &str, field: &str) -> Option<String> {
    entry(engine, field)
        .ok()?
        .get_password()
        .ok()
        .filter(|s| !s.is_empty())
}

/// 保存；值为空时删除。
pub fn set(engine: &str, field: &str, value: &str) -> Result<(), String> {
    let e = entry(engine, field)?;
    let value = value.trim();
    if value.is_empty() {
        return match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        };
    }
    e.set_password(value).map_err(|e| e.to_string())
}

/// 末 4 位，便于用户确认是哪把 Key，又不暴露完整内容。
pub fn tail(v: &str) -> String {
    let chars: Vec<char> = v.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

/// 各字段是否已保存：字段 → 末 4 位（未保存为 `None`）。
pub fn status(engine: &str, fields: &[&str]) -> BTreeMap<String, Option<String>> {
    fields
        .iter()
        .map(|f| ((*f).to_owned(), get(engine, f).map(|v| tail(&v))))
        .collect()
}

/// 某个字段对应的环境变量名：`ANYOCR_<ID>_<FIELD>`。
pub fn env_name(engine: &str, field: &str) -> String {
    format!("ANYOCR_{}_{}", engine.to_uppercase(), field.to_uppercase())
}

fn legacy_env(engine: &str, field: &str) -> Option<&'static str> {
    match (engine, field) {
        ("paddle_aistudio", "token") => Some("ANYOCR_PADDLE_TOKEN"),
        ("mineru", "token") => Some("ANYOCR_MINERU_TOKEN"),
        ("vlm_deepseek", "api_key") => Some("DEEPSEEK_API_KEY"),
        ("vlm_openai", "api_key") => Some("OPENAI_API_KEY"),
        ("vlm_gemini", "api_key") => Some("GEMINI_API_KEY"),
        ("vlm_qwen", "api_key") => Some("DASHSCOPE_API_KEY"),
        _ => None,
    }
}

/// 读取一个字段：环境变量优先，其次凭据管理器。返回值与来源。
pub fn lookup(engine: &str, field: &str) -> Option<(String, Source)> {
    let from_env = std::env::var(env_name(engine, field))
        .ok()
        .or_else(|| legacy_env(engine, field).and_then(|n| std::env::var(n).ok()))
        .filter(|v| !v.trim().is_empty());
    if let Some(v) = from_env {
        return Some((v, Source::Env));
    }
    get(engine, field).map(|v| (v, Source::Keyring))
}

/// 凭据来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Keyring,
}

/// 命令行与 MCP 使用的在线引擎配置：每个字段按“环境变量 → 凭据管理器”取值。
pub fn online_config(preset: &Preset) -> OnlineConfig {
    let mut fields = BTreeMap::new();
    for f in &preset.fields {
        if let Some((v, _)) = lookup(&preset.info.id, f.key) {
            fields.insert(f.key.to_owned(), v);
        }
    }
    OnlineConfig {
        fields,
        timeout_secs: 0,
    }
}

/// 视觉大模型的 API Key（凭据条目 `vlm_<服务>/api_key`）。
pub fn vlm_key(provider: &str) -> Option<String> {
    lookup(&format!("vlm_{provider}"), "api_key").map(|(v, _)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(env_name("baidu", "api_key"), "ANYOCR_BAIDU_API_KEY");
        assert_eq!(
            env_name("vlm_deepseek", "api_key"),
            "ANYOCR_VLM_DEEPSEEK_API_KEY"
        );
        assert_eq!(tail("abcdef1234"), "1234");
        assert_eq!(tail("ab"), "ab");
    }
}
