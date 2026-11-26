//! PDF 读取与渲染（基于 PDFium）。
//!
//! PDFium 动态库的查找顺序：
//! 1. 环境变量 `ANYOCR_PDFIUM_DIR` 指定的目录；
//! 2. 当前可执行文件所在目录；
//! 3. 开发环境下仓库内的 `vendor/pdfium/<平台>/`（由 `scripts/fetch-pdfium.ps1` 下载）；
//! 4. 系统库。

pub mod pipeline;
pub mod textlayer;

use std::path::{Path, PathBuf};

use image::RgbImage;
use pdfium_render::prelude::*;

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("无法加载 PDFium 动态库：{0}")]
    Library(String),
    #[error("无法打开文档：{0}")]
    Open(String),
    #[error("文档已加密，密码不正确")]
    Password,
    #[error("页码超出范围：{0}")]
    PageOutOfRange(usize),
    #[error("渲染失败：{0}")]
    Render(String),
    #[error("保存失败：{0}")]
    Save(String),
    #[error("已取消")]
    Cancelled,
}

impl From<PdfiumError> for PdfError {
    fn from(e: PdfiumError) -> Self {
        PdfError::Render(format!("{e:?}"))
    }
}

/// 渲染得到的一页。
pub struct RenderedPage {
    /// 0 起始的页序号。
    pub index: usize,
    pub image: RgbImage,
    /// 页面尺寸（PDF 点，1/72 英寸）。
    pub width_pt: f32,
    pub height_pt: f32,
    /// 页面已有可提取文字的字符数（不含空白），用于判断是否需要 OCR。
    pub text_chars: usize,
}

fn vendor_dir() -> PathBuf {
    let platform = if cfg!(target_os = "windows") {
        "win-x64"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "mac-arm64"
        } else {
            "mac-x64"
        }
    } else {
        "linux-x64"
    };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/pdfium")
        .join(platform)
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("ANYOCR_PDFIUM_DIR") {
        v.push(PathBuf::from(d));
    }
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        v.push(d);
    }
    v.push(vendor_dir());
    v
}

/// 已绑定 PDFium 的 PDF 工具。创建开销较大，建议整个进程复用一个实例。
pub struct PdfTool {
    pdfium: Pdfium,
}

impl PdfTool {
    pub fn new() -> Result<Self, PdfError> {
        let mut last_err = String::new();
        for dir in candidate_dirs() {
            let lib = Pdfium::pdfium_platform_library_name_at_path(&dir);
            if !lib.exists() {
                continue;
            }
            match Pdfium::bind_to_library(&lib) {
                Ok(b) => {
                    tracing::debug!("PDFium 已加载：{}", lib.display());
                    return Ok(Self {
                        pdfium: Pdfium::new(b),
                    });
                }
                Err(e) => last_err = format!("{}：{e:?}", lib.display()),
            }
        }
        Pdfium::bind_to_system_library()
            .map(|b| Self {
                pdfium: Pdfium::new(b),
            })
            .map_err(|e| PdfError::Library(format!("{last_err} {e:?}")))
    }

    fn open<'a>(
        &'a self,
        path: &Path,
        password: Option<&'a str>,
    ) -> Result<PdfDocument<'a>, PdfError> {
        self.pdfium
            .load_pdf_from_file(path, password)
            .map_err(|e| match e {
                PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
                    PdfError::Password
                }
                other => PdfError::Open(format!("{}：{other:?}", path.display())),
            })
    }

    /// 文档页数。
    pub fn page_count(&self, path: &Path, password: Option<&str>) -> Result<usize, PdfError> {
        Ok(self.open(path, password)?.pages().len() as usize)
    }

    /// 按 DPI 渲染指定页（0 起始），每渲染一页回调一次。
    /// `min_side` 为渲染结果短边的最小像素数，避免小页面渲染得过小。
    pub fn render_pages(
        &self,
        path: &Path,
        password: Option<&str>,
        pages: &[usize],
        dpi: f32,
        min_side: u32,
        mut on_page: impl FnMut(RenderedPage) -> bool,
    ) -> Result<(), PdfError> {
        let doc = self.open(path, password)?;
        let all = doc.pages();
        let n = all.len() as usize;
        for &i in pages {
            if i >= n {
                return Err(PdfError::PageOutOfRange(i + 1));
            }
            let page = all.get(i as PdfPageIndex)?;
            let (w, h) = (page.width().value, page.height().value);
            let cfg = render_config(&page, dpi, min_side);
            let image = page.render_with_config(&cfg)?.as_image()?.to_rgb8();
            let text_chars = page
                .text()
                .map(|t| t.all().chars().filter(|c| !c.is_whitespace()).count())
                .unwrap_or(0);
            if !on_page(RenderedPage {
                index: i,
                image,
                width_pt: w,
                height_pt: h,
                text_chars,
            }) {
                break;
            }
        }
        Ok(())
    }
}

/// 渲染配置：按 DPI 缩放，短边不低于 `min_side` 像素。渲染与坐标换算必须使用同一配置。
pub fn render_config(page: &PdfPage, dpi: f32, min_side: u32) -> PdfRenderConfig {
    let (w, h) = (page.width().value, page.height().value);
    let mut scale = dpi / 72.0;
    let short = w.min(h) * scale;
    if short < min_side as f32 && short > 0.0 {
        scale = min_side as f32 / w.min(h);
    }
    PdfRenderConfig::new()
        .set_target_width((w * scale).round() as i32)
        .set_maximum_height((h * scale).round() as i32 + 1)
        .render_form_data(true)
        .render_annotations(true)
}

/// 页面上一个可提取字符及其外接框（PDF 用户空间）。
#[derive(Debug, Clone)]
pub struct PageChar {
    pub ch: char,
    /// `[left, bottom, right, top]`
    pub rect: [f32; 4],
}

impl PdfTool {
    /// 用已打开的文档执行操作（测试与校验用）。
    pub fn with_document<R>(
        &self,
        path: &Path,
        password: Option<&str>,
        f: impl FnOnce(&PdfDocument) -> R,
    ) -> Result<R, PdfError> {
        let doc = self.open(path, password)?;
        Ok(f(&doc))
    }

    /// 提取某页（0 起始）全部非空白字符及其位置。
    pub fn page_chars(
        &self,
        path: &Path,
        password: Option<&str>,
        index: usize,
    ) -> Result<Vec<PageChar>, PdfError> {
        let doc = self.open(path, password)?;
        let page = doc.pages().get(index as PdfPageIndex)?;
        let text = page.text()?;
        let mut out = Vec::new();
        for c in text.chars().iter() {
            let Some(ch) = c.unicode_char() else { continue };
            if ch.is_whitespace() {
                continue;
            }
            if let Ok(r) = c.loose_bounds() {
                out.push(PageChar {
                    ch,
                    rect: [
                        r.left().value,
                        r.bottom().value,
                        r.right().value,
                        r.top().value,
                    ],
                });
            }
        }
        Ok(out)
    }

    /// 每页已有可提取文字的非空白字符数（不渲染，速度快）。
    pub fn page_text_counts(
        &self,
        path: &Path,
        password: Option<&str>,
    ) -> Result<Vec<usize>, PdfError> {
        let doc = self.open(path, password)?;
        Ok(doc
            .pages()
            .iter()
            .map(|p| {
                p.text()
                    .map(|t| t.all().chars().filter(|c| !c.is_whitespace()).count())
                    .unwrap_or(0)
            })
            .collect())
    }

    /// 每页已有的文字（只打开一次文档）。
    pub fn pages_text(&self, path: &Path, password: Option<&str>) -> Result<Vec<String>, PdfError> {
        let doc = self.open(path, password)?;
        Ok(doc
            .pages()
            .iter()
            .map(|p| p.text().map(|t| t.all()).unwrap_or_default())
            .collect())
    }

    /// 在某页中搜索文本（PDFium 搜索，与 Chrome / Edge 的 Ctrl+F 同一实现），返回命中次数。
    pub fn search_count(
        &self,
        path: &Path,
        password: Option<&str>,
        index: usize,
        needle: &str,
    ) -> Result<usize, PdfError> {
        let doc = self.open(path, password)?;
        let page = doc.pages().get(index as PdfPageIndex)?;
        let text = page.text()?;
        let search = text.search(needle, &PdfSearchOptions::new())?;
        Ok(search.iter(PdfSearchDirection::SearchForward).count())
    }

    /// 某页全部文字（用于校验）。
    pub fn page_text(
        &self,
        path: &Path,
        password: Option<&str>,
        index: usize,
    ) -> Result<String, PdfError> {
        let doc = self.open(path, password)?;
        let page = doc.pages().get(index as PdfPageIndex)?;
        Ok(page.text()?.all())
    }
}

/// 解析页码范围字符串（1 起始，如 `"1-3,5,8-"`），返回 0 起始的页序号。空字符串表示全部页。
pub fn parse_page_ranges(spec: &str, page_count: usize) -> Result<Vec<usize>, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Ok((0..page_count).collect());
    }
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let parse = |s: &str, default: usize| -> Result<usize, String> {
            if s.is_empty() {
                Ok(default)
            } else {
                s.parse::<usize>().map_err(|_| format!("页码无效：{s}"))
            }
        };
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (parse(a.trim(), 1)?, parse(b.trim(), page_count)?),
            None => {
                let p = parse(part, 1)?;
                (p, p)
            }
        };
        if a == 0 || a > b || b > page_count {
            return Err(format!("页码范围无效：{part}（共 {page_count} 页）"));
        }
        out.extend((a - 1)..b);
    }
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::parse_page_ranges;

    #[test]
    fn page_ranges() {
        assert_eq!(parse_page_ranges("", 3).unwrap(), vec![0, 1, 2]);
        assert_eq!(parse_page_ranges("1-2,5", 6).unwrap(), vec![0, 1, 4]);
        assert_eq!(parse_page_ranges("4-", 5).unwrap(), vec![3, 4]);
        assert!(parse_page_ranges("0", 5).is_err());
        assert!(parse_page_ranges("3-9", 5).is_err());
        assert!(parse_page_ranges("x", 5).is_err());
    }
}
