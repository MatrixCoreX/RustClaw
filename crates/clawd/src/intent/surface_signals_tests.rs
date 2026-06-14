use super::{
    analyze_prompt_surface, extract_dotted_field_selector, extract_field_selector_mentions,
    inline_json_transform_request, prompt_contains_delivery_token_reference, InlineJsonShape,
    LocatorHintPromptShape, LocatorReplyPromptShape,
};

#[test]
fn detects_empty_prompt_as_default_signals() {
    let signals = analyze_prompt_surface("   ");
    assert_eq!(signals.token_count, 0);
    assert!(signals.inline_json_shape.is_none());
    assert!(signals.locator_hint_prompt_shape.is_none());
    assert!(signals.locator_reply_prompt_shape.is_none());
    assert!(!signals.has_explicit_path_or_url());
    assert!(!signals.has_concrete_locator_hint());
    assert!(!signals.is_structural_locator_only_reply());
    assert_eq!(signals.field_selector_count, 0);
    assert!(signals.filename_candidates.is_empty());
    assert!(!signals.has_delivery_token_reference());
}

#[test]
fn detects_inline_json_and_locator_shape() {
    let signals = analyze_prompt_surface("{\"path\":\"logs/clawd.log\"}");
    assert_eq!(signals.inline_json_shape, Some(InlineJsonShape::WholeValue));
    assert!(signals.has_concrete_locator_hint());
}

#[test]
fn detects_explicit_path_locator() {
    let signals = analyze_prompt_surface("读取 UI/package.json 里的 name 字段，只输出值");
    assert_eq!(
        signals.locator_hint_prompt_shape,
        Some(LocatorHintPromptShape::ExplicitPathOrUrl)
    );
    assert!(signals.has_explicit_path_or_url());
    assert!(signals.has_concrete_locator_hint());
    assert_eq!(signals.field_selector_count, 0);
    assert!(!signals.filename_candidates.is_empty());
}

#[test]
fn detects_multiple_english_filename_targets() {
    let signals = analyze_prompt_surface(
        "read the opening section of README.md, then read the opening section of AGENTS.md, and say in one short English sentence which one is for end users versus contributors",
    );

    assert_eq!(
        signals.filename_candidates_excluding_field_selectors(),
        vec!["README.md".to_string(), "AGENTS.md".to_string()]
    );
    assert!(signals.dotted_field_selector.is_none());
    assert!(signals.field_selector_mentions.is_empty());
}

#[test]
fn detects_locator_only_reply_shape() {
    let signals = analyze_prompt_surface("logs/model_io.log");
    assert_eq!(
        signals.locator_hint_prompt_shape,
        Some(LocatorHintPromptShape::ExplicitPathOrUrl)
    );
    assert_eq!(
        signals.locator_reply_prompt_shape,
        Some(LocatorReplyPromptShape::LocatorOnly)
    );
    assert!(signals.has_explicit_path_or_url());
    assert!(signals.is_structural_locator_only_reply());
}

#[test]
fn detects_embedded_json_payload() {
    let signals = analyze_prompt_surface(
        r#"sort this JSON array by score descending: [{"name":"alpha","score":7}]"#,
    );
    assert_eq!(
        signals.inline_json_shape,
        Some(InlineJsonShape::EmbeddedPayload)
    );
}

#[test]
fn inline_transform_requires_structured_request_or_csv_records() {
    assert!(inline_json_transform_request(
        r#"{"action":"transform_data","data":[{"name":"alpha","score":7}],"ops":[{"op":"sort","by":"score"}]}"#
    ));
    assert!(inline_json_transform_request(
        r#"{"skill":"transform","args":{"action":"transform_data","records":[{"name":"alpha","score":7}],"ops":["sort"]}}"#
    ));
    assert!(!inline_json_transform_request(
        r#"sort this JSON array by score descending: [{"name":"alpha","score":7}]"#
    ));
    assert!(!inline_json_transform_request(
        r#"统计这个 JSON 数组中对象数量，只输出数字：[{"x":1},{"x":2}]"#
    ));
    assert!(!inline_json_transform_request(
        r#"{"action":"read_field","path":"package.json","field_path":"name"}"#
    ));
    assert!(inline_json_transform_request(
        "render this CSV as a markdown table:\nname,score\nalpha,7\nbeta,9"
    ));
    assert!(inline_json_transform_request(
        "render this CSV as a markdown table:name,score\\nalpha,7\\nbeta,9"
    ));
    assert!(inline_json_transform_request(
        "这个 CSV 按 score 降序输出 markdown 表格：name,score\\nli,3\\nwang,8\\nzhao,5"
    ));
    assert!(!inline_json_transform_request(
        r#"Explain what this JSON represents without sorting it: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#
    ));
    assert!(!inline_json_transform_request(
        r#"解释这个 JSON 代表什么：[{"name":"alpha","score":7},{"name":"beta","score":12}]"#
    ));
}

#[test]
fn deictic_reference_comes_from_structured_state_patch_only() {
    let signals = analyze_prompt_surface(
        r#"{"state_patch":{"deictic_reference":{"target":"unresolved_prior_object"}}}"#,
    );
    assert!(signals.has_deictic_reference());

    let natural = analyze_prompt_surface("read that file");
    assert!(!natural.has_deictic_reference());
}

#[test]
fn extracts_dotted_field_selector_from_mixed_prompt() {
    let out = extract_dotted_field_selector(
        "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo，只输出值",
    )
    .expect("should find dotted field selector");
    assert_eq!(out, "tools.allow_sudo");
}

#[test]
fn ignores_path_tokens_when_extracting_dotted_field_selector() {
    let out = extract_dotted_field_selector("读取 /tmp/config.toml 只输出值");
    assert!(out.is_none());
}

#[test]
fn ignores_filename_tokens_when_extracting_dotted_field_selector() {
    let out = extract_dotted_field_selector("restart_clawd_latest.sh");
    assert!(out.is_none());
}

#[test]
fn keeps_filename_like_selector_when_field_context_is_present() {
    let out = extract_dotted_field_selector("读取 Cargo.toml 的 package.name，只输出值");
    assert_eq!(out.as_deref(), Some("package.name"));
}

#[test]
fn does_not_lift_filename_like_selector_from_language_context_only() {
    assert!(extract_dotted_field_selector("package.name 字段").is_none());
    assert!(extract_dotted_field_selector("package.name field").is_none());
}

#[test]
fn leaves_bare_field_selector_semantics_to_planner() {
    let out = extract_field_selector_mentions(
        "读 scripts/nl_tests/fixtures/device_local/package.json，告诉我 scripts 字段下都有哪些子键",
    );
    assert!(out.is_empty());
}

#[test]
fn extracts_multiple_field_selectors_in_order() {
    let out = extract_field_selector_mentions(
        "读取 /tmp/config.toml 里的 database.sqlite_path 和 tools.allow_sudo，告诉我两个字段的值",
    );
    assert_eq!(
        out,
        vec![
            "database.sqlite_path".to_string(),
            "tools.allow_sudo".to_string()
        ]
    );
}

#[test]
fn leaves_single_segment_field_after_locator_to_planner() {
    let out = extract_field_selector_mentions("去 package.json 里找 name，只把值给我");
    assert!(out.is_empty());
}

#[test]
fn leaves_single_segment_value_phrase_to_planner() {
    let out =
        extract_field_selector_mentions("go into package.json and return only the name value");
    assert!(out.is_empty());
}

#[test]
fn detects_delivery_token_reference_shape() {
    assert!(prompt_contains_delivery_token_reference(
        "再发一次 FILE:/tmp/example.txt"
    ));
    let signals = analyze_prompt_surface("再发一次 FILE:/tmp/example.txt");
    assert!(signals.has_delivery_token_reference());
}

#[test]
fn lifts_locator_target_pair_into_surface_signals() {
    let signals = analyze_prompt_surface("比较 Cargo.toml 和 Cargo.lock 哪个更大");
    assert_eq!(
        signals.locator_target_pair,
        Some(("Cargo.toml".to_string(), "Cargo.lock".to_string()))
    );
}

#[test]
fn locator_target_pair_splits_punctuation_suffix_after_path() {
    let signals = analyze_prompt_surface(
        "把 scripts/nl_tests/fixtures/device_local/docs 打包成 tmp/contract_matrix_docs_bundle.zip，并告诉我生成路径。",
    );
    assert_eq!(
        signals.locator_target_pair,
        Some((
            "scripts/nl_tests/fixtures/device_local/docs".to_string(),
            "tmp/contract_matrix_docs_bundle.zip".to_string()
        ))
    );
}

#[test]
fn locator_target_pair_ignores_contract_test_hint_metadata() {
    let signals = analyze_prompt_surface(concat!(
        "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。",
        "\n[CONTRACT_TEST_HINT]\n",
        "candidate_wrong_action_ref=fs_basic.write_text\n",
        "policy_expectation=runtime_must_reject_or_replace_disallowed_action\n",
        "[/CONTRACT_TEST_HINT]"
    ));
    assert_eq!(
        signals.locator_target_pair,
        Some((
            "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
            "tmp/contract_matrix_unpacked".to_string()
        ))
    );
    assert!(!signals
        .filename_candidates
        .iter()
        .any(|candidate| candidate.contains("candidate_wrong_action_ref")));
}

#[test]
fn locator_target_pair_ignores_dotted_version_numbers() {
    let signals = analyze_prompt_surface("Correction: not Python 3.10, use Python 3.11 instead");
    assert!(signals.locator_target_pair.is_none());
}

#[test]
fn dotted_version_numbers_are_not_field_or_filename_signals() {
    let signals = analyze_prompt_surface("Correction: mention Python 3.11, not Python 3.10.");
    assert_eq!(signals.field_selector_count, 0);
    assert!(signals.dotted_field_selector.is_none());
    assert!(signals.filename_candidates.is_empty());
}
