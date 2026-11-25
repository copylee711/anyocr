//! 二维码 / 条形码读写，基于 [rxing](https://github.com/rxing-core/rxing)（ZXing 的 Rust 移植，Apache-2.0）。
//!
//! 预处理参数与 Umi-OCR 二维码页一致：中值滤波、锐度、对比度、灰度、二值化阈值。
//! rxing 的 `image` 特性锁定了旧版 image crate，这里只用其亮度数组接口，图像转换自行完成。

use image::{GrayImage, Luma, RgbImage, imageops};
use rxing::{BarcodeFormat, EncodeHints, MultiFormatWriter, Writer};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum QrError {
    #[error("图片尺寸无效")]
    EmptyImage,
    #[error("生成失败：{0}")]
    Encode(String),
    #[error("不支持的码制：{0}")]
    UnsupportedFormat(String),
}

/// 识别前的图像预处理（默认全部关闭）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preprocess {
    /// 中值滤波窗口大小（奇数，0 或 1 为关闭）。
    pub median_filter_size: u32,
    /// 锐度系数，1.0 为不变，大于 1 锐化。
    pub sharpness_factor: f32,
    /// 对比度系数，1.0 为不变。
    pub contrast_factor: f32,
    /// 二值化阈值（0–255），0 为关闭。
    pub threshold: u8,
}

impl Default for Preprocess {
    fn default() -> Self {
        Self {
            median_filter_size: 0,
            sharpness_factor: 1.0,
            contrast_factor: 1.0,
            threshold: 0,
        }
    }
}

/// 一个识别结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Code {
    /// 码制，如 `QR_CODE`、`EAN_13`。
    pub format: String,
    pub text: String,
    /// 定位点（像素坐标）。QR 通常为 3–4 个点，一维码为 2 个点。
    pub points: Vec<[f32; 2]>,
}

/// 预处理并转为灰度图。
pub fn preprocess(img: &RgbImage, p: &Preprocess) -> GrayImage {
    let mut g: GrayImage = imageops::grayscale(img);
    if p.median_filter_size > 1 {
        let r = p.median_filter_size / 2;
        g = imageproc::filter::median_filter(&g, r, r);
    }
    if (p.sharpness_factor - 1.0).abs() > f32::EPSILON {
        // 同 PIL ImageEnhance.Sharpness 的思路：在原图与 3×3 平滑图之间线性插值 / 外推
        let blurred = imageproc::filter::box_filter(&g, 1, 1);
        let f = p.sharpness_factor;
        for (dst, b) in g.pixels_mut().zip(blurred.pixels()) {
            let v = b.0[0] as f32 + f * (dst.0[0] as f32 - b.0[0] as f32);
            dst.0[0] = v.round().clamp(0.0, 255.0) as u8;
        }
    }
    if (p.contrast_factor - 1.0).abs() > f32::EPSILON {
        let mean =
            g.pixels().map(|p| p.0[0] as f32).sum::<f32>() / (g.width() * g.height()).max(1) as f32;
        let f = p.contrast_factor;
        for px in g.pixels_mut() {
            px.0[0] = (mean + f * (px.0[0] as f32 - mean))
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    if p.threshold > 0 {
        for px in g.pixels_mut() {
            px.0[0] = if px.0[0] >= p.threshold { 255 } else { 0 };
        }
    }
    g
}

/// 识别图中所有码。没有找到时返回空列表。
pub fn read(img: &RgbImage, p: &Preprocess) -> Result<Vec<Code>, QrError> {
    if img.width() == 0 || img.height() == 0 {
        return Err(QrError::EmptyImage);
    }
    let g = preprocess(img, p);
    let (w, h) = g.dimensions();
    let results = rxing::helpers::detect_multiple_in_luma(g.into_raw(), w, h).unwrap_or_default();
    let mut codes: Vec<Code> = results
        .iter()
        .map(|r| Code {
            format: format!("{:?}", r.getBarcodeFormat()),
            text: r.getText().to_owned(),
            points: r.getPoints().iter().map(|pt| [pt.x, pt.y]).collect(),
        })
        .collect();
    // 同一个码可能被多个读取器重复报告
    codes.dedup_by(|a, b| a.text == b.text && a.format == b.format);
    Ok(codes)
}

/// 生成参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WriteOptions {
    /// 码制：`QR_CODE`、`DATA_MATRIX`、`AZTEC`、`PDF_417`、`CODE_128`、`CODE_39`、`EAN_13`、`EAN_8`、`UPC_A`、`ITF`。
    pub format: String,
    pub width: u32,
    pub height: u32,
    /// 静区（边距，模块数）。
    pub quiet_zone: u32,
    /// QR 纠错等级：`L` / `M` / `Q` / `H`。
    pub ec_level: String,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            format: "QR_CODE".into(),
            width: 256,
            height: 256,
            quiet_zone: 2,
            ec_level: "M".into(),
        }
    }
}

pub const WRITE_FORMATS: [&str; 10] = [
    "QR_CODE",
    "DATA_MATRIX",
    "AZTEC",
    "PDF_417",
    "CODE_128",
    "CODE_39",
    "EAN_13",
    "EAN_8",
    "UPC_A",
    "ITF",
];

fn parse_format(s: &str) -> Result<BarcodeFormat, QrError> {
    Ok(match s {
        "QR_CODE" => BarcodeFormat::QR_CODE,
        "DATA_MATRIX" => BarcodeFormat::DATA_MATRIX,
        "AZTEC" => BarcodeFormat::AZTEC,
        "PDF_417" => BarcodeFormat::PDF_417,
        "CODE_128" => BarcodeFormat::CODE_128,
        "CODE_39" => BarcodeFormat::CODE_39,
        "EAN_13" => BarcodeFormat::EAN_13,
        "EAN_8" => BarcodeFormat::EAN_8,
        "UPC_A" => BarcodeFormat::UPC_A,
        "ITF" => BarcodeFormat::ITF,
        other => return Err(QrError::UnsupportedFormat(other.into())),
    })
}

/// 由文本生成码图（黑白灰度图）。
pub fn write(text: &str, o: &WriteOptions) -> Result<GrayImage, QrError> {
    let format = parse_format(&o.format)?;
    // 一维码两侧需要足够的静区（约 10 个模块），否则多数扫码器无法识别
    let one_d = matches!(
        o.format.as_str(),
        "CODE_128" | "CODE_39" | "EAN_13" | "EAN_8" | "UPC_A" | "ITF"
    );
    let margin = if one_d {
        o.quiet_zone.max(10)
    } else {
        o.quiet_zone
    };
    let hints = EncodeHints {
        Margin: Some(margin.to_string()),
        ErrorCorrection: (format == BarcodeFormat::QR_CODE).then(|| o.ec_level.clone()),
        CharacterSet: Some("UTF-8".into()),
        ..Default::default()
    };
    let m = MultiFormatWriter
        .encode_with_hints(
            text,
            &format,
            o.width.max(1) as i32,
            o.height.max(1) as i32,
            &hints,
        )
        .map_err(|e| QrError::Encode(e.to_string()))?;
    let (w, h) = (m.getWidth(), m.getHeight());
    let mut img = GrayImage::from_pixel(w, h, Luma([255]));
    for y in 0..h {
        for x in 0..w {
            if m.get(x, y) {
                img.put_pixel(x, y, Luma([0]));
            }
        }
    }
    // 部分码制（如一维码）不会按请求高度伸缩，这里统一缩放到目标尺寸
    if (w, h) != (o.width, o.height) && o.width > 0 && o.height > 0 {
        img = imageops::resize(&img, o.width, o.height, imageops::FilterType::Nearest);
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb};

    fn to_rgb(g: &GrayImage) -> RgbImage {
        DynamicImage::ImageLuma8(g.clone()).to_rgb8()
    }

    #[test]
    fn qr_round_trip_with_unicode() {
        let img = write("AnyOCR 你好 https://example.com", &WriteOptions::default()).unwrap();
        assert_eq!(img.dimensions(), (256, 256));
        let codes = read(&to_rgb(&img), &Preprocess::default()).unwrap();
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].format, "QR_CODE");
        assert_eq!(codes[0].text, "AnyOCR 你好 https://example.com");
        assert!(codes[0].points.len() >= 3);
    }

    #[test]
    fn other_formats_round_trip() {
        for (fmt, text, w, h) in [
            ("CODE_128", "ANYOCR-2026", 400, 120),
            ("EAN_13", "6901234567892", 400, 140),
            ("DATA_MATRIX", "dm test", 200, 200),
        ] {
            let o = WriteOptions {
                format: fmt.into(),
                width: w,
                height: h,
                ..Default::default()
            };
            let img = write(text, &o).unwrap();
            let codes = read(&to_rgb(&img), &Preprocess::default()).unwrap();
            assert!(
                codes.iter().any(|c| c.text == text && c.format == fmt),
                "{fmt}: {codes:?}"
            );
        }
    }

    #[test]
    fn multiple_codes_in_one_image() {
        let a = write("first", &WriteOptions::default()).unwrap();
        let b = write("second", &WriteOptions::default()).unwrap();
        let mut canvas = RgbImage::from_pixel(600, 300, Rgb([255, 255, 255]));
        imageops::overlay(&mut canvas, &to_rgb(&a), 10, 20);
        imageops::overlay(&mut canvas, &to_rgb(&b), 320, 20);
        let mut texts: Vec<String> = read(&canvas, &Preprocess::default())
            .unwrap()
            .into_iter()
            .map(|c| c.text)
            .collect();
        texts.sort();
        assert_eq!(texts, ["first", "second"]);
    }

    #[test]
    fn preprocessing_recovers_low_contrast_code() {
        let qr = write("low contrast", &WriteOptions::default()).unwrap();
        // 压缩到 110–150 的灰度范围，模拟褪色的打印
        let faded = RgbImage::from_fn(qr.width(), qr.height(), |x, y| {
            let v = if qr.get_pixel(x, y).0[0] < 128 {
                110
            } else {
                150
            };
            Rgb([v, v, v])
        });
        let p = Preprocess {
            threshold: 130,
            median_filter_size: 3,
            ..Default::default()
        };
        let codes = read(&faded, &p).unwrap();
        assert_eq!(codes.first().map(|c| c.text.as_str()), Some("low contrast"));
        // 预处理各项不会出错
        let all = Preprocess {
            median_filter_size: 3,
            sharpness_factor: 2.0,
            contrast_factor: 1.5,
            threshold: 0,
        };
        let _ = preprocess(&faded, &all);
    }

    #[test]
    fn unsupported_format() {
        let o = WriteOptions {
            format: "FOO".into(),
            ..Default::default()
        };
        assert!(matches!(write("x", &o), Err(QrError::UnsupportedFormat(_))));
    }
}
