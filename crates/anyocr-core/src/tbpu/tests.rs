use super::*;
use crate::types::TextBlock;

fn tb(text: &str, x0: f32, y0: f32, x1: f32, y1: f32) -> TextBlock {
    TextBlock::from_rect(text, [x0, y0, x1, y1], 0.9)
}

fn texts(v: &[TextBlock]) -> Vec<&str> {
    v.iter().map(|b| b.text.as_str()).collect()
}

/// 标题横跨两栏，下面是左右两栏各 3 行。输入顺序故意打乱。
fn two_columns() -> Vec<TextBlock> {
    vec![
        tb("右2", 320.0, 90.0, 580.0, 110.0),
        tb("左1", 20.0, 60.0, 280.0, 80.0),
        tb("标题", 200.0, 10.0, 400.0, 40.0),
        tb("右1", 320.0, 60.0, 580.0, 80.0),
        tb("左3", 20.0, 120.0, 280.0, 140.0),
        tb("左2", 20.0, 90.0, 280.0, 110.0),
        tb("右3", 320.0, 120.0, 580.0, 140.0),
    ]
}

#[test]
fn multi_line_reads_columns_in_order() {
    let out = Parser::MultiLine.run(two_columns());
    assert_eq!(
        texts(&out),
        ["标题", "左1", "左2", "左3", "右1", "右2", "右3"]
    );
    assert!(out.iter().all(|b| b.end == "\n"));
}

#[test]
fn single_line_reads_rows() {
    let out = Parser::SingleLine.run(two_columns());
    assert_eq!(
        texts(&out),
        ["标题", "左1", "右1", "左2", "右2", "左3", "右3"]
    );
    // 同一行内两栏间隙很大，强制空格
    assert_eq!(out[1].end, " ");
    assert_eq!(out[2].end, "\n");
}

#[test]
fn multi_para_joins_lines_of_a_paragraph() {
    // 一栏：首行缩进的两段，每段 3 行
    let blocks = vec![
        tb("段一第一行", 40.0, 0.0, 400.0, 20.0),
        tb("段一第二行", 10.0, 30.0, 400.0, 50.0),
        tb("段一末行", 10.0, 60.0, 200.0, 80.0),
        tb("段二第一行", 40.0, 90.0, 400.0, 110.0),
        tb("段二第二行", 10.0, 120.0, 400.0, 140.0),
        tb("段二末行", 10.0, 150.0, 250.0, 170.0),
    ];
    let out = Parser::MultiPara.run(blocks);
    let text: String = out.iter().map(|b| format!("{}{}", b.text, b.end)).collect();
    assert_eq!(
        text,
        "段一第一行段一第二行段一末行\n段二第一行段二第二行段二末行\n"
    );
}

#[test]
fn single_para_english_uses_spaces() {
    let blocks = vec![
        tb("The quick brown", 10.0, 0.0, 400.0, 20.0),
        tb("fox jumps over", 10.0, 30.0, 400.0, 50.0),
        tb("the dog.", 10.0, 60.0, 150.0, 80.0),
    ];
    let out = Parser::SinglePara.run(blocks);
    let text: String = out.iter().map(|b| format!("{}{}", b.text, b.end)).collect();
    assert_eq!(text, "The quick brown fox jumps over the dog.\n");
}

#[test]
fn single_none_and_multi_none_have_no_newline_inside() {
    for p in [Parser::SingleNone, Parser::MultiNone] {
        let out = p.run(two_columns());
        let text: String = out.iter().map(|b| format!("{}{}", b.text, b.end)).collect();
        assert_eq!(text.matches('\n').count(), 1, "{p:?}: {text:?}");
    }
}

#[test]
fn single_code_keeps_indent() {
    // 层级点为 [x_min, x_min+行高, ...]，且不超过最大行首 x（与 Umi-OCR 一致）
    let blocks = vec![
        tb("fn main() {", 10.0, 0.0, 200.0, 20.0),
        tb("if a {", 30.0, 25.0, 200.0, 45.0),
        tb("b();", 50.0, 50.0, 200.0, 70.0),
        tb("}", 10.0, 75.0, 20.0, 95.0),
        tb("x", 90.0, 100.0, 120.0, 120.0),
    ];
    let out = Parser::SingleCode.run(blocks);
    assert_eq!(
        texts(&out),
        ["fn main() {", "  if a {", "    b();", "}", "      x"]
    );
}

#[test]
fn rotated_page_is_normalized() {
    // 整页顺时针旋转约 5°：两行，每行由左右两段组成
    let rot = |x: f32, y: f32| {
        let a = 5f32.to_radians();
        [x * a.cos() - y * a.sin() + 50.0, x * a.sin() + y * a.cos()]
    };
    let quad = |x0, y0, x1, y1| [rot(x0, y0), rot(x1, y0), rot(x1, y1), rot(x0, y1)];
    let blocks = vec![
        TextBlock::new("B", quad(110.0, 0.0, 200.0, 20.0), 0.9),
        TextBlock::new("D", quad(110.0, 40.0, 200.0, 60.0), 0.9),
        TextBlock::new("A", quad(0.0, 0.0, 100.0, 20.0), 0.9),
        TextBlock::new("C", quad(0.0, 40.0, 100.0, 60.0), 0.9),
    ];
    let out = Parser::SingleLine.run(blocks);
    assert_eq!(texts(&out), ["A", "B", "C", "D"]);
}

#[test]
fn none_parser_keeps_order() {
    let out = Parser::None.run(two_columns());
    assert_eq!(out[0].text, "右2");
}

#[test]
fn empty_and_blank_inputs() {
    for p in Parser::ALL {
        assert!(p.run(vec![]).is_empty());
    }
    let out = Parser::MultiPara.run(vec![
        tb("", 0.0, 0.0, 1.0, 1.0),
        tb("x", 0.0, 0.0, 10.0, 10.0),
    ]);
    assert_eq!(texts(&out), ["x"]);
}

#[test]
fn parser_keys_round_trip() {
    for p in Parser::ALL {
        assert_eq!(p.key().parse::<Parser>().unwrap(), p);
    }
    assert!("bogus".parse::<Parser>().is_err());
}

#[test]
fn ignore_area_removes_contained_blocks() {
    let area = [[0.0, 0.0], [450.0, 0.0], [450.0, 50.0], [0.0, 50.0]];
    let out = ignore_areas(two_columns(), &[area]);
    assert!(!texts(&out).contains(&"标题"));
    assert_eq!(out.len(), 6);
}
