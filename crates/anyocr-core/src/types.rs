//! 统一数据模型。字段命名与 Umi-OCR 的 OCR 结果 JSON 保持兼容（`text`/`box`/`score`/`end`）。

use serde::{Deserialize, Serialize};

/// 图像坐标系下的点 `[x, y]`，原点在左上角，单位为像素。
pub type Point = [f32; 2];

/// 四边形文本框，顺序为：左上、右上、右下、左下。
pub type Quad = [Point; 4];

/// 文本块的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlockSource {
    /// OCR 引擎识别。
    #[default]
    Ocr,
    /// 从 PDF 原有文字层直接提取。
    Text,
    /// 经视觉大模型复核修改过。
    Reviewed,
}

impl BlockSource {
    fn is_default(&self) -> bool {
        *self == BlockSource::Ocr
    }
}

/// 一个文本块：OCR 返回的一行（或一段）文字及其位置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBlock {
    pub text: String,
    #[serde(rename = "box")]
    pub bbox: Quad,
    /// 置信度，0~1。
    pub score: f32,
    /// 排版解析得到的结尾分隔符：`"\n"`、`" "` 或 `""`。
    #[serde(default = "default_end")]
    pub end: String,
    #[serde(
        default,
        rename = "from",
        skip_serializing_if = "BlockSource::is_default"
    )]
    pub source: BlockSource,
}

fn default_end() -> String {
    "\n".to_owned()
}

impl TextBlock {
    pub fn new(text: impl Into<String>, bbox: Quad, score: f32) -> Self {
        Self {
            text: text.into(),
            bbox,
            score,
            end: default_end(),
            source: BlockSource::Ocr,
        }
    }

    /// 由轴对齐矩形 `(x0, y0, x1, y1)` 构造。
    pub fn from_rect(text: impl Into<String>, rect: [f32; 4], score: f32) -> Self {
        let [x0, y0, x1, y1] = rect;
        Self::new(text, [[x0, y0], [x1, y0], [x1, y1], [x0, y1]], score)
    }

    /// 外接轴对齐矩形 `(x0, y0, x1, y1)`。
    pub fn bounds(&self) -> [f32; 4] {
        quad_bounds(&self.bbox)
    }
}

/// 四边形的外接轴对齐矩形。
pub fn quad_bounds(q: &Quad) -> [f32; 4] {
    let mut r = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for [x, y] in q {
        r[0] = r[0].min(*x);
        r[1] = r[1].min(*y);
        r[2] = r[2].max(*x);
        r[3] = r[3].max(*y);
    }
    r
}

/// 一页（一张图）的识别结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PageResult {
    /// 识别所用图像的宽高（像素）。坐标都以此为基准。
    pub width: u32,
    pub height: u32,
    pub blocks: Vec<TextBlock>,
    /// 识别耗时（毫秒）。
    #[serde(default)]
    pub elapsed_ms: u64,
    /// 实际使用的引擎 id。
    #[serde(default)]
    pub engine: String,
}

impl PageResult {
    /// 按排版解析后的顺序与分隔符拼接全文。
    pub fn text(&self) -> String {
        let mut s = String::new();
        for b in &self.blocks {
            s.push_str(&b.text);
            s.push_str(&b.end);
        }
        s
    }

    /// 平均置信度；无文本块时为 0。
    pub fn mean_score(&self) -> f32 {
        if self.blocks.is_empty() {
            return 0.0;
        }
        self.blocks.iter().map(|b| b.score).sum::<f32>() / self.blocks.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_compatible_with_umi() {
        let json = r#"{"text":"文本111","box":[[29,19],[172,19],[172,44],[29,44]],"score":0.89}"#;
        let b: TextBlock = serde_json::from_str(json).unwrap();
        assert_eq!(b.text, "文本111");
        assert_eq!(b.end, "\n");
        assert_eq!(b.bounds(), [29.0, 19.0, 172.0, 44.0]);
        let out = serde_json::to_string(&b).unwrap();
        assert!(out.contains("\"box\""));
        assert!(!out.contains("\"from\""));
    }

    #[test]
    fn page_text_joins_with_end() {
        let mut a = TextBlock::from_rect("Hello", [0., 0., 10., 10.], 1.0);
        a.end = " ".into();
        let b = TextBlock::from_rect("world", [12., 0., 20., 10.], 0.5);
        let p = PageResult {
            blocks: vec![a, b],
            ..Default::default()
        };
        assert_eq!(p.text(), "Hello world\n");
        assert!((p.mean_score() - 0.75).abs() < 1e-6);
    }
}
