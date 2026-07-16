use super::*;

#[test]
fn compare_paths_size_ratio_answer_computes_ratio_from_structured_output() {
    let answer = compare_paths_size_ratio_answer(
        r#"{"action":"compare_paths","left":{"path":"Cargo.lock","size_bytes":121647},"right":{"path":"Cargo.toml","size_bytes":2606},"comparison":{"same_size":false}}"#,
        false,
    )
    .expect("ratio answer");

    assert!(answer.contains("Cargo.lock"));
    assert!(answer.contains("Cargo.toml"));
    assert!(answer.contains("message_key=clawd.msg.quantity.size_comparison.observed"));
    assert!(answer.contains("reason_code=quantity_size_comparison_observed"));
    assert!(answer.contains("ratio=46.679586"));
}

#[test]
fn direct_quantity_compare_paths_preserves_existence_fields_when_contract_requires_metadata() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"compare_paths","comparison":{"same_path":false,"same_size":false,"size_delta_bytes":119},"field_value":{"left_exists":true,"right_exists":true,"same_path":false,"same_size":false,"size_delta_bytes":119},"left":{"exists":true,"kind":"file","path":"service_notes.md","size_bytes":272},"right":{"exists":true,"kind":"file","path":"release_checklist.md","size_bytes":153}},"text":"{}"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "service_notes.md | release_checklist.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, _summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "return same_path and existence fields",
        &loop_state,
        Some(&ctx),
    )
    .expect("existence metadata answer");

    assert_eq!(
        answer,
        "same_path=false\nleft_exists=true\nleft_kind=file\nright_exists=true\nright_kind=file"
    );
}

#[test]
fn direct_quantity_compare_paths_uses_output_contract_required_metadata_without_route_reason() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"compare_paths","comparison":{"same_path":false,"same_size":false,"size_delta_bytes":119},"field_value":{"left_exists":true,"right_exists":true,"same_path":false,"same_size":false,"size_delta_bytes":119},"left":{"exists":true,"kind":"file","path":"service_notes.md","size_bytes":272},"right":{"exists":true,"kind":"file","path":"release_checklist.md","size_bytes":153}},"text":"{\"action\":\"compare_paths\",\"comparison\":{\"same_path\":false,\"same_size\":false,\"size_delta_bytes\":119},\"field_value\":{\"left_exists\":true,\"right_exists\":true,\"same_path\":false,\"same_size\":false,\"size_delta_bytes\":119},\"left\":{\"exists\":true,\"kind\":\"file\",\"path\":\"service_notes.md\",\"size_bytes\":272},\"right\":{\"exists\":true,\"kind\":\"file\",\"path\":\"release_checklist.md\",\"size_bytes\":153}}"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.delivery_required = false;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, _summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "return same_path and existence fields",
        &loop_state,
        Some(&ctx),
    )
    .expect("existence metadata answer");

    assert_eq!(
        answer,
        "same_path=false\nleft_exists=true\nleft_kind=file\nright_exists=true\nright_kind=file"
    );
}

#[tokio::test]
async fn finalize_quantity_compare_paths_preserves_required_existence_fields() {
    let state = test_state();
    let task = claimed_task("task-quantity-compare-paths-existence-fields");
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.delivery_required = false;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "service_notes.md | release_checklist.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"compare_paths","comparison":{"same_path":false,"same_size":false,"size_delta_bytes":119},"field_value":{"left_exists":true,"right_exists":true,"same_path":false,"same_size":false,"size_delta_bytes":119},"left":{"exists":true,"kind":"file","path":"service_notes.md","size_bytes":272},"right":{"exists":true,"kind":"file","path":"release_checklist.md","size_bytes":153}},"text":"{}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "respond",
        r#"{"left":{"path":"service_notes.md","exists":true,"kind":"file","size_bytes":272},"right":{"path":"release_checklist.md","exists":true,"kind":"file","size_bytes":153},"same_path":false}"#,
    ));
    let prose_delivery =
        "same_path=false; both exist:\n- service_notes.md exists=true\n- release_checklist.md exists=true";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "respond", prose_delivery));
    loop_state
        .delivery_messages
        .push(prose_delivery.to_string());
    loop_state.last_user_visible_respond = Some(prose_delivery.to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "return same_path and both exist fields",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should preserve compare_paths existence fields");

    assert_eq!(
        reply.text,
        "same_path=false\nleft_exists=true\nleft_kind=file\nright_exists=true\nright_kind=file"
    );
}

#[tokio::test]
async fn finalize_one_sentence_quantity_compare_paths_projects_required_metadata() {
    let state = test_state();
    let task = claimed_task("task-one-sentence-quantity-compare-paths-metadata");
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.delivery_required = false;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"compare_paths","comparison":{"same_path":false,"same_size":false,"size_delta_bytes":119},"field_value":{"left_exists":true,"right_exists":true,"same_path":false,"same_size":false,"size_delta_bytes":119},"left":{"exists":true,"kind":"file","path":"service_notes.md","size_bytes":272},"right":{"exists":true,"kind":"file","path":"release_checklist.md","size_bytes":153}},"text":"{\"action\":\"compare_paths\",\"comparison\":{\"same_path\":false,\"same_size\":false,\"size_delta_bytes\":119},\"field_value\":{\"left_exists\":true,\"right_exists\":true,\"same_path\":false,\"same_size\":false,\"size_delta_bytes\":119},\"left\":{\"exists\":true,\"kind\":\"file\",\"path\":\"service_notes.md\",\"size_bytes\":272},\"right\":{\"exists\":true,\"kind\":\"file\",\"path\":\"release_checklist.md\",\"size_bytes\":153}}"}"#,
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "return same_path and both exist fields",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should project compare_paths metadata");

    assert_eq!(
        reply.text,
        "same_path=false\nleft_exists=true\nleft_kind=file\nright_exists=true\nright_kind=file"
    );
}

#[test]
fn path_batch_size_comparison_answer_picks_largest_structured_size() {
    let answer = path_batch_size_comparison_answer(
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","size_bytes":2606},"path":"Cargo.toml"},{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","size_bytes":121647},"path":"Cargo.lock"}]}"#,
        false,
    )
    .expect("size comparison answer");

    assert!(answer.contains("message_key=clawd.msg.quantity.size_comparison.observed"));
    assert!(answer.contains("reason_code=quantity_size_comparison_observed"));
    assert!(answer.contains("Cargo.lock"));
    assert!(answer.contains("largest.label=Cargo.lock"));
    assert!(answer.contains("runner_up.label=Cargo.toml"));
    assert!(answer.contains("delta_bytes=119041"));
    assert!(answer.contains("ratio=46.679586"));
}

#[test]
fn direct_quantity_comparison_from_count_inventory_prefers_total_size() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"dir","path":"target","resolved_path":"/tmp/repo/target","size_bytes":4096},"path":"target"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"count_inventory","path":"target","resolved_path":"/tmp/repo/target","recursive":true,"counts":{"total":129116,"files":100000,"dirs":29116,"total_size_bytes":57268736832}}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "target".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "看一下 target 大概多大",
        &loop_state,
        Some(&ctx),
    )
    .expect("count_inventory total size answer");

    assert!(answer.contains("57268736832"));
    assert!(answer.contains("53.3 GiB"));
    assert!(answer.contains("129116"));
    assert!(!answer.contains('\n'));
    assert!(answer.starts_with("path=target size.bytes=57268736832"));
    assert!(!answer.trim().eq("129116"));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn scalar_quantity_count_inventory_prefers_total_count_over_total_size_bytes() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"count_inventory","path":"document","recursive":false,"counts":{"total":47,"files":36,"dirs":11,"total_size_bytes":1190520}},"text":"{\"action\":\"count_inventory\",\"path\":\"document\",\"recursive\":false,\"counts\":{\"total\":47,\"files\":36,\"dirs\":11,\"total_size_bytes\":1190520}}"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "document".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_scalar_observed_answer(Some(&state), &loop_state, Some(&ctx))
        .expect("scalar count answer");

    assert_eq!(answer, "47");
    assert!(summary.contract_ok);
}

#[test]
fn direct_quantity_comparison_defers_two_count_inventory_totals_to_synthesis() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"count_inventory","path":"scripts/nl_tests/fixtures/device_local/docs","recursive":false,"counts":{"total":3,"files":2,"dirs":1,"total_size_bytes":425}},"text":"{\"action\":\"count_inventory\",\"path\":\"scripts/nl_tests/fixtures/device_local/docs\",\"recursive\":false,\"counts\":{\"total\":3,\"files\":2,\"dirs\":1,\"total_size_bytes\":425}}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"count_inventory","path":"scripts/nl_tests/fixtures/device_local/logs","recursive":false,"counts":{"total":2,"files":2,"dirs":0,"total_size_bytes":2698}},"text":"{\"action\":\"count_inventory\",\"path\":\"scripts/nl_tests/fixtures/device_local/logs\",\"recursive\":false,\"counts\":{\"total\":2,\"files\":2,\"dirs\":0,\"total_size_bytes\":2698}}"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let answer = direct_quantity_comparison_from_compare_paths(
        &state,
        "先数 docs 直接子项数量，再数 logs 直接子项数量，最后一句中文说哪个更多",
        &loop_state,
        Some(&ctx),
    );

    assert!(
        answer.is_none(),
        "multi-target count comparisons need synthesized language, got {answer:?}"
    );
}

#[test]
fn direct_quantity_comparison_from_ranked_inventory_outputs_name_size_lines() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"logs","resolved_path":"/tmp/repo/logs","sort_by":"size_desc","entries":[{"kind":"file","name":"large.log","size_bytes":900},{"kind":"file","name":"small.log","size_bytes":12}],"counts":{"files":2,"total":2}}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "列出 logs 目录下最大的 2 个文件",
        &loop_state,
        Some(&ctx),
    )
    .expect("ranked inventory answer");

    assert_eq!(answer, "large.log 900\nsmall.log 12");
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn matrix_strict_file_names_ranked_inventory_outputs_name_size_lines() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"logs","resolved_path":"/tmp/repo/logs","sort_by":"size_desc","entries":[{"kind":"file","name":"large.log","size_bytes":900},{"kind":"file","name":"small.log","size_bytes":12}],"counts":{"files":2,"total":2}}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs".to_string();

    let (answer, summary) = matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("ranked file-name inventory answer");

    assert_eq!(answer, "large.log 900\nsmall.log 12");
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn matrix_strict_file_names_ranked_inventory_applies_selector_limit() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"logs","resolved_path":"/tmp/repo/logs","sort_by":"size_desc","entries":[{"kind":"file","name":"large.log","size_bytes":900},{"kind":"file","name":"medium.log","size_bytes":120},{"kind":"file","name":"small.log","size_bytes":12}],"counts":{"files":3,"total":3}}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs".to_string();
    route.self_extension.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    route.self_extension.list_selector.limit = Some(2);
    route.self_extension.list_selector.sort_by = Some("size_desc".to_string());
    route.self_extension.list_selector.include_metadata = Some(true);

    let (answer, summary) = matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("ranked file-name inventory answer");

    assert_eq!(answer, "large.log 900\nmedium.log 120");
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn quantity_comparison_replaces_synthesis_count_with_total_size_answer() {
    let state = test_state();
    let task = claimed_task("task-quantity-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("129116，当前范围内共有 129116 个项目。".to_string());
    loop_state.last_user_visible_respond =
        Some("129116，当前范围内共有 129116 个项目。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","path":"target","resolved_path":"/tmp/repo/target","recursive":true,"counts":{"total":129116,"files":100000,"dirs":29116,"total_size_bytes":57268736832}}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "target".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        super::replace_delivery_with_deterministic_quantity_comparison_answer(
            &state,
            &task,
            "看一下 target 大概多大",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    let answer = loop_state.delivery_messages.join("\n");
    assert!(answer.contains("57268736832"));
    assert!(answer.contains("53.3 GiB"));
    assert!(!answer.contains('\n'));
    assert!(!answer.trim().starts_with("129116"));
    assert!(summary.is_some());
}

#[test]
fn quantity_comparison_preserves_synthesis_with_both_path_sizes() {
    let state = test_state();
    let task = claimed_task("task-quantity-preserve-synthesis");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    let synthesized = "README.md（46320 字节）比 README.zh-CN.md（39733 字节）更大。原因是英文文档通常比中文文档占用更多字节，同等内容的英文表达往往比中文更冗长。";
    loop_state.delivery_messages.push(synthesized.to_string());
    loop_state.last_user_visible_respond = Some(synthesized.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/tmp/repo/README.md","size_bytes":46320},"path":"README.md"},{"exists":true,"fact":{"kind":"file","path":"README.zh-CN.md","resolved_path":"/tmp/repo/README.zh-CN.md","size_bytes":39733},"path":"README.zh-CN.md"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        synthesized,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "README.md|README.zh-CN.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        super::replace_delivery_with_deterministic_quantity_comparison_answer(
            &state,
            &task,
            "比较 README.md 和 README.zh-CN.md 哪个更大，再解释原因",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![synthesized.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(synthesized)
    );
    assert!(summary.is_some());
}

#[test]
fn direct_quantity_comparison_from_compare_paths_recovers_after_synthesis_failure() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"compare_paths","left":{"path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","kind":"file","size_bytes":121647},"right":{"path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","kind":"file","size_bytes":2606},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":119041,"left_newer":false,"same_content":false}}"#,
    ));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("synthesis failed".to_string()),
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "Cargo.lock|Cargo.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured ratio fallback");

    assert!(answer.contains("ratio=46.679586"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_quantity_comparison_from_path_batch_facts_recovers_after_synthesis_failure() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","size_bytes":2606},"path":"Cargo.toml"},{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","size_bytes":121647},"path":"Cargo.lock"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("synthesis failed".to_string()),
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "Cargo.toml|Cargo.lock".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "比较 Cargo.toml 和 Cargo.lock 哪个更大，顺手用一句通俗话解释原因",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured path facts size fallback");

    assert!(answer.contains("Cargo.lock"));
    assert!(answer.contains("ratio=46.679586"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_quantity_comparison_scalar_shape_returns_ratio_not_byte_delta() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","size_bytes":121800},"path":"Cargo.lock"},{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","size_bytes":2639},"path":"Cargo.toml"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "Cargo.lock|Cargo.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured scalar ratio fallback");

    assert!(answer.contains("message_key=clawd.msg.quantity.size_comparison.observed"));
    assert!(answer.contains("reason_code=quantity_size_comparison_observed"));
    assert!(answer.contains("largest.label=Cargo.lock"));
    assert!(answer.contains("runner_up.label=Cargo.toml"));
    assert!(answer.contains("delta_bytes=119161"));
    assert!(answer.contains("46.15"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_quantity_comparison_unwraps_skill_success_envelope() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","size_bytes":121800},"path":"Cargo.lock"},{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","size_bytes":2639},"path":"Cargo.toml"}],"include_missing":true},"text":"{\"action\":\"path_batch_facts\",\"count\":2,\"facts\":[{\"exists\":true,\"fact\":{\"kind\":\"file\",\"path\":\"Cargo.lock\",\"resolved_path\":\"/tmp/Cargo.lock\",\"size_bytes\":121800},\"path\":\"Cargo.lock\"},{\"exists\":true,\"fact\":{\"kind\":\"file\",\"path\":\"Cargo.toml\",\"resolved_path\":\"/tmp/Cargo.toml\",\"size_bytes\":2639},\"path\":\"Cargo.toml\"}],\"include_missing\":true}"}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "Cargo.lock|Cargo.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, _) = direct_quantity_comparison_from_compare_paths(
        &state,
        "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍",
        &loop_state,
        Some(&ctx),
    )
    .expect("wrapped path facts ratio fallback");

    assert!(answer.contains("reason_code=quantity_size_comparison_observed"));
    assert!(answer.contains("ratio=46.153846"));
    assert!(!answer.contains(r#""action":"#));
}

#[test]
fn direct_quantity_comparison_uses_original_request_language_over_scaffold() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","size_bytes":121800},"path":"Cargo.lock"},{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","size_bytes":2639},"path":"Cargo.toml"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "Cargo.lock|Cargo.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        original_user_request: Some(
            "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍".to_string(),
        ),
        ..Default::default()
    };

    let (answer, _) = direct_quantity_comparison_from_compare_paths(
        &state,
        "### MEMORY_USE_POLICY\nCargo.lock Cargo.toml lock toml ratio",
        &loop_state,
        Some(&ctx),
    )
    .expect("contextual language fallback");

    assert!(answer.contains("reason_code=quantity_size_comparison_observed"));
    assert!(answer.contains("largest.label=Cargo.lock"));
    assert!(answer.contains("runner_up.label=Cargo.toml"));
    assert!(answer.contains("ratio=46.153846"));
}

#[test]
fn direct_quantity_comparison_strict_shape_returns_byte_delta() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/tmp/README.md","size_bytes":29191},"path":"README.md"},{"exists":true,"fact":{"kind":"file","path":"AGENTS.md","resolved_path":"/tmp/AGENTS.md","size_bytes":20744},"path":"AGENTS.md"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("synthesis failed".to_string()),
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "README.md|AGENTS.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "Compare README.md and AGENTS.md by file size.\n[CONTRACT_TEST_HINT]\nselector_answer_style=delta_only\n[/CONTRACT_TEST_HINT]",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured strict delta fallback");

    assert!(answer.contains("message_key=clawd.msg.quantity.size_comparison.observed"));
    assert!(answer.contains("reason_code=quantity_size_comparison_observed"));
    assert!(answer.contains("style=delta_only"));
    assert!(answer.contains("largest.label=README.md"));
    assert!(answer.contains("runner_up.label=AGENTS.md"));
    assert!(answer.contains("delta_bytes=8447"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_quantity_comparison_strict_shape_returns_json_by_default() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/tmp/README.md","size_bytes":29191},"path":"README.md"},{"exists":true,"fact":{"kind":"file","path":"AGENTS.md","resolved_path":"/tmp/AGENTS.md","size_bytes":20744},"path":"AGENTS.md"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "Compare README.md and AGENTS.md by size and return structured output.",
        &loop_state,
        Some(&ctx),
    )
    .expect("strict JSON fallback");

    assert_eq!(
        answer,
        r#"{"larger_file":"README.md","size_delta_bytes":8447}"#
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn strict_quantity_comparison_replacement_does_not_preserve_prose_delivery() {
    let state = test_state();
    let task = claimed_task("task-strict-quantity-json");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/tmp/README.md","size_bytes":29191},"path":"README.md"},{"exists":true,"fact":{"kind":"file","path":"AGENTS.md","resolved_path":"/tmp/AGENTS.md","size_bytes":20744},"path":"AGENTS.md"}],"include_missing":true}"#,
    ));
    loop_state
        .delivery_messages
        .push("README.md is larger: 29191 bytes; AGENTS.md is 20744 bytes.".to_string());
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_quantity_comparison_answer(
            &state,
            &task,
            "Compare README.md and AGENTS.md by size and return JSON.",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(
        loop_state.delivery_messages,
        vec![r#"{"larger_file":"README.md","size_delta_bytes":8447}"#.to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(r#"{"larger_file":"README.md","size_delta_bytes":8447}"#)
    );
    assert!(summary.is_some());
}

#[test]
fn free_quantity_comparison_replacement_waits_for_model_language_synthesis() {
    let state = test_state();
    let task = claimed_task("task-free-quantity-synthesis");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"compare_paths","left":{"kind":"file","path":"README.md","resolved_path":"/repo/README.md","size_bytes":46905},"right":{"kind":"file","path":"README.zh-CN.md","resolved_path":"/repo/README.zh-CN.md","size_bytes":40250},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":6655,"left_newer":false,"same_content":false}}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        !replace_delivery_with_deterministic_quantity_comparison_answer(
            &state,
            &task,
            "Compare README.md and README.zh-CN.md by size and include a bounded synthesis.",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.last_user_visible_respond.is_none());
    assert!(summary.is_none());
}

#[test]
fn strict_quantity_comparison_preserves_grounded_synthesis_with_grouped_sizes() {
    let state = test_state();
    let task = claimed_task("task-strict-quantity-synthesis");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/tmp/README.md","size_bytes":46905},"path":"README.md"},{"exists":true,"fact":{"kind":"file","path":"AGENTS.md","resolved_path":"/tmp/AGENTS.md","size_bytes":25181},"path":"AGENTS.md"}],"include_missing":true}"#,
    ));
    let answer = "README.md (46\u{202f}905 bytes) is about 1.9x larger than AGENTS.md (25\u{202f}181 bytes), which is normal for documentation with broader setup and usage coverage.".to_string();
    loop_state.last_publishable_synthesis_output = Some(answer.clone());
    loop_state.delivery_messages.push(answer.clone());
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_quantity_comparison_answer(
            &state,
            &task,
            "Compare README.md and AGENTS.md by size and explain the result in one sentence.",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![answer.clone()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer.as_str())
    );
    assert!(summary.is_some());
}

#[test]
fn quantity_comparison_preserves_grounded_respond_from_wrapped_compare_paths() {
    let state = test_state();
    let task = claimed_task("task-quantity-wrapped-respond");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"compare_paths","comparison":{"left_newer":true,"same_content":false,"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":21724},"left":{"kind":"file","modified_ts":1780529974,"path":"README.md","resolved_path":"/repo/README.md","size_bytes":46905},"right":{"kind":"file","modified_ts":1779962345,"path":"AGENTS.md","resolved_path":"/repo/AGENTS.md","size_bytes":25181}},"text":"{\"action\":\"compare_paths\",\"comparison\":{\"left_newer\":true,\"same_content\":false,\"same_kind\":true,\"same_name\":false,\"same_size\":false,\"size_delta_bytes\":21724},\"left\":{\"kind\":\"file\",\"modified_ts\":1780529974,\"path\":\"README.md\",\"resolved_path\":\"/repo/README.md\",\"size_bytes\":46905},\"right\":{\"kind\":\"file\",\"modified_ts\":1779962345,\"path\":\"AGENTS.md\",\"resolved_path\":\"/repo/AGENTS.md\",\"size_bytes\":25181}}"}"#,
    ));
    let answer = "README.md is about 1.9x larger (46,905 bytes vs 25,181 bytes), which is expected because it usually carries broader setup and usage context while AGENTS.md is narrower.".to_string();
    loop_state.delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer.clone());
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.exact_sentence_count = Some(1);
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_quantity_comparison_answer(
            &state,
            &task,
            "Compare README.md and AGENTS.md by size and explain the result in one sentence.",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![answer.clone()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer.as_str())
    );
    assert!(summary.is_some());
}

#[test]
fn quantity_comparison_preserves_model_answer_with_rounded_size_units() {
    let state = test_state();
    let task = claimed_task("task-quantity-rounded-units");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"compare_paths","comparison":{"left_newer":true,"same_content":false,"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":21724},"left":{"kind":"file","modified_ts":1780529974,"path":"README.md","resolved_path":"/repo/README.md","size_bytes":46905},"right":{"kind":"file","modified_ts":1779962345,"path":"AGENTS.md","resolved_path":"/repo/AGENTS.md","size_bytes":25181}},"text":"{\"action\":\"compare_paths\",\"comparison\":{\"left_newer\":true,\"same_content\":false,\"same_kind\":true,\"same_name\":false,\"same_size\":false,\"size_delta_bytes\":21724},\"left\":{\"kind\":\"file\",\"modified_ts\":1780529974,\"path\":\"README.md\",\"resolved_path\":\"/repo/README.md\",\"size_bytes\":46905},\"right\":{\"kind\":\"file\",\"modified_ts\":1779962345,\"path\":\"AGENTS.md\",\"resolved_path\":\"/repo/AGENTS.md\",\"size_bytes\":25181}}"}"#,
    ));
    let answer = "README.md (46.9 KB) is about 86% larger than AGENTS.md (25.2 KB), which is expected because README files often carry broader setup and usage context.";
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.last_user_visible_respond = Some(answer.to_string());
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut summary = None;

    assert!(
        replace_delivery_with_deterministic_quantity_comparison_answer(
            &state,
            &task,
            "Compare README.md and AGENTS.md by size and explain the result in one sentence.",
            &mut loop_state,
            Some(&ctx),
            &mut summary,
        )
    );

    assert_eq!(loop_state.delivery_messages, vec![answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer)
    );
    assert!(summary.is_some());
}

#[test]
fn quantity_comparison_preserves_structured_json_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/tmp/README.md","size_bytes":29191},"path":"README.md"},{"exists":true,"fact":{"kind":"file","path":"AGENTS.md","resolved_path":"/tmp/AGENTS.md","size_bytes":20744},"path":"AGENTS.md"}],"include_missing":true}"#,
    ));
    loop_state
        .delivery_messages
        .push(r#"{"larger_file":"README.md","size_delta_bytes":8447}"#.to_string());

    assert_eq!(
        latest_delivery_preserves_observed_quantity_size_facts(&loop_state).as_deref(),
        Some(r#"{"larger_file":"README.md","size_delta_bytes":8447}"#)
    );
}

#[test]
fn quantity_comparison_preserves_model_answer_with_grouped_sizes() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/tmp/README.md","size_bytes":46905},"path":"README.md"},{"exists":true,"fact":{"kind":"file","path":"AGENTS.md","resolved_path":"/tmp/AGENTS.md","size_bytes":25181},"path":"AGENTS.md"}],"include_missing":true}"#,
    ));
    let answer = "README.md (46\u{202f}905 bytes) is about 1.9x larger than AGENTS.md (25\u{202f}181 bytes).";
    loop_state.delivery_messages.push(answer.to_string());

    assert_eq!(
        latest_delivery_preserves_observed_quantity_size_facts(&loop_state).as_deref(),
        Some(answer)
    );
}

#[test]
fn direct_quantity_comparison_contract_selector_returns_larger_with_sizes() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"compare_paths","left":{"path":"release_checklist.md","resolved_path":"/tmp/release_checklist.md","kind":"file","size_bytes":153},"right":{"path":"package.json","resolved_path":"/tmp/package.json","kind":"file","size_bytes":246},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":-93,"left_newer":false,"same_content":false}}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, _summary) = direct_quantity_comparison_from_compare_paths(
        &state,
        "比较两个文件大小\n[CONTRACT_TEST_HINT]\nselector_answer_style=larger_with_sizes\n[/CONTRACT_TEST_HINT]",
        &loop_state,
        Some(&ctx),
    )
    .expect("selector should force complete comparison answer");

    assert!(answer.contains("package.json"), "answer: {answer}");
    assert!(answer.contains("246"), "answer: {answer}");
    assert!(answer.contains("release_checklist.md"), "answer: {answer}");
    assert!(answer.contains("153"), "answer: {answer}");
    assert!(
        !answer.contains("package.json：93 字节"),
        "answer: {answer}"
    );
}
