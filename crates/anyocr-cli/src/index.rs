//! 扫描件的页文字缓存与关键词搜索（MCP 服务使用）。
//!
//! 扫描页的 OCR 结果按文件缓存在 `<缓存目录>/anyocr/pages/<键>.json`，键由路径、大小、修改时间决定，
//! 文件变了缓存自然失效。这样智能体反复读取或搜索同一本书时不必重复识别。

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 一份文档的页文字缓存（0 起始页序号 → 文字）。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PageCache {
    pub source: String,
    pub pages: BTreeMap<usize, String>,
    #[serde(skip)]
    file: PathBuf,
}

fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("anyocr")
        .join("pages")
}

fn key(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canon.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok().hash(&mut h);
    Some(format!("{:016x}", h.finish()))
}

impl PageCache {
    /// 读取（不存在时为空缓存）。
    pub fn load(path: &Path) -> Self {
        let Some(k) = key(path) else {
            return Self::default();
        };
        let file = cache_dir().join(format!("{k}.json"));
        let mut c: PageCache = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        c.source = path.display().to_string();
        c.file = file;
        c
    }

    pub fn save(&self) {
        if self.file.as_os_str().is_empty() {
            return;
        }
        if let Some(d) = self.file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = serde_json::to_string(self) {
            let _ = std::fs::write(&self.file, s);
        }
    }
}

/// 一处命中。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    /// 1 起始页码。
    pub page: usize,
    pub context: String,
}

/// 搜索用的规范化：去掉空白（OCR 文本的换行、空格常把词拆开），转小写。
fn normalize(s: &str) -> Vec<char> {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 在各页文字中查找 `query`（忽略空白与大小写），每处命中附前后 `context` 个字符。
/// `pages` 为 (0 起始页序号, 文字)。最多返回 `limit` 处，同时返回命中总数。
pub fn search<'a>(
    pages: impl IntoIterator<Item = (usize, &'a str)>,
    query: &str,
    context: usize,
    limit: usize,
) -> (Vec<Hit>, usize) {
    let q = normalize(query);
    let mut hits = Vec::new();
    let mut total = 0;
    if q.is_empty() {
        return (hits, 0);
    }
    for (i, text) in pages {
        let t = normalize(text);
        let mut start = 0;
        while start + q.len() <= t.len() {
            if t[start..start + q.len()] == q[..] {
                total += 1;
                if hits.len() < limit {
                    let a = start.saturating_sub(context);
                    let b = (start + q.len() + context).min(t.len());
                    let s: String = t[a..start].iter().collect();
                    let m: String = t[start..start + q.len()].iter().collect();
                    let e: String = t[start + q.len()..b].iter().collect();
                    hits.push(Hit {
                        page: i + 1,
                        context: format!(
                            "{}{s}【{m}】{e}{}",
                            if a > 0 { "…" } else { "" },
                            if b < t.len() { "…" } else { "" }
                        ),
                    });
                }
                start += q.len();
            } else {
                start += 1;
            }
        }
    }
    (hits, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ignores_whitespace_and_case() {
        let pages = [
            (0, "第一章 初等积分法\n1.1 微分方程与解"),
            (4, "线性微分\n方程的解。Linear ODE"),
        ];
        let (hits, total) = search(pages.iter().map(|(i, t)| (*i, *t)), "微分方程", 3, 10);
        assert_eq!(total, 2);
        assert_eq!(hits[0].page, 1);
        assert_eq!(hits[0].context, "…1.1【微分方程】与解");
        assert_eq!(hits[1].page, 5);
        assert_eq!(hits[1].context, "线性【微分方程】的解。…");
        let (hits, total) = search(pages.iter().map(|(i, t)| (*i, *t)), "linear ode", 2, 1);
        assert_eq!((hits.len(), total), (1, 1));
    }

    #[test]
    fn limit_counts_all() {
        let (hits, total) = search([(0usize, "aaaa")], "a", 0, 2);
        assert_eq!((hits.len(), total), (2, 4));
    }
}
