//! 忽略区域。移植自 Umi-OCR `tbpu/ignore_area.py`：完全落在任一忽略区域内的文本块被移除。

use crate::types::{Quad, TextBlock};

/// 移除完全位于任一区域（四边形，取其左上角与右下角）内的文本块。
pub fn ignore_areas(blocks: Vec<TextBlock>, areas: &[Quad]) -> Vec<TextBlock> {
    if areas.is_empty() {
        return blocks;
    }
    let inside = |a: &Quad, b: &Quad| {
        a[0][0] <= b[0][0] && a[0][1] <= b[0][1] && a[2][0] >= b[2][0] && a[2][1] >= b[2][1]
    };
    blocks
        .into_iter()
        .filter(|b| !areas.iter().any(|a| inside(a, &b.bbox)))
        .collect()
}
