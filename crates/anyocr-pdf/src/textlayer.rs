//! 不可见文字层写入（GlyphLessFont 方案，参考 Tesseract / OCRmyPDF）。
//!
//! - 字体：Type0 + CIDFontType2，`Identity-H` 编码，CID = UTF-16 码元；`CIDToGIDMap` 把所有 CID
//!   映射到 1 号空字形（宽 500/1000 em）。文字内容只由 `ToUnicode` CMap 决定。
//!   BMP 以外的字符（如数学字母 𝑥）分配到 D800–DFFF 区间的私有 CID，再由 ToUnicode 映射回代理对。
//! - 每个文本块一个 `Tj`，渲染模式 `3 Tr`（不可见），字号 1，用文本矩阵 `Tm` 同时完成
//!   旋转、按框宽水平拉伸、按框高缩放。
//! - 原内容流前后用 `q … Q` 包裹，避免原内容未配对的图形状态影响文字层。
//! - 未加密文档使用增量更新：原文件字节原样保留，新对象追加在文件末尾，原图像不会被重新编码。

use std::collections::BTreeMap;
use std::path::Path;

use anyocr_core::PageResult;
use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, dictionary};

use crate::PdfError;

const GLYPHLESS_TTF: &[u8] = include_bytes!("../assets/glyphless.ttf");
const FONT_KEY: &[u8] = b"FAnyOCR";
const OPEN_MARK: &[u8] = b"q %AnyOCR\n";
const CLOSE_MARK: &[u8] = b"Q %AnyOCR\n";

/// 页面几何信息（PDF 用户空间，单位为点）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    /// 可见区域 `[x0, y0, x1, y1]`（CropBox，缺省为 MediaBox），已规范化为 x0<x1、y0<y1。
    pub crop: [f32; 4],
    /// 页面显示旋转角度，取值 0 / 90 / 180 / 270（顺时针）。
    pub rotate: i32,
}

impl PageGeometry {
    /// 显示尺寸（考虑旋转后的宽高）。
    pub fn display_size(&self) -> (f32, f32) {
        let (w, h) = (self.crop[2] - self.crop[0], self.crop[3] - self.crop[1]);
        if self.rotate % 180 == 0 {
            (w, h)
        } else {
            (h, w)
        }
    }

    /// 把渲染图像上的像素坐标（左上原点、y 向下）换算为 PDF 用户空间坐标。
    pub fn image_to_user(&self, px: f32, py: f32, img_w: f32, img_h: f32) -> (f32, f32) {
        let (dw, dh) = self.display_size();
        let x = px * dw / img_w; // 显示坐标，左上原点
        let y = py * dh / img_h;
        let [x0, y0, x1, y1] = self.crop;
        match self.rotate {
            90 => (x0 + y, y0 + x),
            180 => (x1 - x, y0 + y),
            270 => (x1 - y, y1 - x),
            _ => (x0 + x, y1 - y),
        }
    }
}

/// CID 编码器：BMP 字符直接用码点；BMP 以外的字符分配 D800–DFFF 中的私有 CID。
#[derive(Debug, Default)]
pub struct CidEncoder {
    extra: BTreeMap<u16, char>,
    lookup: std::collections::HashMap<char, u16>,
}

impl CidEncoder {
    const PRIVATE_START: u16 = 0xD800;
    const PRIVATE_END: u16 = 0xDFFF;

    pub fn encode(&mut self, text: &str) -> Vec<u16> {
        text.chars().map(|c| self.cid(c)).collect()
    }

    fn cid(&mut self, c: char) -> u16 {
        let code = c as u32;
        if code <= 0xFFFF {
            return code as u16;
        }
        if let Some(&cid) = self.lookup.get(&c) {
            return cid;
        }
        let next = Self::PRIVATE_START + self.extra.len() as u16;
        if next > Self::PRIVATE_END {
            return 0xFFFD; // 私有区用尽，退化为替换字符
        }
        self.extra.insert(next, c);
        self.lookup.insert(c, next);
        next
    }

    /// 生成 ToUnicode CMap。每个高字节一个 256 项的 bfrange（避免末字节溢出，符合规范），
    /// 跳过代理区 D8–DF，再为私有 CID 写 bfchar。
    pub fn to_unicode_cmap(&self) -> Vec<u8> {
        let mut s = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
             1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        let highs: Vec<u16> = (0u16..=0xFF)
            .filter(|h| !(0xD8..=0xDF).contains(h))
            .collect();
        for chunk in highs.chunks(100) {
            s.push_str(&format!("{} beginbfrange\n", chunk.len()));
            for h in chunk {
                s.push_str(&format!("<{h:02X}00> <{h:02X}FF> <{h:02X}00>\n"));
            }
            s.push_str("endbfrange\n");
        }
        let extra: Vec<(&u16, &char)> = self.extra.iter().collect();
        for chunk in extra.chunks(100) {
            s.push_str(&format!("{} beginbfchar\n", chunk.len()));
            for (cid, ch) in chunk {
                let mut buf = [0u16; 2];
                let units = ch.encode_utf16(&mut buf);
                let hex: String = units.iter().map(|u| format!("{u:04X}")).collect();
                s.push_str(&format!("<{cid:04X}> <{hex}>\n"));
            }
            s.push_str("endbfchar\n");
        }
        s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        s.into_bytes()
    }
}

/// 生成一页文字层的内容流（不含 q/Q 包裹）。`page` 的坐标基于宽 `page.width`、高 `page.height` 的渲染图。
pub fn build_page_content(page: &PageResult, geo: &PageGeometry, enc: &mut CidEncoder) -> Vec<u8> {
    let (iw, ih) = (page.width.max(1) as f32, page.height.max(1) as f32);
    let mut s = String::from("/AnyOCRText BMC\nBT\n3 Tr\n");
    let font = String::from_utf8_lossy(FONT_KEY).into_owned();
    s.push_str(&format!("/{font} 1 Tf\n"));
    let mut wrote = false;
    for b in &page.blocks {
        let text = b.text.trim_end();
        let n = text.chars().count();
        if n == 0 {
            continue;
        }
        let to_user = |p: [f32; 2]| image_point(geo, p, iw, ih);
        let [p0, p1, p2, p3] = b.bbox.map(to_user);
        // 基线：左下 p3 → 右下 p2（与上边 p0→p1 取平均方向更稳）
        let bx = ((p2.0 - p3.0) + (p1.0 - p0.0)) / 2.0;
        let by = ((p2.1 - p3.1) + (p1.1 - p0.1)) / 2.0;
        let width = (bx * bx + by * by).sqrt();
        let height = (((p0.0 - p3.0).powi(2) + (p0.1 - p3.1).powi(2)).sqrt()
            + ((p1.0 - p2.0).powi(2) + (p1.1 - p2.1).powi(2)).sqrt())
            / 2.0;
        if width < 0.5 || height < 0.5 {
            continue;
        }
        let (cos, sin) = (bx / width, by / width);
        let sx = width / (0.5 * n as f32); // 每个字形宽 0.5（字号 1）
        let sy = height;
        let cids = enc.encode(text);
        let hex: String = cids.iter().map(|c| format!("{c:04X}")).collect();
        s.push_str(&format!(
            "{} {} {} {} {} {} Tm\n<{hex}> Tj\n",
            fmt(sx * cos),
            fmt(sx * sin),
            fmt(-sy * sin),
            fmt(sy * cos),
            fmt(p3.0),
            fmt(p3.1)
        ));
        wrote = true;
    }
    s.push_str("ET\nEMC\n");
    if wrote { s.into_bytes() } else { Vec::new() }
}

fn image_point(geo: &PageGeometry, p: [f32; 2], iw: f32, ih: f32) -> (f32, f32) {
    geo.image_to_user(p[0], p[1], iw, ih)
}

fn fmt(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_owned()
    }
}

fn num(o: &Object) -> Option<f32> {
    match o {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

enum Store {
    Incremental(Box<IncrementalDocument>),
    Full(Box<Document>),
}

/// 文字层写入器：打开 PDF，逐页加入文字层，最后保存。
pub struct TextLayerWriter {
    store: Store,
    pages: BTreeMap<u32, ObjectId>,
    font_id: Option<ObjectId>,
    to_unicode_id: Option<ObjectId>,
    encoder: CidEncoder,
    pages_written: usize,
}

impl TextLayerWriter {
    pub fn open(path: &Path, password: Option<&str>) -> Result<Self, PdfError> {
        let bytes =
            std::fs::read(path).map_err(|e| PdfError::Open(format!("{}：{e}", path.display())))?;
        let mut doc = match password {
            Some(p) => {
                Document::load_mem_with_options(&bytes, lopdf::LoadOptions::with_password(p))
            }
            None => Document::load_mem(&bytes),
        }
        .map_err(|e| PdfError::Open(format!("{}：{e}", path.display())))?;
        let pages = doc.get_pages();
        let store = if doc.is_encrypted() || doc.encryption_state.is_some() {
            // 加密文档无法增量更新：解密后整体重写（输出不再加密）
            if doc.is_encrypted() {
                doc.decrypt(password.unwrap_or(""))
                    .map_err(|_| PdfError::Password)?;
            }
            Store::Full(Box::new(doc))
        } else {
            Store::Incremental(Box::new(IncrementalDocument::create_from(bytes, doc)))
        };
        Ok(Self {
            store,
            pages,
            font_id: None,
            to_unicode_id: None,
            encoder: CidEncoder::default(),
            pages_written: 0,
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn pages_written(&self) -> usize {
        self.pages_written
    }

    fn get(&self, id: ObjectId) -> Result<&Object, PdfError> {
        let r = match &self.store {
            Store::Incremental(d) => d
                .new_document
                .get_object(id)
                .or_else(|_| d.get_prev_documents().get_object(id)),
            Store::Full(d) => d.get_object(id),
        };
        r.map_err(|e| PdfError::Open(format!("对象 {id:?}：{e}")))
    }

    fn get_mut(&mut self, id: ObjectId) -> Result<&mut Object, PdfError> {
        let r = match &mut self.store {
            Store::Incremental(d) => {
                d.opt_clone_object_to_new_document(id)
                    .map_err(|e| PdfError::Open(e.to_string()))?;
                d.new_document.get_object_mut(id)
            }
            Store::Full(d) => d.get_object_mut(id),
        };
        r.map_err(|e| PdfError::Open(format!("对象 {id:?}：{e}")))
    }

    fn add(&mut self, obj: impl Into<Object>) -> ObjectId {
        match &mut self.store {
            Store::Incremental(d) => d.new_document.add_object(obj),
            Store::Full(d) => d.add_object(obj),
        }
    }

    fn resolve<'a>(&'a self, o: &'a Object) -> &'a Object {
        match o {
            Object::Reference(id) => self.get(*id).unwrap_or(o),
            _ => o,
        }
    }

    /// 读取页面属性，沿 /Parent 向上查找可继承属性。
    fn inherited(&self, page_id: ObjectId, key: &[u8]) -> Option<Object> {
        let mut id = page_id;
        for _ in 0..64 {
            let dict = self.get(id).ok()?.as_dict().ok()?;
            if let Ok(v) = dict.get(key) {
                return Some(self.resolve(v).clone());
            }
            id = dict.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    }

    fn rect(&self, page_id: ObjectId, key: &[u8]) -> Option<[f32; 4]> {
        let arr = self.inherited(page_id, key)?;
        let arr = arr.as_array().ok()?;
        if arr.len() != 4 {
            return None;
        }
        let v: Vec<f32> = arr.iter().filter_map(|o| num(self.resolve(o))).collect();
        if v.len() != 4 {
            return None;
        }
        Some([
            v[0].min(v[2]),
            v[1].min(v[3]),
            v[0].max(v[2]),
            v[1].max(v[3]),
        ])
    }

    /// 页面几何（0 起始页序号）。
    pub fn geometry(&self, index: usize) -> Result<PageGeometry, PdfError> {
        let id = self.page_id(index)?;
        let media = self
            .rect(id, b"MediaBox")
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let crop = match self.rect(id, b"CropBox") {
            // 与 MediaBox 取交集（PDF 规范要求）
            Some(c) => [
                c[0].max(media[0]),
                c[1].max(media[1]),
                c[2].min(media[2]),
                c[3].min(media[3]),
            ],
            None => media,
        };
        let crop = if crop[2] > crop[0] && crop[3] > crop[1] {
            crop
        } else {
            media
        };
        let rotate = self
            .inherited(id, b"Rotate")
            .and_then(|o| num(&o))
            .unwrap_or(0.0) as i32;
        Ok(PageGeometry {
            crop,
            rotate: rotate.rem_euclid(360) / 90 * 90,
        })
    }

    fn page_id(&self, index: usize) -> Result<ObjectId, PdfError> {
        self.pages
            .get(&(index as u32 + 1))
            .copied()
            .ok_or(PdfError::PageOutOfRange(index + 1))
    }

    fn ensure_font(&mut self) -> ObjectId {
        if let Some(id) = self.font_id {
            return id;
        }
        let mut file = Stream::new(
            dictionary! { "Length1" => GLYPHLESS_TTF.len() as i64 },
            GLYPHLESS_TTF.to_vec(),
        );
        let _ = file.compress();
        let file_id = self.add(file);
        let descriptor_id = self.add(dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => "GlyphLessFont",
            "Flags" => 5,
            "FontBBox" => vec![0.into(), 0.into(), 500.into(), 1000.into()],
            "ItalicAngle" => 0,
            "Ascent" => 1000,
            "Descent" => 0,
            "CapHeight" => 1000,
            "StemV" => 80,
            "FontFile2" => file_id,
        });
        let mut map = Stream::new(Dictionary::new(), [0u8, 1].repeat(65536));
        let _ = map.compress();
        let map_id = self.add(map);
        let cid_font_id = self.add(dictionary! {
            "Type" => "Font",
            "Subtype" => "CIDFontType2",
            "BaseFont" => "GlyphLessFont",
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "FontDescriptor" => descriptor_id,
            "DW" => 500,
            "CIDToGIDMap" => map_id,
        });
        // ToUnicode 在保存时根据实际用到的字符生成，这里先占位
        let tu_id = self.add(Stream::new(Dictionary::new(), Vec::new()));
        let font_id = self.add(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "GlyphLessFont",
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::Reference(cid_font_id)],
            "ToUnicode" => tu_id,
        });
        self.font_id = Some(font_id);
        self.to_unicode_id = Some(tu_id);
        font_id
    }

    fn contents_ids(&self, page_id: ObjectId) -> Vec<Object> {
        let Ok(dict) = self
            .get(page_id)
            .and_then(|o| o.as_dict().map_err(|e| PdfError::Open(e.to_string())))
        else {
            return vec![];
        };
        match dict.get(b"Contents") {
            Ok(Object::Reference(id)) => match self.get(*id) {
                Ok(Object::Array(a)) => a.clone(),
                _ => vec![Object::Reference(*id)],
            },
            Ok(Object::Array(a)) => a.clone(),
            _ => vec![],
        }
    }

    fn stream_starts_with(&self, o: &Object, prefix: &[u8]) -> bool {
        let Object::Reference(id) = o else {
            return false;
        };
        match self.get(*id) {
            Ok(Object::Stream(s)) => {
                let content = s
                    .decompressed_content()
                    .unwrap_or_else(|_| s.content.clone());
                content.starts_with(prefix)
            }
            _ => false,
        }
    }

    /// 页面是否已有 AnyOCR 写入的文字层。
    pub fn has_own_layer(&self, index: usize) -> bool {
        self.page_id(index)
            .map(|id| {
                self.contents_ids(id)
                    .last()
                    .is_some_and(|o| self.stream_starts_with(o, CLOSE_MARK))
            })
            .unwrap_or(false)
    }

    /// 移除 AnyOCR 之前写入的文字层（redo 模式）。
    pub fn remove_own_layer(&mut self, index: usize) -> Result<bool, PdfError> {
        let id = self.page_id(index)?;
        let mut c = self.contents_ids(id);
        let has_open = c
            .first()
            .is_some_and(|o| self.stream_starts_with(o, OPEN_MARK));
        let has_close = c
            .last()
            .is_some_and(|o| self.stream_starts_with(o, CLOSE_MARK));
        if !(has_open && has_close) {
            return Ok(false);
        }
        c.remove(0);
        c.pop();
        let page = self
            .get_mut(id)?
            .as_dict_mut()
            .map_err(|e| PdfError::Open(e.to_string()))?;
        page.set("Contents", Object::Array(c));
        Ok(true)
    }

    /// 为一页加入文字层。`result` 为该页渲染图上的识别结果（建议先做排版解析，以获得更好的提取顺序）。
    pub fn add_page(&mut self, index: usize, result: &PageResult) -> Result<(), PdfError> {
        let page_id = self.page_id(index)?;
        let geo = self.geometry(index)?;
        let body = build_page_content(result, &geo, &mut self.encoder);
        if body.is_empty() {
            return Ok(());
        }
        let font_id = self.ensure_font();

        // 内容流：[q, 原内容…, Q + 文字层]
        let mut close = CLOSE_MARK.to_vec();
        close.extend_from_slice(&body);
        let mut close_stream = Stream::new(Dictionary::new(), close);
        let _ = close_stream.compress();
        let close_id = self.add(close_stream);
        let open_id = self.add(Stream::new(Dictionary::new(), OPEN_MARK.to_vec()));
        let mut contents = self.contents_ids(page_id);
        contents.insert(0, Object::Reference(open_id));
        contents.push(Object::Reference(close_id));

        // 资源：确保页面拥有自己的 Resources，并在 /Font 中加入字体
        let resources = self
            .inherited(page_id, b"Resources")
            .unwrap_or(Object::Dictionary(Dictionary::new()));
        let mut res_dict = match resources {
            Object::Dictionary(d) => d,
            _ => Dictionary::new(),
        };
        let mut fonts = match res_dict.get(b"Font").map(|o| self.resolve(o).clone()) {
            Ok(Object::Dictionary(d)) => d,
            _ => Dictionary::new(),
        };
        fonts.set(FONT_KEY, Object::Reference(font_id));
        res_dict.set("Font", Object::Dictionary(fonts));

        let page = self
            .get_mut(page_id)?
            .as_dict_mut()
            .map_err(|e| PdfError::Open(e.to_string()))?;
        page.set("Contents", Object::Array(contents));
        page.set("Resources", Object::Dictionary(res_dict));
        self.pages_written += 1;
        Ok(())
    }

    /// 保存到 `out`。
    pub fn save(mut self, out: &Path) -> Result<(), PdfError> {
        if let Some(tu) = self.to_unicode_id {
            let mut s = Stream::new(Dictionary::new(), self.encoder.to_unicode_cmap());
            let _ = s.compress();
            *self.get_mut(tu)? = Object::Stream(s);
        }
        let tmp = out.with_extension("anyocr-tmp");
        let res = match &mut self.store {
            Store::Incremental(d) => d.save(&tmp).map(|_| ()),
            Store::Full(d) => d.save(&tmp).map(|_| ()),
        };
        res.map_err(|e| PdfError::Save(e.to_string()))?;
        std::fs::rename(&tmp, out)
            .map_err(|e| PdfError::Save(format!("{}：{e}", out.display())))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo(crop: [f32; 4], rotate: i32) -> PageGeometry {
        PageGeometry { crop, rotate }
    }

    #[test]
    fn image_to_user_all_rotations() {
        let crop = [10.0, 20.0, 110.0, 220.0]; // 宽 100，高 200
        // 旋转 0：图像左上 → 用户空间 (x0, y1)
        let g = geo(crop, 0);
        assert_eq!(g.display_size(), (100.0, 200.0));
        assert_eq!(g.image_to_user(0.0, 0.0, 100.0, 200.0), (10.0, 220.0));
        assert_eq!(g.image_to_user(100.0, 200.0, 100.0, 200.0), (110.0, 20.0));
        // 旋转 90：显示宽高互换，图像左上 → (x0, y0)
        let g = geo(crop, 90);
        assert_eq!(g.display_size(), (200.0, 100.0));
        assert_eq!(g.image_to_user(0.0, 0.0, 200.0, 100.0), (10.0, 20.0));
        assert_eq!(g.image_to_user(200.0, 0.0, 200.0, 100.0), (10.0, 220.0));
        // 旋转 180：图像左上 → (x1, y0)
        let g = geo(crop, 180);
        assert_eq!(g.image_to_user(0.0, 0.0, 100.0, 200.0), (110.0, 20.0));
        // 旋转 270：图像左上 → (x1, y1)
        let g = geo(crop, 270);
        assert_eq!(g.image_to_user(0.0, 0.0, 200.0, 100.0), (110.0, 220.0));
    }

    #[test]
    fn encoder_handles_non_bmp() {
        let mut e = CidEncoder::default();
        let cids = e.encode("a中𝑥𝑥");
        assert_eq!(cids[0], 'a' as u16);
        assert_eq!(cids[1], '中' as u16);
        assert_eq!(cids[2], 0xD800);
        assert_eq!(cids[3], 0xD800);
        let cmap = String::from_utf8(e.to_unicode_cmap()).unwrap();
        assert!(cmap.contains("<D800> <D835DC65>"));
        assert!(cmap.contains("<4E00> <4EFF> <4E00>"));
        assert!(!cmap.contains("<D800> <D8FF>"));
    }

    #[test]
    fn number_format() {
        assert_eq!(fmt(1.0), "1");
        assert_eq!(fmt(-0.0001), "0");
        assert_eq!(fmt(12.3456), "12.346");
    }
}
