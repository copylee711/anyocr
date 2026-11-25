//! 多栏解析器。移植自 Umi-OCR `parser_multi_para.py` / `parser_multi_line.py` / `parser_multi_none.py`。

use super::paragraph::{ParaUnit, paragraph_ends};
use super::preprocess::line_preprocessing;
use super::separator::word_separator;
use super::{Tb, gap_tree};
use crate::types::TextBlock;

/// 预处理并按间隙树排序，返回 (排序后的单元, 以区块为单位的下标分组（指向排序后的单元）)。
fn sorted(blocks: Vec<TextBlock>) -> (Vec<Tb>, Vec<Vec<usize>>) {
    let tbs = line_preprocessing(blocks);
    let bboxes: Vec<[f32; 4]> = tbs.iter().map(|t| t.nb).collect();
    let res = gap_tree::sort(&bboxes);
    // 原下标 -> 排序后的新下标
    let mut new_pos = vec![0usize; tbs.len()];
    for (new_i, &old_i) in res.order.iter().enumerate() {
        new_pos[old_i] = new_i;
    }
    let mut slots: Vec<Option<Tb>> = tbs.into_iter().map(Some).collect();
    let out: Vec<Tb> = res
        .order
        .iter()
        .filter_map(|&old_i| slots[old_i].take())
        .collect();
    let nodes = res
        .nodes
        .into_iter()
        .map(|g| g.into_iter().map(|old_i| new_pos[old_i]).collect())
        .collect();
    (out, nodes)
}

pub(crate) fn multi_line(blocks: Vec<TextBlock>) -> Vec<TextBlock> {
    let (tbs, _) = sorted(blocks);
    tbs.into_iter()
        .map(|mut t| {
            t.block.end = "\n".into();
            t.block
        })
        .collect()
}

pub(crate) fn multi_none(blocks: Vec<TextBlock>) -> Vec<TextBlock> {
    let (mut tbs, _) = sorted(blocks);
    let n = tbs.len();
    for i in 0..n {
        let end = if i + 1 < n {
            word_separator(tbs[i].last_char(), tbs[i + 1].first_char())
        } else {
            "\n"
        };
        tbs[i].block.end = end.into();
    }
    super::into_blocks(tbs)
}

pub(crate) fn multi_para(blocks: Vec<TextBlock>) -> Vec<TextBlock> {
    let (mut tbs, nodes) = sorted(blocks);
    for group in nodes {
        let units: Vec<ParaUnit> = group
            .iter()
            .map(|&i| ParaUnit {
                bbox: tbs[i].nb,
                first: tbs[i].first_char(),
                last: tbs[i].last_char(),
            })
            .collect();
        let ends = paragraph_ends(&units);
        for (k, &i) in group.iter().enumerate() {
            tbs[i].block.end = ends[k].into();
        }
    }
    super::into_blocks(tbs)
}
