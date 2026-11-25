//! 上下句间隔符。移植自 Umi-OCR `parser_tools/paragraph_parse.py::word_separator`。

use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF       // 中文
        | 0x3040..=0x30FF     // 日文
        | 0x1100..=0x11FF     // 韩文
        | 0x3130..=0x318F     // 韩文兼容字母
        | 0xAC00..=0xD7AF     // 韩文音节
        | 0x3000..=0x303F     // 中文符号和标点
        | 0xFE30..=0xFE4F     // 中文兼容形式标点
        | 0xFF00..=0xFFEF     // 半角和全角形式字符
    )
}

/// 传入前句尾字符和后句首字符，返回两句之间的分隔符（`""` 或 `" "`）。
pub fn word_separator(prev_last: char, next_first: char) -> &'static str {
    if is_cjk(prev_last) && is_cjk(next_first) {
        return "";
    }
    // 前文为连字符
    if prev_last == '-' {
        return "";
    }
    // 后文为任意标点符号
    if next_first.general_category_group() == GeneralCategoryGroup::Punctuation {
        return "";
    }
    " "
}

#[cfg(test)]
mod tests {
    use super::word_separator;

    #[test]
    fn separators() {
        assert_eq!(word_separator('中', '文'), "");
        assert_eq!(word_separator('a', 'b'), " ");
        assert_eq!(word_separator('中', 'b'), " ");
        assert_eq!(word_separator('-', 'b'), "");
        assert_eq!(word_separator('a', ','), "");
        assert_eq!(word_separator('。', '下'), "");
    }
}
