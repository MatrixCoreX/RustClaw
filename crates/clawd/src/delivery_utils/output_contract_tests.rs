use crate::{IntentOutputContract, OutputResponseShape, OutputSemanticKind};

use super::{enforce_output_contract, looks_like_delivery_locator_literal, take_first_sentence};

#[test]
fn delivery_locator_literal_accepts_hint_and_path_shapes() {
    assert!(looks_like_delivery_locator_literal(
        "README.md",
        "README.md"
    ));
    assert!(looks_like_delivery_locator_literal(
        "/tmp/report.md",
        "README.md"
    ));
    assert!(looks_like_delivery_locator_literal(
        "configs/config.toml",
        "configs/config.toml"
    ));
    assert!(looks_like_delivery_locator_literal("LICENSE", "LICENSE"));
}

#[test]
fn delivery_locator_literal_rejects_user_facing_sentences() {
    assert!(!looks_like_delivery_locator_literal(
        "未找到该文件。文件 `definitely_missing_named_file_rustclaw_001.txt` 在工作区中不存在。",
        "definitely_missing_named_file_rustclaw_001.txt"
    ));
    assert!(!looks_like_delivery_locator_literal(
        "LOCATOR_CLARIFY_PROMPT",
        "README.md"
    ));
}

#[test]
fn one_sentence_contract_skips_leading_ordered_list() {
    let text = "最后5行日志：\n1. ts:1775000025, status:ok\n2. ts:1775000030, status:ok\n现象：所有任务执行成功，状态均为ok。";

    assert_eq!(
        take_first_sentence(text),
        "现象：所有任务执行成功，状态均为ok。"
    );
}

#[test]
fn content_excerpt_one_sentence_prefers_tail_summary_over_code_excerpt() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        ..Default::default()
    };
    let mut text = "脚本的前15行内容为：\n#!/usr/bin/env bash\nset -euo pipefail\n\n该脚本主要用于为重启 clawd 服务准备环境和运行目录。".to_string();
    let mut messages = vec![text.clone()];

    enforce_output_contract(
        &state,
        "读脚本并一句话说明",
        &contract,
        &mut text,
        &mut messages,
    );

    assert_eq!(text, "该脚本主要用于为重启 clawd 服务准备环境和运行目录。");
    assert_eq!(messages, vec![text.clone()]);
}

#[test]
fn content_evidence_one_sentence_prefers_tail_conclusion_over_inventory_first_line() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        ..Default::default()
    };
    let mut text = "logs 目录下与 clawd 相关的文件（26 个，按观察顺序）：\nclawd.run.log\nclawd.log\n\nclawd.run.log 最后 20 行均为 INFO task_call 流转。\n\n更像正常启动，没有遇到报错。".to_string();
    let mut messages = vec![text.clone()];

    enforce_output_contract(
        &state,
        "读日志并只用一句中文判断正常启动还是刚遇到报错",
        &contract,
        &mut text,
        &mut messages,
    );

    assert_eq!(text, "更像正常启动，没有遇到报错。");
    assert_eq!(messages, vec![text.clone()]);
}

#[test]
fn non_file_contract_strips_spurious_leading_file_label_from_prose() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        delivery_required: false,
        semantic_kind: OutputSemanticKind::None,
        ..Default::default()
    };
    let mut text =
        "FILE: RustClaw-介绍.md\n# RustClaw\nRustClaw 是一个本地智能体运行时。".to_string();
    let mut messages = vec![text.clone()];

    enforce_output_contract(
        &state,
        "帮我写一篇关于 RustClaw 的长文",
        &contract,
        &mut text,
        &mut messages,
    );

    assert!(!text.starts_with("FILE:"));
    assert!(text.starts_with("# RustClaw"));
    assert_eq!(messages, vec![text]);
}

#[test]
fn exact_sentence_count_overrides_mislabelled_one_sentence_contract() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: Some(3),
        response_shape: OutputResponseShape::OneSentence,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        ..Default::default()
    };
    let expected = "第一句概括背景。第二句说明重点。第三句给出结论。";
    let mut text = expected.to_string();
    let mut messages = vec![text.clone()];

    enforce_output_contract(
        &state,
        "读文档后用三句话概括重点",
        &contract,
        &mut text,
        &mut messages,
    );

    assert_eq!(text, expected);
    assert_eq!(messages, vec![expected.to_string()]);
}

#[test]
fn one_sentence_quantity_comparison_preserves_derived_ratio_line() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        semantic_kind: OutputSemanticKind::QuantityComparison,
        requires_content_evidence: true,
        ..Default::default()
    };
    let expected = "Cargo.lock size_bytes=121800; Cargo.toml size_bytes=2639.\nsize_ratio=46.15";
    let mut text = expected.to_string();
    let mut messages = vec![text.clone()];

    enforce_output_contract(
        &state,
        "compare Cargo.lock and Cargo.toml sizes",
        &contract,
        &mut text,
        &mut messages,
    );

    assert_eq!(text, expected);
    assert_eq!(messages, vec![expected.to_string()]);
}

#[test]
fn scalar_contract_extracts_single_machine_token_from_sentence() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..Default::default()
    };
    let mut text = "测试编号是 **minimax-small-20260429_201348**。".to_string();
    let mut messages = Vec::new();

    enforce_output_contract(&state, "只回答编号", &contract, &mut text, &mut messages);

    assert_eq!(text, "minimax-small-20260429_201348");
}

#[test]
fn scalar_contract_does_not_extract_delimited_path_from_context_sentence() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        locator_hint: "./NO_SUCH_RUSTCLAW_TEST_987654.txt".to_string(),
        ..Default::default()
    };
    let expected = "未找到 `./NO_SUCH_RUSTCLAW_TEST_987654.txt`，请确认路径后再继续。";
    let mut text = expected.to_string();
    let mut messages = Vec::new();

    enforce_output_contract(
        &state,
        "读取 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行",
        &contract,
        &mut text,
        &mut messages,
    );

    assert_eq!(text, expected);
    assert_eq!(messages, vec![expected.to_string()]);
}

#[test]
fn scalar_contract_preserves_natural_language_summary_with_single_ascii_token() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::None,
        ..Default::default()
    };
    let expected = "该测试验证 RustClaw 在连续会话下能否稳定保持上下文、记忆和状态。";
    let mut text = expected.to_string();
    let mut messages = Vec::new();

    enforce_output_contract(
        &state,
        "请用一句话总结这个连续会话测试主要验证什么。",
        &contract,
        &mut text,
        &mut messages,
    );

    assert_eq!(text, expected);
    assert_eq!(messages, vec![expected.to_string()]);
}

#[test]
fn scalar_count_contract_still_extracts_count_from_sentence() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        ..Default::default()
    };
    let mut text = "共有 3 个文件。".to_string();
    let mut messages = Vec::new();

    enforce_output_contract(&state, "只回答数量", &contract, &mut text, &mut messages);

    assert_eq!(text, "3");
    assert_eq!(messages, vec!["3".to_string()]);
}

#[test]
fn scalar_count_contract_does_not_extract_path_from_missing_result() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        ..Default::default()
    };
    let expected = "`configs/config_copy` 不存在，无法统计匹配项数量。";
    let mut text = expected.to_string();
    let mut messages = Vec::new();

    enforce_output_contract(&state, "只回答数量", &contract, &mut text, &mut messages);

    assert_eq!(text, expected);
    assert_eq!(messages, vec![expected.to_string()]);
}

#[test]
fn scalar_count_contract_ignores_digits_embedded_in_path_tokens() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        ..Default::default()
    };
    let expected = "`configs/config_copy_2026` 不存在，无法统计匹配项数量。";
    let mut text = expected.to_string();
    let mut messages = Vec::new();

    enforce_output_contract(&state, "只回答数量", &contract, &mut text, &mut messages);

    assert_eq!(text, expected);
    assert_eq!(messages, vec![expected.to_string()]);
}

#[test]
fn scalar_contract_preserves_missing_sentinel() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..Default::default()
    };
    let mut text = "<missing>".to_string();
    let mut messages = Vec::new();

    enforce_output_contract(&state, "只回答字段值", &contract, &mut text, &mut messages);

    assert_eq!(text, "<missing>");
}

#[test]
fn scalar_contract_preserves_structured_missing_field_line() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..Default::default()
    };
    let mut text = "package.name: <missing>".to_string();
    let mut messages = Vec::new();

    enforce_output_contract(&state, "只回答字段值", &contract, &mut text, &mut messages);

    assert_eq!(text, "package.name: <missing>");
}
