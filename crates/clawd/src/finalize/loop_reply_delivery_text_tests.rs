use super::delivery_text::{final_answer_text_from_delivery, single_publishable_delivery_message};

#[test]
fn final_answer_text_from_delivery_joins_publishable_chunks() {
    let delivery = vec![
        "**执行过程**\n1. 调用技能 `read_file`".to_string(),
        "第一部分内容。".to_string(),
        "第二部分内容。".to_string(),
    ];

    assert_eq!(
        final_answer_text_from_delivery(&delivery),
        "第一部分内容。\n\n第二部分内容。"
    );
}

#[test]
fn final_answer_text_from_delivery_ignores_minimax_tool_call_markup() {
    let delivery = vec![
        "我需要读取 Cargo.toml。\n<minimax:tool_call>\n<invoke name=\"fs_basic\"></invoke>\n</minimax:tool_call>".to_string(),
        "Cargo.toml 是 Rust 项目的包清单。".to_string(),
    ];

    assert_eq!(
        final_answer_text_from_delivery(&delivery),
        "Cargo.toml 是 Rust 项目的包清单。"
    );
}

#[test]
fn final_answer_text_from_delivery_ignores_non_answer_separator() {
    assert_eq!(
        final_answer_text_from_delivery(&["---SEPARATOR---".to_string()]),
        ""
    );
    assert_eq!(
        final_answer_text_from_delivery(&[
            "---SEPARATOR---".to_string(),
            "state=true can_poll=true".to_string(),
        ]),
        "state=true can_poll=true"
    );
    assert!(single_publishable_delivery_message(&["---SEPARATOR---".to_string()]).is_none());
}
