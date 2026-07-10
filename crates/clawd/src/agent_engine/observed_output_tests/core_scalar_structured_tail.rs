#[test]
fn direct_scalar_defers_count_inventory_total_with_component_breakdown_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"count_inventory","counts":{"total":12,"files":9,"dirs":3}}"#,
    ));
    assert!(extract_direct_scalar_from_generic_output(&loop_state, None).is_none());
}

#[test]
fn direct_scalar_reads_count_inventory_single_dimension_from_structured_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","kind_filter":"file","counts":{"total":12,"files":9,"dirs":3}}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("9")
    );
}

#[test]
fn direct_count_inventory_uses_total_when_response_contract_is_known() {
    let value = serde_json::json!({
        "action": "count_inventory",
        "counts": {"total": 66, "files": 40, "dirs": 26},
        "path": ".",
        "recursive": false
    });

    assert!(super::count_inventory_direct_answer_candidate(None, &value, None, false,).is_none());

    assert_eq!(
        super::count_inventory_direct_answer_candidate(
            None,
            &value,
            Some(OutputResponseShape::Scalar),
            false,
        )
        .as_deref(),
        Some("66")
    );

    let one_sentence = super::count_inventory_direct_answer_candidate(
        None,
        &value,
        Some(OutputResponseShape::OneSentence),
        false,
    )
    .expect("one-sentence count answer");
    assert!(one_sentence.contains("66"));
}

#[test]
fn inventory_dir_grouped_contract_uses_names_by_kind() {
    let value = serde_json::json!({
        "action": "inventory_dir",
        "names_only": true,
        "names": ["Cargo.toml", "src", "README.md"],
        "names_by_kind": {
            "files": ["Cargo.toml", "README.md"],
            "dirs": ["src"],
            "other": []
        },
        "counts": {"files": 2, "dirs": 1, "total": 3}
    });
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;

    let answer = inventory_dir_direct_answer_candidate(None, Some(&route), &value, false)
        .expect("grouped inventory answer");

    assert!(answer.contains("目录:"));
    assert!(answer.contains("- src"));
    assert!(answer.contains("文件:"));
    assert!(answer.contains("- Cargo.toml"));
    assert!(answer.contains("- README.md"));
}

#[test]
fn inventory_dir_file_names_contract_filters_names_by_kind() {
    let value = serde_json::json!({
        "action": "inventory_dir",
        "names_only": true,
        "names": ["archive", "release_checklist.md", "service_notes.md"],
        "names_by_kind": {
            "files": ["release_checklist.md", "service_notes.md"],
            "dirs": ["archive"],
            "other": []
        },
        "counts": {"files": 2, "dirs": 1, "total": 3}
    });
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let answer = inventory_dir_direct_answer_candidate(None, Some(&route), &value, false)
        .expect("file names answer");

    assert!(answer.contains("release_checklist.md"));
    assert!(answer.contains("service_notes.md"));
    assert!(!answer.contains("archive"));
}

#[test]
fn inventory_dir_file_names_contract_accepts_route_marker_without_semantic_enum() {
    let value = serde_json::json!({
        "action": "inventory_dir",
        "names_only": true,
        "names": ["archive", "release_checklist.md", "service_notes.md"],
        "names_by_kind": {
            "files": ["release_checklist.md", "service_notes.md"],
            "dirs": ["archive"],
            "other": []
        },
        "counts": {"files": 2, "dirs": 1, "total": 3}
    });
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.route_reason = "contract:file_names".to_string();
    assert_eq!(route.output_contract.semantic_kind, OutputSemanticKind::None);

    let answer = inventory_dir_direct_answer_candidate(None, Some(&route), &value, false)
        .expect("file names marker answer");

    assert!(answer.contains("release_checklist.md"));
    assert!(answer.contains("service_notes.md"));
    assert!(!answer.contains("archive"));
}

#[test]
fn direct_answer_groups_inventory_dir_for_chat_wrapped_directory_entry_contract() {
    let mut loop_state = LoopState::new(1);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/root","names_only":false,"names":["docs","README.md"],"names_by_kind":{"files":["README.md"],"dirs":["docs"],"other":[]},"counts":{"files":1,"dirs":1,"total":2}}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    let context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&context))
        .expect("inventory_dir should produce grouped direct answer");

    assert!(answer.contains("目录:") || answer.contains("Directories:"));
    assert!(answer.contains("- docs"));
    assert!(answer.contains("文件:") || answer.contains("Files:"));
    assert!(answer.contains("- README.md"));
}

#[test]
fn tree_summary_direct_answer_lists_top_level_groups_without_false_truncation() {
    let value = serde_json::json!({
        "action": "tree_summary",
        "path": "/tmp/root",
        "resolved_path": "/tmp/root",
        "truncated_nodes": 0,
        "tree": {
            "kind": "dir",
            "path": "/tmp/root",
            "child_count": 3,
            "omitted_children": 0,
            "children": [
                {
                    "kind": "dir",
                    "path": "/tmp/root/configs",
                    "child_count": 1,
                    "omitted_children": 0,
                    "children": []
                },
                {
                    "kind": "file",
                    "path": "/tmp/root/package.json",
                    "size_bytes": 10
                },
                {
                    "kind": "dir",
                    "path": "/tmp/root/logs",
                    "child_count": 1,
                    "omitted_children": 0,
                    "children": []
                }
            ]
        }
    });

    let answer = tree_summary_direct_answer_candidate(None, &value, false).expect("answer");

    assert!(answer.contains("顶层结构"), "answer: {answer}");
    assert!(answer.contains("configs/"), "answer: {answer}");
    assert!(answer.contains("logs/"), "answer: {answer}");
    assert!(answer.contains("package.json"), "answer: {answer}");
    assert!(!answer.contains("未显示"), "answer: {answer}");
    assert!(!answer.contains("截断"), "answer: {answer}");
}

#[test]
fn tree_summary_direct_answer_prefers_machine_summary_rows() {
    let value = serde_json::json!({
        "action": "tree_summary",
        "summary_rows": [
            {
                "path": "scripts/nl_tests/fixtures/device_local",
                "name": "device_local",
                "kind": "dir",
                "file_count": 2,
                "truncated": false
            },
            {
                "path": "scripts/nl_tests/fixtures/device_local/docs",
                "name": "docs",
                "kind": "dir",
                "file_count": 2,
                "truncated": false
            }
        ],
        "tree": {
            "kind": "dir",
            "path": "scripts/nl_tests/fixtures/device_local",
            "children": []
        }
    });

    let answer = tree_summary_direct_answer_candidate(None, &value, false).expect("answer");

    assert!(answer.contains("name=device_local file_count=2 truncated=false"));
    assert!(answer.contains("name=docs file_count=2 truncated=false"));
    assert!(!answer.contains("顶层结构"), "answer: {answer}");
}

#[test]
fn dir_compare_direct_answer_reports_no_differences() {
    let value = serde_json::json!({
        "action": "dir_compare",
        "left_path": "tmp/bundle_src",
        "right_path": "tmp/dynamic_guard_unpack_case",
        "counts": {
            "left_only": 0,
            "right_only": 0,
            "kind_mismatches": 0,
            "common": 3
        }
    });

    let answer = dir_compare_direct_answer_candidate(None, &value, true).expect("answer");

    assert!(answer.contains("message_key=clawd.msg.dir_compare.observed"));
    assert!(answer.contains("reason_code=dir_compare_observed"));
    assert!(answer.contains("has_differences=false"));
    assert!(answer.contains("left_only_count=0"));
    assert!(answer.contains("right_only_count=0"));
    assert!(answer.contains("kind_mismatch_count=0"));
    assert!(!answer.contains("No differences"), "answer: {answer}");
    assert!(!answer.contains("未发现差异"), "answer: {answer}");
}

#[test]
fn direct_count_inventory_answer_uses_file_count_and_explanation_for_one_sentence() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":53,"files":53,"dirs":0},"kind_filter":"file","path":".","recursive":false}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "数一下当前目录一级有多少个普通文件，只告诉我数字和一句解释".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "scalar_count".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarCount,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        original_user_request: Some(
            "数一下当前目录一级有多少个普通文件，只告诉我数字和一句解释".to_string(),
        ),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("count_inventory should produce a direct count answer");

    assert!(answer.contains("53"));
    assert!(answer.contains("普通文件"));
    assert!(!answer.contains("无法计数"));
}
