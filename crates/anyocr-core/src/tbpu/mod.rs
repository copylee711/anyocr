//! tbpu：text block processing unit，文本块后处理（排版解析）。
//!
//! 移植自 Umi-OCR `UmiOCR-data/py_src/ocr/tbpu/`（MIT，Copyright (c) 2023 hiroi-sora），
//! 其中间隙树排序算法源自 <https://github.com/hiroi-sora/GapTree_Sort_Algorithm>。
//!
//! 输入：引擎输出的无序文本块。输出：按阅读顺序排序的文本块，并为每块设置结尾分隔符 `end`。

mod gap_tree;
mod ignore;
mod multi;
mod paragraph;
mod preprocess;
mod separator;
mod single;

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::types::TextBlock;

pub use ignore::ignore_areas;
pub use separator::word_separator;

/// 排版解析方案，与 Umi-OCR 的取值一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Parser {
    /// 多栏-按自然段换行。
    #[default]
    MultiPara,
    /// 多栏-总是换行。
    MultiLine,
    /// 多栏-无换行。
    MultiNone,
    /// 单栏-按自然段换行。
    SinglePara,
    /// 单栏-总是换行。
    SingleLine,
    /// 单栏-无换行。
    SingleNone,
    /// 单栏-保留缩进（代码段）。
    SingleCode,
    /// 不做处理。
    None,
}

impl Parser {
    pub const ALL: [Parser; 8] = [
        Parser::MultiPara,
        Parser::MultiLine,
        Parser::MultiNone,
        Parser::SinglePara,
        Parser::SingleLine,
        Parser::SingleNone,
        Parser::SingleCode,
        Parser::None,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Parser::MultiPara => "multi_para",
            Parser::MultiLine => "multi_line",
            Parser::MultiNone => "multi_none",
            Parser::SinglePara => "single_para",
            Parser::SingleLine => "single_line",
            Parser::SingleNone => "single_none",
            Parser::SingleCode => "single_code",
            Parser::None => "none",
        }
    }

    /// 执行排版解析。
    pub fn run(self, blocks: Vec<TextBlock>) -> Vec<TextBlock> {
        match self {
            Parser::None => {
                // 保留引擎给出的顺序与分隔符
                blocks
            }
            Parser::MultiPara => multi::multi_para(blocks),
            Parser::MultiLine => multi::multi_line(blocks),
            Parser::MultiNone => multi::multi_none(blocks),
            Parser::SinglePara => single::single_para(blocks),
            Parser::SingleLine => single::single_line(blocks),
            Parser::SingleNone => single::single_none(blocks),
            Parser::SingleCode => single::single_code(blocks),
        }
    }
}

impl FromStr for Parser {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Parser::ALL
            .into_iter()
            .find(|p| p.key() == s)
            .ok_or_else(|| format!("未知的排版解析方案：{s}"))
    }
}

/// 工作单元：文本块 + 标准化外接框 `(x0, y0, x1, y1)`。
#[derive(Debug, Clone)]
pub(crate) struct Tb {
    pub block: TextBlock,
    pub nb: [f32; 4],
}

impl Tb {
    pub fn first_char(&self) -> char {
        self.block.text.chars().next().unwrap_or(' ')
    }
    pub fn last_char(&self) -> char {
        self.block.text.chars().last().unwrap_or(' ')
    }
}

pub(crate) fn into_blocks(tbs: Vec<Tb>) -> Vec<TextBlock> {
    tbs.into_iter().map(|t| t.block).collect()
}

/// `f32` 的全序比较（NaN 视为相等），用于排序。
pub(crate) fn cmp_f32(a: f32, b: f32) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

#[cfg(test)]
mod tests;
