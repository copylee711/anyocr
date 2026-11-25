//! 按行预处理。移植自 Umi-OCR `parser_tools/line_preprocessing.py`。
//!
//! 估计整页文字的旋转角度；角度超过阈值时把所有框反向旋转到水平，再取外接矩形，
//! 得到标准化 bbox，最后按顶部 y 排序。

use std::f32::consts::PI;

use super::{Tb, cmp_f32};
use crate::types::{Quad, TextBlock};

const ANGLE_THRESHOLD_RAD: f32 = 3.0 * PI / 180.0;

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

fn box_angle(b: &Quad) -> f32 {
    let w = dist(b[0], b[1]);
    let h = dist(b[1], b[2]);
    let mut a = if w < h {
        (b[2][1] - b[1][1]).atan2(b[2][0] - b[1][0])
    } else {
        (b[1][1] - b[0][1]).atan2(b[1][0] - b[0][0])
    };
    if a < -PI / 2.0 + ANGLE_THRESHOLD_RAD {
        a += PI;
    } else if a >= PI / 2.0 + ANGLE_THRESHOLD_RAD {
        a -= PI;
    }
    a
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| cmp_f32(*a, *b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// 去除空文本，计算标准化 bbox，并按顶部从上到下排序。
pub(crate) fn line_preprocessing(blocks: Vec<TextBlock>) -> Vec<Tb> {
    let blocks: Vec<TextBlock> = blocks.into_iter().filter(|b| !b.text.is_empty()).collect();
    let rotation = median(blocks.iter().map(|b| box_angle(&b.bbox)).collect());

    let mut tbs: Vec<Tb> = if rotation.abs() <= ANGLE_THRESHOLD_RAD {
        blocks
            .into_iter()
            .map(|b| {
                let nb = b.bounds();
                Tb { block: b, nb }
            })
            .collect()
    } else {
        let (s, c) = (-rotation).sin_cos();
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut tbs: Vec<Tb> = blocks
            .into_iter()
            .map(|b| {
                let mut nb = [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ];
                for [x, y] in b.bbox {
                    let rx = c * x - s * y;
                    let ry = s * x + c * y;
                    nb[0] = nb[0].min(rx);
                    nb[1] = nb[1].min(ry);
                    nb[2] = nb[2].max(rx);
                    nb[3] = nb[3].max(ry);
                }
                min_x = min_x.min(nb[0]);
                min_y = min_y.min(nb[1]);
                Tb { block: b, nb }
            })
            .collect();
        if min_x < 0.0 || min_y < 0.0 {
            for t in &mut tbs {
                t.nb = [
                    t.nb[0] - min_x,
                    t.nb[1] - min_y,
                    t.nb[2] - min_x,
                    t.nb[3] - min_y,
                ];
            }
        }
        tbs
    };
    tbs.sort_by(|a, b| cmp_f32(a.nb[1], b.nb[1]));
    tbs
}
