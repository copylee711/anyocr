//! 间隙·树·排序算法（GapTree Sort Algorithm）。
//!
//! 移植自 Umi-OCR `parser_tools/gap_tree.py`（作者 hiroi-sora，MIT），
//! 原始项目 <https://github.com/hiroi-sora/GapTree_Sort_Algorithm>。
//!
//! 思路：自上而下把文本块分成“行”；每行中块与块之间的空白为“间隙”；
//! 多个连续行在同一位置的间隙连成“竖切线”；竖切线把页面切成若干区块（节点），
//! 节点构成布局树，前序遍历即为阅读顺序。

use super::cmp_f32;

const EPS: f32 = 0.0001;

/// 排序结果，元素均为输入 bbox 的下标。
pub(crate) struct GapTreeResult {
    /// 按阅读顺序排列的全部下标。
    pub order: Vec<usize>,
    /// 以区块为单位的下标分组（同样按阅读顺序）。
    pub nodes: Vec<Vec<usize>>,
}

/// 单元：(bbox, 原始下标)。
type Unit = ([f32; 4], usize);

struct Node {
    x_left: f32,
    x_right: f32,
    r_top: i64,
    r_bottom: i64,
    units: Vec<Unit>,
    children: Vec<usize>,
}

pub(crate) fn sort(bboxes: &[[f32; 4]]) -> GapTreeResult {
    if bboxes.is_empty() {
        return GapTreeResult {
            order: vec![],
            nodes: vec![],
        };
    }
    let (units, page_l, page_r) = get_units(bboxes);
    let (cuts, rows) = get_cuts_rows(&units, page_l, page_r);
    let (arena, root) = get_layout_tree(&cuts, &rows);
    let seq = preorder(&arena, root);
    let mut order = Vec::with_capacity(bboxes.len());
    let mut nodes = Vec::new();
    for id in seq {
        let n = &arena[id];
        if !n.units.is_empty() {
            let g: Vec<usize> = n.units.iter().map(|u| u.1).collect();
            order.extend_from_slice(&g);
            nodes.push(g);
        }
    }
    GapTreeResult { order, nodes }
}

fn get_units(bboxes: &[[f32; 4]]) -> (Vec<Unit>, f32, f32) {
    let mut page_l = f32::INFINITY;
    let mut page_r = -1.0f32;
    let mut units: Vec<Unit> = bboxes
        .iter()
        .enumerate()
        .map(|(i, b)| {
            page_l = page_l.min(b[0]);
            page_r = page_r.max(b[2]);
            (*b, i)
        })
        .collect();
    units.sort_by(|a, b| cmp_f32(a.0[1], b.0[1]));
    (units, page_l, page_r)
}

/// 竖切线 (左边缘x, 右边缘x, 起始行号, 结束行号)。
type Cut = (f32, f32, i64, i64);
/// 考察中的间隙 (左边缘x, 右边缘x, 开始行号)。
type Gap = (f32, f32, i64);

fn update_gaps(gaps1: &[Gap], gaps2: &[Gap]) -> (Vec<Gap>, Vec<Gap>) {
    let mut keep1 = vec![false; gaps1.len()];
    let mut used2 = vec![false; gaps2.len()];
    let mut new_gaps = Vec::new();
    for (i1, g1) in gaps1.iter().enumerate() {
        for (i2, g2) in gaps2.iter().enumerate() {
            let l = g1.0.max(g2.0);
            let r = g1.1.min(g2.1);
            if l <= r {
                new_gaps.push((l, r, g1.2));
                keep1[i1] = true;
                used2[i2] = true;
            }
        }
    }
    for (i2, g2) in gaps2.iter().enumerate() {
        if !used2[i2] {
            new_gaps.push(*g2);
        }
    }
    let removed = gaps1
        .iter()
        .enumerate()
        .filter(|(i, _)| !keep1[*i])
        .map(|(_, g)| *g)
        .collect();
    (new_gaps, removed)
}

fn get_cuts_rows(units: &[Unit], page_l: f32, page_r: f32) -> (Vec<Cut>, Vec<Vec<Unit>>) {
    let page_l = page_l - 1.0;
    let page_r = page_r + 1.0;
    let mut rows: Vec<Vec<Unit>> = Vec::new();
    let mut completed: Vec<Cut> = Vec::new();
    let mut gaps: Vec<Gap> = Vec::new();
    let mut row_index: i64 = 0;
    let mut ui = 0usize;
    while ui < units.len() {
        let unit = units[ui];
        let u_bottom = unit.0[3];
        let mut row = vec![unit];
        for (i, next) in units.iter().enumerate().skip(ui + 1) {
            if next.0[1] > u_bottom {
                break;
            }
            row.push(*next);
            ui = i;
        }
        row.sort_by(|a, b| cmp_f32(a.0[0], b.0[0]).then(cmp_f32(a.0[2], b.0[2])));
        let mut row_gaps: Vec<Gap> = Vec::new();
        let mut start = page_l;
        for u in &row {
            let (l, r) = (u.0[0], u.0[2]);
            if l > start {
                row_gaps.push((start, l, row_index));
            }
            if r > start {
                start = r;
            }
        }
        row_gaps.push((start, page_r, row_index));
        let (ng, removed) = update_gaps(&gaps, &row_gaps);
        gaps = ng;
        for g in removed {
            completed.push((g.0, g.1, g.2, row_index - 1));
        }
        rows.push(row);
        ui += 1;
        row_index += 1;
    }
    let row_max = rows.len() as i64 - 1;
    for g in gaps {
        completed.push((g.0, g.1, g.2, row_max));
    }
    completed.sort_by(|a, b| cmp_f32(a.0, b.0));
    (completed, rows)
}

fn get_layout_tree(cuts: &[Cut], rows: &[Vec<Unit>]) -> (Vec<Node>, usize) {
    let mut rows_gaps: Vec<Vec<(f32, f32)>> = vec![Vec::new(); rows.len()];
    for c in cuts {
        for r in c.2.max(0)..=c.3 {
            if let Some(v) = rows_gaps.get_mut(r as usize) {
                v.push((c.0, c.1));
            }
        }
    }
    let mut arena: Vec<Node> = vec![Node {
        x_left: cuts.first().map_or(0.0, |c| c.0) - 1.0,
        x_right: cuts.last().map_or(0.0, |c| c.1) + 1.0,
        r_top: -1,
        r_bottom: -1,
        units: vec![],
        children: vec![],
    }];
    let root = 0usize;
    let mut completed: Vec<usize> = vec![root];
    let mut now: Vec<usize> = Vec::new();

    fn complete(arena: &mut [Node], completed: &mut Vec<usize>, id: usize) {
        let node_r = arena[id].x_right - 2.0;
        let r_top = arena[id].r_top;
        let mut best: Vec<usize> = Vec::new();
        let mut max_r: i64 = -2;
        for &c in completed.iter() {
            let cn = &arena[c];
            if node_r < cn.x_left || node_r > cn.x_right + EPS {
                continue;
            }
            if cn.r_bottom >= r_top {
                continue;
            }
            if cn.r_bottom > max_r {
                max_r = cn.r_bottom;
                best = vec![c];
            } else if cn.r_bottom == max_r {
                best.push(c);
            }
        }
        // 最低的候选中，取最右的作为父节点（并列时取先出现者）；根节点总是候选，兜底用根
        let mut parent = best.first().copied().unwrap_or(0);
        for &b in &best {
            if arena[b].x_right > arena[parent].x_right {
                parent = b;
            }
        }
        arena[parent].children.push(id);
        completed.push(id);
    }

    for (r_i, row) in rows.iter().enumerate() {
        let r_i = r_i as i64;
        let row_gaps = &rows_gaps[r_i as usize];

        // 检查正在考虑的节点能否延续到本行
        let mut still = Vec::new();
        for id in std::mem::take(&mut now) {
            let (xl, xr) = (arena[id].x_left, arena[id].x_right);
            let (mut l_flag, mut r_flag, mut done) = (false, false, false);
            for g in row_gaps {
                if g.1 == xl {
                    l_flag = true;
                }
                if g.0 == xr {
                    r_flag = true;
                }
                if (xl < g.0 && g.0 < xr) || (xl < g.1 && g.1 < xr) {
                    done = true;
                    break;
                }
            }
            if !l_flag || !r_flag {
                done = true;
            }
            if done {
                complete(&mut arena, &mut completed, id);
            } else {
                arena[id].r_bottom = r_i;
                still.push(id);
            }
        }
        now = still;

        // 从左到右把文本块放入所在区间的节点
        let (mut u_i, mut g_i) = (0usize, 0usize);
        while u_i < row.len() {
            let unit = row[u_i];
            if g_i + 1 >= row_gaps.len() {
                break; // 理论上不会发生；防御性退出
            }
            let x_l = row_gaps[g_i].1;
            let x_r = row_gaps[g_i + 1].0;
            if unit.0[0] + EPS > x_r {
                g_i += 1;
                continue;
            }
            if let Some(&id) = now
                .iter()
                .find(|&&id| arena[id].x_left == x_l && arena[id].x_right == x_r)
            {
                arena[id].units.push(unit);
            } else {
                arena.push(Node {
                    x_left: x_l,
                    x_right: x_r,
                    r_top: r_i,
                    r_bottom: r_i,
                    units: vec![unit],
                    children: vec![],
                });
                now.push(arena.len() - 1);
            }
            u_i += 1;
        }
        // 防御：异常情况下未分配的块单独成节点，保证不丢字
        for unit in row.iter().skip(u_i) {
            arena.push(Node {
                x_left: unit.0[0],
                x_right: unit.0[2],
                r_top: r_i,
                r_bottom: r_i,
                units: vec![*unit],
                children: vec![],
            });
            let id = arena.len() - 1;
            complete(&mut arena, &mut completed, id);
        }
    }
    for id in now {
        complete(&mut arena, &mut completed, id);
    }
    for id in 0..arena.len() {
        let mut ch = std::mem::take(&mut arena[id].children);
        ch.sort_by(|a, b| cmp_f32(arena[*a].x_left, arena[*b].x_left));
        arena[id].children = ch;
        arena[id].units.sort_by(|a, b| cmp_f32(a.0[1], b.0[1]));
    }
    (arena, root)
}

fn preorder(arena: &[Node], root: usize) -> Vec<usize> {
    let mut stack = vec![root];
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        out.push(id);
        stack.extend(arena[id].children.iter().rev());
    }
    out
}
