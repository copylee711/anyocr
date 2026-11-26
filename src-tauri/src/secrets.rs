//! 在线引擎的密钥保存在系统凭据管理器（Windows 凭据管理器 / macOS 钥匙串），不写进设置文件。
//! 条目：服务名 `anyocr`，用户名 `<引擎 id>/<字段>`。

use std::collections::BTreeMap;

const SERVICE: &str = "anyocr";

fn entry(engine: &str, field: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("{engine}/{field}")).map_err(|e| e.to_string())
}

/// 读取密钥；不存在时返回 `None`。
pub fn get(engine: &str, field: &str) -> Option<String> {
    entry(engine, field)
        .ok()?
        .get_password()
        .ok()
        .filter(|s| !s.is_empty())
}

/// 保存密钥；值为空时删除。
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

/// 各密钥字段是否已保存，以及末 4 位（便于用户确认是哪把 Key）。
pub fn status(engine: &str, fields: &[&str]) -> BTreeMap<String, Option<String>> {
    fields
        .iter()
        .map(|f| {
            let tail = get(engine, f).map(|v| {
                let chars: Vec<char> = v.chars().collect();
                chars[chars.len().saturating_sub(4)..].iter().collect()
            });
            ((*f).to_owned(), tail)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// 读写真实的系统凭据管理器，默认不运行。
    #[test]
    #[ignore]
    fn roundtrip() {
        super::set("anyocr_selftest", "api_key", "abcd1234").unwrap();
        assert_eq!(
            super::get("anyocr_selftest", "api_key").as_deref(),
            Some("abcd1234")
        );
        assert_eq!(
            super::status("anyocr_selftest", &["api_key"])["api_key"].as_deref(),
            Some("1234")
        );
        super::set("anyocr_selftest", "api_key", "").unwrap();
        assert_eq!(super::get("anyocr_selftest", "api_key"), None);
    }
}
