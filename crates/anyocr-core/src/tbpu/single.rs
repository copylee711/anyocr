//! 单栏解析器。移植自 Umi-OCR `parser_single_line.py` / `parser_single_para.py` /
//! `parser_single_none.py` / `parser_single_code.py`。
//!
//! 与原实现的差异（修正原代码中的明显笔误）：
//! - `single_line` 计算平均行高时，原代码为 `(b1 + b2 - t1 - l2) * 0.5`，此处改为 `((b1 - t1) + (b2 - t2)) * 0.5`。
//! - `single_para` 合并整行外接框时，原代码 `b2 = max(b1, bb[2])` 等写错了变量，此处改为正确取最值。

use super::paragraph::{ParaUnit, paragraph_ends};
use super::preprocess::line_preprocessing;
use super::separator::word_separator;
use super::{Tb, cmp_f32};
use crate::types::TextBlock;

/// 把预处理后的文本块分成行。每行内从左到右，行与行从上到下；同时设置行内分隔符。
fn get_lines(mut tbs: Vec<Tb>) -> Vec<Vec<Tb>> {
    tbs.sort_by(|a, b| cmp_f32(a.nb[0], b.nb[0]));
    let mut slots: Vec<Option<Tb>> = tbs.into_iter().map(Some).collect();
    let mut lines: Vec<Vec<Tb>> = Vec::new();
    for i1 in 0..slots.len() {
        let Some(first) = slots[i1].take() else {
            continue;
        };
        let [_, top1, mut r1, bottom1] = first.nb;
        let h1 = bottom1 - top1;
        let mut line = vec![first];
        for slot in slots.iter_mut().skip(i1 + 1) {
            let Some(tb2) = slot.as_ref() else { continue };
            let [l2, top2, r2, bottom2] = tb2.nb;
            let h2 = bottom2 - top2;
            if l2 < r1 - h1 {
                continue; // 左侧与当前行末尾重叠太多
            }
            if top2 < top1 - h1 * 0.5 || bottom2 > bottom1 + h1 * 0.5 {
                continue; // 垂直方向不在同一行
            }
            if (h1 - h2).abs() > h1.min(h2) * 0.5 {
                continue; // 行高差异过大
            }
            r1 = r2;
            if let Some(tb2) = slot.take() {
                line.push(tb2);
            }
        }
        for i in 0..line.len() - 1 {
            let [_, t1, r1, b1] = line[i].nb;
            let [l2, t2, _, b2] = line[i + 1].nb;
            let h = ((b1 - t1) + (b2 - t2)) * 0.5;
            let end = if l2 - r1 > h * 1.5 {
                " " // 间隙太大，强制空格
            } else {
                word_separator(line[i].last_char(), line[i + 1].first_char())
            };
            line[i].block.end = end.into();
        }
        if let Some(last) = line.last_mut() {
            last.block.end = "\n".into();
        }
        lines.push(line);
    }
    lines.sort_by(|a, b| cmp_f32(a[0].nb[1], b[0].nb[1]));
    lines
}

pub(crate) fn single_line(blocks: Vec<TextBlock>) -> Vec<TextBlock> {
    get_lines(line_preprocessing(blocks))
        .into_iter()
        .flatten()
        .map(|t| t.block)
        .collect()
}

pub(crate) fn single_none(blocks: Vec<TextBlock>) -> Vec<TextBlock> {
    let mut out = single_line(blocks);
    for i in 0..out.len().saturating_sub(1) {
        if out[i].end == "\n" {
            let a = out[i].text.chars().last().unwrap_or(' ');
            let b = out[i + 1].text.chars().next().unwrap_or(' ');
            out[i].end = word_separator(a, b).into();
        }
    }
    out
}

pub(crate) fn single_para(blocks: Vec<TextBlock>) -> Vec<TextBlock> {
    let mut lines = get_lines(line_preprocessing(blocks));
    let units: Vec<ParaUnit> = lines
        .iter()
        .map(|line| {
            let mut bb = line[0].nb;
            for t in &line[1..] {
                bb[1] = bb[1].min(t.nb[1]);
                bb[2] = bb[2].max(t.nb[2]);
                bb[3] = bb[3].max(t.nb[3]);
            }
            ParaUnit {
                bbox: bb,
                first: line[0].first_char(),
                last: line[line.len() - 1].last_char(),
            }
        })
        .collect();
    let ends = paragraph_ends(&units);
    for (line, end) in lines.iter_mut().zip(ends) {
        if let Some(last) = line.last_mut() {
            last.block.end = end.into();
        }
    }
    lines.into_iter().flatten().map(|t| t.block).collect()
}

/// 把一行合并成一个文本块，块间按间距补空格。
fn merge_line(line: Vec<Tb>) -> TextBlock {
    let n = line.len() as f32;
    let mut it = line.into_iter();
    let mut a = it
        .next()
        .map(|t| t.block)
        .unwrap_or_else(|| TextBlock::from_rect("", [0.0; 4], 0.0));
    let mut ha = a.bbox[3][1] - a.bbox[0][1];
    let mut score = a.score;
    for t in it {
        let b = t.block;
        let (ba, bb) = (a.bbox, b.bbox);
        ha = (ha + bb[3][1] - bb[0][1]) / 2.0;
        let mut space = 0usize;
        if bb[0][0] > ba[1][0] && ha > 0.0 {
            space = ((bb[0][0] - ba[1][0]) / ha).round() as usize;
        }
        a.text.push_str(&"  ".repeat(space));
        a.text.push_str(&b.text);
        let y_top = ba[0][1].min(ba[1][1]).min(bb[0][1]).min(bb[1][1]);
        let y_bottom = ba[2][1].max(ba[3][1]).max(bb[2][1]).max(bb[3][1]);
        let x_left = ba[0][0].min(ba[3][0]).min(bb[0][0]).min(bb[3][0]);
        let x_right = ba[1][0].max(ba[2][0]).max(bb[1][0]).max(bb[2][0]);
        a.bbox = [
            [x_left, y_top],
            [x_right, y_top],
            [x_right, y_bottom],
            [x_left, y_bottom],
        ];
        score += b.score;
    }
    a.score = score / n;
    a.end = "\n".into();
    a
}

pub(crate) fn single_code(blocks: Vec<TextBlock>) -> Vec<TextBlock> {
    let lines = get_lines(line_preprocessing(blocks));
    let mut tbs: Vec<TextBlock> = lines.into_iter().map(merge_line).collect();
    if tbs.is_empty() {
        return tbs;
    }
    // 按行首 x 坐标计算缩进层级
    let lh = tbs.iter().map(|t| t.bbox[3][1] - t.bbox[0][1]).sum::<f32>() / tbs.len() as f32;
    let x_min = tbs
        .iter()
        .map(|t| t.bbox[0][0])
        .fold(f32::INFINITY, f32::min);
    let x_max = tbs
        .iter()
        .map(|t| t.bbox[0][0])
        .fold(f32::NEG_INFINITY, f32::max);
    let mut levels = Vec::new();
    if lh > 0.0 {
        let mut x = x_min;
        while x < x_max {
            levels.push(x);
            x += lh;
        }
    }
    for t in &mut tbs {
        let key = t.bbox[0][0] + lh / 2.0;
        // bisect_left - 1
        let level = levels.partition_point(|&v| v < key).saturating_sub(1);
        t.text = format!("{}{}", "  ".repeat(level), t.text);
        t.bbox[0][0] = x_min;
        t.bbox[3][0] = x_min;
    }
    tbs
}
