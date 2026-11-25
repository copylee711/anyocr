//! 段落分析器：对同一列区块内的文本块判断段落关系，预测每块的结尾分隔符。
//! 移植自 Umi-OCR `parser_tools/paragraph_parse.py`。

use super::cmp_f32;
use super::separator::word_separator;

/// 行高对比阈值。
const TH: f32 = 1.2;

/// 段落分析的输入单元。
pub(crate) struct ParaUnit {
    pub bbox: [f32; 4],
    pub first: char,
    pub last: char,
}

/// 返回与输入一一对应的结尾分隔符。
pub(crate) fn paragraph_ends(units: &[ParaUnit]) -> Vec<&'static str> {
    let mut ends = vec!["\n"; units.len()];
    if units.is_empty() {
        return ends;
    }
    let mut idx: Vec<usize> = (0..units.len()).collect();
    idx.sort_by(|a, b| cmp_f32(units[*a].bbox[1], units[*b].bbox[1]));
    let bb = |i: usize| units[i].bbox;

    let [mut para_l, para_top, mut para_r, mut para_bottom] = bb(idx[0]);
    let mut para_h = para_bottom - para_top;
    let mut para_s: Option<f32> = None;
    let mut now = vec![idx[0]];
    let mut paras: Vec<Vec<usize>> = Vec::new();
    let mut spaces: Vec<Option<f32>> = Vec::new();

    for &i in &idx[1..] {
        let [l, top, r, bottom] = bb(i);
        let h = bottom - top;
        let ls = top - para_bottom;
        let same = (para_l - l).abs() <= para_h * TH
            && (para_r - r).abs() <= para_h * TH
            && para_s.is_none_or(|s| ls < s + para_h * 0.5);
        if same {
            para_l = (para_l + l) / 2.0;
            para_r = (para_r + r) / 2.0;
            para_h = (para_h + h) / 2.0;
            para_s = Some(para_s.map_or(ls, |s| (s + ls) / 2.0));
            now.push(i);
        } else {
            paras.push(std::mem::replace(&mut now, vec![i]));
            spaces.push(para_s);
            para_l = l;
            para_r = r;
            para_h = h;
            para_s = None;
        }
        para_bottom = bottom;
    }
    paras.push(now);
    spaces.push(para_s);

    // 只有一行的段，并入上一段末尾或下一段开头
    for i1 in (0..paras.len()).rev() {
        if paras[i1].len() != 1 {
            continue;
        }
        let [l, top, r, bottom] = bb(paras[i1][0]);
        let mut up = false;
        let mut down = false;
        let mut up_bottom = 0.0;
        let mut down_top = 0.0;
        if i1 > 0 {
            let [ul, ut, ur, ub] = bb(*paras[i1 - 1].last().unwrap_or(&paras[i1][0]));
            let uh = ub - ut;
            up_bottom = ub;
            up = (ul - l).abs() <= uh * TH && r <= ur + uh * TH;
            if let Some(s) = spaces[i1 - 1]
                && top - ub > s + uh * 0.5
            {
                up = false;
            }
        }
        if i1 + 1 < paras.len() {
            let [dl, dt, dr, db] = bb(paras[i1 + 1][0]);
            let dh = db - dt;
            down_top = dt;
            if dl - dh * TH <= l && l <= dl + dh * (1.0 + TH) {
                down = if paras[i1 + 1].len() > 1 {
                    (dr - r).abs() <= dh * TH
                } else {
                    dr - dh * TH < r
                };
            }
            if let Some(s) = spaces[i1 + 1]
                && dt - bottom > s + dh * 0.5
            {
                down = false;
            }
        }
        let item = paras[i1][0];
        let target_up = match (up, down) {
            (true, true) => Some(top - up_bottom < down_top - bottom),
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        };
        if let Some(to_up) = target_up {
            if to_up {
                paras[i1 - 1].push(item);
            } else {
                paras[i1 + 1].insert(0, item);
            }
            paras.remove(i1);
            spaces.remove(i1);
        }
    }

    for para in &paras {
        for w in para.windows(2) {
            ends[w[0]] = word_separator(units[w[0]].last, units[w[1]].first);
        }
        if let Some(&last) = para.last() {
            ends[last] = "\n";
        }
    }
    ends
}
