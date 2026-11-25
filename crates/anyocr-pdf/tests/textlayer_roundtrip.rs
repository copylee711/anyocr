//! 文字层端到端坐标测试（不依赖 OCR 模型）。
//!
//! 1. 用 lopdf 生成含可见文字的 PDF，覆盖 0/90/180/270 旋转、CropBox 偏移、非零原点 MediaBox、倾斜文字；
//! 2. 用 PDFium 自己的坐标换算（FPDF_PageToDevice）把原字符框转成渲染图上的像素四边形，模拟 OCR 输出；
//! 3. 用 TextLayerWriter 写入不可见文字层，再用 PDFium 提取新文字及其位置，与原文字比对。
//!
//! 坐标换算若有任何错误（旋转方向、CropBox 偏移、y 轴翻转），新文字位置都会与原文字错开。

use std::path::{Path, PathBuf};

use anyocr_core::{PageResult, TextBlock};
use anyocr_pdf::textlayer::TextLayerWriter;
use anyocr_pdf::{PdfTool, render_config};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use pdfium_render::prelude::*;

struct Spec {
    media: [f32; 4],
    crop: Option<[f32; 4]>,
    rotate: i64,
}

const WORDS: [(&str, f32, f32, f32); 2] = [
    ("HELLO", 140.0, 520.0, 28.0),
    ("WORLD12345", 140.0, 440.0, 20.0),
];
const TILTED: &str = "TILTED";

fn make_fixture(path: &Path, specs: &[Spec], with_text: bool) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let mut kids = vec![];
    for s in specs {
        let [mx, my, ..] = s.media;
        let mut ops = vec![];
        for (w, x, y, size) in WORDS {
            ops.extend([
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), size.into()]),
                Operation::new("Td", vec![(mx + x).into(), (my + y).into()]),
                Operation::new("Tj", vec![Object::string_literal(w)]),
                Operation::new("ET", vec![]),
            ]);
        }
        // 倾斜 30° 的一行
        let (c, sn) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
        ops.extend([
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 1.into()]),
            Operation::new(
                "Tm",
                vec![
                    (22.0 * c).into(),
                    (22.0 * sn).into(),
                    (-22.0 * sn).into(),
                    (22.0 * c).into(),
                    (mx + 160.0).into(),
                    (my + 250.0).into(),
                ],
            ),
            Operation::new("Tj", vec![Object::string_literal(TILTED)]),
            Operation::new("ET", vec![]),
        ]);
        if !with_text {
            ops.clear();
        }
        let content = Content { operations: ops };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => s.media.iter().map(|v| Object::Real(*v)).collect::<Vec<_>>(),
            "Rotate" => s.rotate,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
        };
        if let Some(c) = s.crop {
            page.set(
                "CropBox",
                c.iter().map(|v| Object::Real(*v)).collect::<Vec<_>>(),
            );
        }
        kids.push(Object::Reference(doc.add_object(page)));
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("anyocr-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// 把用户空间点转为渲染图像素坐标。
fn to_px(page: &PdfPage, cfg: &PdfRenderConfig, x: f32, y: f32) -> [f32; 2] {
    let (px, py) = page
        .points_to_pixels(PdfPoints::new(x), PdfPoints::new(y), cfg)
        .unwrap();
    [px as f32, py as f32]
}

struct Orig {
    page: PageResult,
    /// 每个块对应的原字符框（用户空间 [l, b, r, t]）的并集，以及块文本
    expected: Vec<(String, [f32; 4], bool)>,
}

/// 用 PDFium 读取原文字，构造“模拟 OCR”结果。
fn simulate_ocr(pdf: &PdfTool, path: &Path, index: usize) -> Orig {
    pdf.with_document(path, None, |doc| {
        let page = doc.pages().get(index as PdfPageIndex).unwrap();
        let cfg = render_config(&page, 100.0, 0);
        let bmp = page.render_with_config(&cfg).unwrap();
        let (iw, ih) = (bmp.width() as u32, bmp.height() as u32);
        let chars: Vec<(char, [f32; 4])> = page
            .text()
            .unwrap()
            .chars()
            .iter()
            .filter_map(|c| {
                let ch = c.unicode_char()?;
                let r = c.loose_bounds().ok()?;
                (!ch.is_whitespace()).then_some((
                    ch,
                    [
                        r.left().value,
                        r.bottom().value,
                        r.right().value,
                        r.top().value,
                    ],
                ))
            })
            .collect();
        let mut blocks = vec![];
        let mut expected = vec![];
        let seq: Vec<char> = chars.iter().map(|c| c.0).collect();
        let find = |w: &str| {
            let w: Vec<char> = w.chars().collect();
            seq.windows(w.len())
                .position(|win| win == w.as_slice())
                .expect("找不到原文字")
        };
        // 水平单词：整词一个块，四边形按阅读方向（用户空间中文字是正的）取角点再换算
        for (w, ..) in WORDS {
            let n = w.chars().count();
            let i = find(w);
            let group = &chars[i..i + n];
            let text: String = group.iter().map(|c| c.0).collect();
            let l = group.iter().map(|c| c.1[0]).fold(f32::MAX, f32::min);
            let b = group.iter().map(|c| c.1[1]).fold(f32::MAX, f32::min);
            let r = group.iter().map(|c| c.1[2]).fold(f32::MIN, f32::max);
            let t = group.iter().map(|c| c.1[3]).fold(f32::MIN, f32::max);
            let quad = [
                to_px(&page, &cfg, l, t),
                to_px(&page, &cfg, r, t),
                to_px(&page, &cfg, r, b),
                to_px(&page, &cfg, l, b),
            ];
            blocks.push(TextBlock::new(text.clone(), quad, 1.0));
            expected.push((text, [l, b, r, t], false));
        }
        // 倾斜文字：逐字一个块
        let i = find(TILTED);
        for (ch, [l, b, r, t]) in &chars[i..i + TILTED.len()] {
            let quad = [
                to_px(&page, &cfg, *l, *t),
                to_px(&page, &cfg, *r, *t),
                to_px(&page, &cfg, *r, *b),
                to_px(&page, &cfg, *l, *b),
            ];
            blocks.push(TextBlock::new(ch.to_string(), quad, 1.0));
            expected.push((ch.to_string(), [*l, *b, *r, *t], true));
        }
        Orig {
            page: PageResult {
                width: iw,
                height: ih,
                blocks,
                ..Default::default()
            },
            expected,
        }
    })
    .unwrap()
}

fn iou(a: [f32; 4], b: [f32; 4]) -> f32 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = ix * iy;
    let area = |r: [f32; 4]| (r[2] - r[0]) * (r[3] - r[1]);
    inter / (area(a) + area(b) - inter)
}

#[test]
fn text_layer_positions_match_original_text() {
    let specs = vec![
        Spec {
            media: [0.0, 0.0, 612.0, 792.0],
            crop: None,
            rotate: 0,
        },
        Spec {
            media: [0.0, 0.0, 612.0, 792.0],
            crop: None,
            rotate: 90,
        },
        Spec {
            media: [0.0, 0.0, 612.0, 792.0],
            crop: None,
            rotate: 180,
        },
        Spec {
            media: [0.0, 0.0, 612.0, 792.0],
            crop: None,
            rotate: 270,
        },
        Spec {
            media: [0.0, 0.0, 612.0, 792.0],
            crop: Some([40.0, 60.0, 560.0, 700.0]),
            rotate: 0,
        },
        Spec {
            media: [100.0, 200.0, 712.0, 992.0],
            crop: Some([120.0, 230.0, 700.0, 950.0]),
            rotate: 90,
        },
    ];
    // src：带可见文字，用于模拟 OCR；blank：几何相同但无文字，文字层写在它上面，提取结果只含新文字
    let src = tmp("roundtrip-src.pdf");
    let blank = tmp("roundtrip-blank.pdf");
    let out = tmp("roundtrip-out.pdf");
    make_fixture(&src, &specs, true);
    make_fixture(&blank, &specs, false);

    let pdf = PdfTool::new().expect("需要先运行 scripts/fetch-pdfium.ps1");
    let mut writer = TextLayerWriter::open(&blank, None).unwrap();
    let mut origs = vec![];
    for i in 0..specs.len() {
        let o = simulate_ocr(&pdf, &src, i);
        writer.add_page(i, &o.page).unwrap();
        origs.push(o);
    }
    assert_eq!(writer.pages_written(), specs.len());
    writer.save(&out).unwrap();

    // 原文件字节应作为前缀被完整保留（增量更新）
    let a = std::fs::read(&blank).unwrap();
    let b = std::fs::read(&out).unwrap();
    assert!(b.starts_with(&a), "输出不是增量更新");
    assert!(
        b.len() - a.len() < 200_000,
        "增量过大：{} 字节",
        b.len() - a.len()
    );

    for (i, o) in origs.iter().enumerate() {
        let new_chars = pdf.page_chars(&out, None, i).unwrap();
        // 提取顺序由 PDFium 按显示方向决定，因此按内容匹配，只要求字符总数一致
        let expected_len: usize = o.expected.iter().map(|e| e.0.chars().count()).sum();
        assert_eq!(new_chars.len(), expected_len, "第 {} 页字符数不一致", i + 1);
        let seq: Vec<char> = new_chars.iter().map(|c| c.ch).collect();
        let mut used = vec![false; seq.len()];

        for (text, rect, per_char) in &o.expected {
            let n = text.chars().count();
            let want: Vec<char> = text.chars().collect();
            // 取第一个未使用、且与原文字框重叠最大的匹配位置
            let k = (0..=seq.len() - n)
                .filter(|&k| seq[k..k + n] == want[..] && !used[k])
                .max_by(|&a, &b| {
                    let c = |k: usize| new_chars[k].rect;
                    iou(c(a), *rect).total_cmp(&iou(c(b), *rect))
                })
                .unwrap_or_else(|| panic!("第 {} 页找不到 {text:?}：{seq:?}", i + 1));
            used[k] = true;
            let group = &new_chars[k..k + n];
            let union = [
                group.iter().map(|c| c.rect[0]).fold(f32::MAX, f32::min),
                group.iter().map(|c| c.rect[1]).fold(f32::MAX, f32::min),
                group.iter().map(|c| c.rect[2]).fold(f32::MIN, f32::max),
                group.iter().map(|c| c.rect[3]).fold(f32::MIN, f32::max),
            ];
            let score = iou(union, *rect);
            let min = if *per_char { 0.5 } else { 0.75 };
            assert!(
                score > min,
                "第 {} 页 {text:?} 位置偏差：IoU {score:.2}，新 {union:?} 原 {rect:?}",
                i + 1
            );
        }
    }

    // 重新打开：检测自有文字层、redo 移除后与原文一致
    let mut again = TextLayerWriter::open(&out, None).unwrap();
    assert!(again.has_own_layer(0));
    assert!(again.remove_own_layer(0).unwrap());
    let redo = tmp("roundtrip-redo.pdf");
    again.save(&redo).unwrap();
    assert!(pdf.page_chars(&redo, None, 0).unwrap().is_empty());
    assert!(!pdf.page_chars(&redo, None, 1).unwrap().is_empty());
}
