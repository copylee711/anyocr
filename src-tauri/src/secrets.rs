//! 在线引擎的密钥保存在系统凭据管理器，与命令行 `anyocr auth` 共用同一套条目，实现见
//! `anyocr_engines::credentials`。

pub use anyocr_engines::credentials::{get, set, status};

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
