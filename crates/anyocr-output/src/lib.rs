//! 识别结果输出。格式与 Umi-OCR（`py_src/ocr/output/`，MIT）保持一致：
//!
//! | 格式 | 文件 | 说明 |
//! |---|---|---|
//! | `txt` | `<名>.txt` | 开头写开始时间；每项以 `≦ 文件名 ≧` 开头，项间空一行 |
//! | `txt_plain` | `<名>.p.txt` | 只有识别文字，强制忽略空白项 |
//! | `txt_individual` | 每个源文件一个 `.txt` | 写到源文件旁或指定目录 |
//! | `md` | `<名>.md` | 图片链接 + 引用块文字 |
//! | `csv` | `<名>.csv` | 表头 Name, OCR, Path；UTF-8 BOM（Excel 打开不乱码） |
//! | `jsonl` | `<名>.jsonl` | 每项一行 JSON |

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyocr_core::PageResult;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error("写入 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

type Result<T> = std::result::Result<T, OutputError>;

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> OutputError + '_ {
    move |source| OutputError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Txt,
    TxtPlain,
    TxtIndividual,
    Md,
    Csv,
    Jsonl,
}

impl Format {
    pub const ALL: [Format; 6] = [
        Format::Txt,
        Format::TxtPlain,
        Format::TxtIndividual,
        Format::Md,
        Format::Csv,
        Format::Jsonl,
    ];
}

/// 输出配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    /// 输出目录（txt_individual 在 `individual_beside_source` 为真时写到源文件旁）。
    pub dir: PathBuf,
    /// 不含扩展名的文件名（通常由 [`fill_name_template`] 生成）。
    pub file_name: String,
    pub formats: Vec<Format>,
    /// 忽略没有文字或失败的项。
    pub ignore_blank: bool,
    pub individual_beside_source: bool,
}

/// 一条待输出的记录。
#[derive(Debug, Clone)]
pub struct Record {
    /// 显示名（文件名，文档页为 `文件名 #页码`）。
    pub name: String,
    /// 源文件路径。
    pub path: PathBuf,
    /// 文档页码（1 起始）；图片为空。
    pub page: Option<usize>,
    pub result: std::result::Result<PageResult, String>,
}

impl Record {
    fn is_blank(&self) -> bool {
        !matches!(&self.result, Ok(p) if !p.blocks.is_empty())
    }
}

/// 拼接文本（与 Umi `getDataText` 一致：最后一块不追加结尾分隔符）。
pub fn data_text(page: &PageResult) -> String {
    let mut s = String::new();
    let n = page.blocks.len();
    for (i, b) in page.blocks.iter().enumerate() {
        s.push_str(&b.text);
        if i + 1 < n {
            s.push_str(&b.end);
        }
    }
    s
}

/// 填充文件名模板：`%name` 替换为名称，`%date` 按 `datetime_format` 格式化
/// （支持 `%Y %m %d %H %M %S %unix`）。
pub fn fill_name_template(
    template: &str,
    name: &str,
    datetime_format: &str,
    now: DateTime<Local>,
) -> String {
    let date = datetime_format
        .replace("%unix", &now.timestamp().to_string())
        .replace("%Y", &now.format("%Y").to_string())
        .replace("%m", &now.format("%m").to_string())
        .replace("%d", &now.format("%d").to_string())
        .replace("%H", &now.format("%H").to_string())
        .replace("%M", &now.format("%M").to_string())
        .replace("%S", &now.format("%S").to_string());
    let out = template.replace("%name", name).replace("%date", &date);
    // 去掉 Windows 文件名非法字符
    out.chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect()
}

/// 多种格式的输出集合。
pub struct OutputSet {
    cfg: OutputConfig,
    started: String,
    txt: Option<BufWriter<File>>,
    plain: Option<BufWriter<File>>,
    md: Option<(PathBuf, BufWriter<File>)>,
    jsonl: Option<BufWriter<File>>,
    csv_rows: Option<Vec<[String; 3]>>,
    written: Vec<PathBuf>,
}

impl OutputSet {
    pub fn new(cfg: OutputConfig, now: DateTime<Local>) -> Result<Self> {
        std::fs::create_dir_all(&cfg.dir).map_err(io_err(&cfg.dir))?;
        let started = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let mut s = Self {
            cfg,
            started,
            txt: None,
            plain: None,
            md: None,
            jsonl: None,
            csv_rows: None,
            written: Vec::new(),
        };
        for f in s.cfg.formats.clone() {
            match f {
                Format::Txt => {
                    let mut w = s.create("txt")?;
                    write!(w, "{}\n\n", s.started).map_err(io_err(&s.cfg.dir))?;
                    s.txt = Some(w);
                }
                Format::TxtPlain => s.plain = Some(s.create("p.txt")?),
                Format::Md => {
                    let path = s.path("md");
                    let mut w = s.create("md")?;
                    write!(w, "> {}\n\n", s.started).map_err(io_err(&path))?;
                    s.md = Some((path, w));
                }
                Format::Jsonl => s.jsonl = Some(s.create("jsonl")?),
                Format::Csv => s.csv_rows = Some(Vec::new()),
                Format::TxtIndividual => {}
            }
        }
        Ok(s)
    }

    fn path(&self, ext: &str) -> PathBuf {
        self.cfg.dir.join(format!("{}.{ext}", self.cfg.file_name))
    }

    fn create(&mut self, ext: &str) -> Result<BufWriter<File>> {
        let path = self.path(ext);
        let f = File::create(&path).map_err(io_err(&path))?;
        self.written.push(path);
        Ok(BufWriter::new(f))
    }

    /// 已创建 / 写入的文件。
    pub fn files(&self) -> &[PathBuf] {
        &self.written
    }

    pub fn push(&mut self, r: &Record) -> Result<()> {
        let blank = r.is_blank();
        let text = match &r.result {
            Ok(p) => data_text(p),
            Err(_) => String::new(),
        };
        let err_line = match &r.result {
            Err(e) => Some(format!(
                "[Error] OCR failed. Msg: {e}\n【异常】OCR识别失败。"
            )),
            _ => None,
        };
        let dir = self.cfg.dir.clone();
        if let Some(w) = &mut self.txt
            && !(blank && self.cfg.ignore_blank)
        {
            let mut out = format!("≦ {} ≧\n", r.name);
            if !text.is_empty() {
                out.push_str(&text);
                out.push('\n');
            }
            if let Some(e) = &err_line {
                out.push_str(e);
                out.push('\n');
            }
            out.push('\n');
            w.write_all(out.as_bytes()).map_err(io_err(&dir))?;
        }
        if let Some(w) = &mut self.plain
            && !blank
        {
            let mut out = text.clone();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            w.write_all(out.as_bytes()).map_err(io_err(&dir))?;
        }
        if let Some((md_path, w)) = &mut self.md
            && !(blank && self.cfg.ignore_blank)
        {
            let rel = relative_link(md_path.parent().unwrap_or(Path::new(".")), &r.path);
            let mut out = if r.page.is_some() {
                format!("\n---\n[{}]({rel})\n", r.name)
            } else {
                format!("\n---\n![{0}]({rel})\n[{0}]({rel})\n", r.name)
            };
            if let Some(e) = &err_line {
                for l in e.lines() {
                    out.push_str(&format!("> {l}  \n"));
                }
            } else if !text.is_empty() {
                for l in text.split('\n') {
                    out.push_str(&format!("> {l}  \n"));
                }
            }
            w.write_all(out.as_bytes()).map_err(io_err(md_path))?;
        }
        if let Some(w) = &mut self.jsonl {
            let v = serde_json::json!({
                "name": r.name,
                "path": r.path,
                "page": r.page,
                "code": match &r.result { Ok(p) if !p.blocks.is_empty() => 100, Ok(_) => 101, Err(_) => 102 },
                "data": match &r.result { Ok(p) => serde_json::to_value(&p.blocks).unwrap_or_default(), Err(e) => e.clone().into() },
                "text": text,
            });
            writeln!(w, "{v}").map_err(io_err(&dir))?;
        }
        if let Some(rows) = &mut self.csv_rows
            && !(blank && self.cfg.ignore_blank)
        {
            rows.push([
                r.name.clone(),
                err_line.clone().unwrap_or(text.clone()),
                r.path.display().to_string(),
            ]);
        }
        if self.cfg.formats.contains(&Format::TxtIndividual) && !(blank && self.cfg.ignore_blank) {
            let stem = r
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let stem = match r.page {
                Some(p) => format!("{stem}_p{p}"),
                None => stem,
            };
            let base = if self.cfg.individual_beside_source {
                r.path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| dir.clone())
            } else {
                dir.clone()
            };
            let path = base.join(format!("{stem}.txt"));
            let body = err_line.unwrap_or(text);
            std::fs::write(&path, body).map_err(io_err(&path))?;
            self.written.push(path);
        }
        Ok(())
    }

    /// 结束输出：刷新缓冲并写出 csv。
    pub fn finish(mut self) -> Result<Vec<PathBuf>> {
        for w in [self.txt.as_mut(), self.plain.as_mut(), self.jsonl.as_mut()]
            .into_iter()
            .flatten()
        {
            w.flush().map_err(io_err(&self.cfg.dir))?;
        }
        if let Some((p, w)) = self.md.as_mut() {
            w.flush().map_err(io_err(p))?;
        }
        if let Some(rows) = self.csv_rows.take() {
            let path = self.path("csv");
            let mut s = String::from("\u{feff}Name,OCR,Path\r\n");
            for row in rows {
                s.push_str(
                    &row.iter()
                        .map(|f| csv_field(f))
                        .collect::<Vec<_>>()
                        .join(","),
                );
                s.push_str("\r\n");
            }
            std::fs::write(&path, s).map_err(io_err(&path))?;
            self.written.push(path);
        }
        Ok(self.written)
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// md 中从 md 文件目录到源文件的相对链接（空格转 %20，统一正斜杠）。
fn relative_link(from_dir: &Path, target: &Path) -> String {
    let rel = pathdiff(target, from_dir).unwrap_or_else(|| target.to_path_buf());
    rel.to_string_lossy().replace('\\', "/").replace(' ', "%20")
}

fn pathdiff(path: &Path, base: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let (path, base) = (
        path.components().collect::<Vec<_>>(),
        base.components().collect::<Vec<_>>(),
    );
    // 不同盘符无法构造相对路径
    if path.first() != base.first() {
        return None;
    }
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for c in &base[common..] {
        if matches!(c, Component::Normal(_)) {
            out.push("..");
        }
    }
    for c in &path[common..] {
        out.push(c.as_os_str());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyocr_core::TextBlock;
    use chrono::TimeZone;

    fn page(texts: &[(&str, &str)]) -> PageResult {
        PageResult {
            width: 10,
            height: 10,
            blocks: texts
                .iter()
                .map(|(t, e)| {
                    let mut b = TextBlock::from_rect(*t, [0.0, 0.0, 1.0, 1.0], 0.9);
                    b.end = (*e).into();
                    b
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn data_text_skips_last_end() {
        assert_eq!(data_text(&page(&[("a", " "), ("b", "\n")])), "a b");
    }

    #[test]
    fn name_template() {
        let now = Local.with_ymd_and_hms(2023, 9, 1, 12, 13, 5).unwrap();
        assert_eq!(
            fill_name_template("[OCR]_%name_%date", "我的图片", "%Y-%m-%d_%H-%M", now),
            "[OCR]_我的图片_2023-09-01_12-13"
        );
        assert_eq!(fill_name_template("a:%name", "b/c", "%unix", now), "a_b_c");
    }

    #[test]
    fn writes_all_formats() {
        let dir = std::env::temp_dir().join("anyocr-output-test");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("imgs");
        std::fs::create_dir_all(&src).unwrap();
        let cfg = OutputConfig {
            dir: dir.join("out"),
            file_name: "res".into(),
            formats: Format::ALL.to_vec(),
            ignore_blank: true,
            individual_beside_source: false,
        };
        let now = Local.with_ymd_and_hms(2023, 9, 1, 12, 0, 0).unwrap();
        let mut set = OutputSet::new(cfg, now).unwrap();
        set.push(&Record {
            name: "a b.png".into(),
            path: src.join("a b.png"),
            page: None,
            result: Ok(page(&[("你好", ""), ("世界", "\n"), ("x,\"y\"", "\n")])),
        })
        .unwrap();
        set.push(&Record {
            name: "blank.png".into(),
            path: src.join("blank.png"),
            page: None,
            result: Ok(page(&[])),
        })
        .unwrap();
        let files = set.finish().unwrap();
        let read = |n: &str| std::fs::read_to_string(dir.join("out").join(n)).unwrap();
        assert!(read("res.txt").contains("≦ a b.png ≧\n你好世界\nx,\"y\"\n\n"));
        assert!(!read("res.txt").contains("blank"));
        assert_eq!(read("res.p.txt"), "你好世界\nx,\"y\"\n");
        assert!(read("res.md").contains("![a b.png](../imgs/a%20b.png)"));
        assert!(read("res.md").contains("> 你好世界  \n"));
        let csv = read("res.csv");
        assert!(csv.starts_with("\u{feff}Name,OCR,Path\r\n"));
        assert!(csv.contains("\"你好世界\nx,\"\"y\"\"\""));
        assert_eq!(read("res.jsonl").lines().count(), 2);
        assert_eq!(read("a b.txt"), "你好世界\nx,\"y\"");
        assert!(files.len() >= 6);
    }
}
