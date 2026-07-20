#[test]
fn observed_outputs_include_structured_run_cmd_error() {
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 128",
            "platform": "linux",
            "extra": {
                "command": "git -C /tmp status",
                "exit_code": 128,
                "exit_category": "terminated_by_signal_or_shell_status",
                "stderr": "fatal: not a git repository",
                "output_truncated": false
            }
        })
    );
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(error_step("step_1", "run_cmd", &err));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(has_observed_answer_candidates(&loop_state));
    assert!(joined.contains("skill(run_cmd)"), "entries: {joined}");
    assert!(
        joined.contains("execution_status:error"),
        "entries: {joined}"
    );
    assert!(
        joined.contains("fatal: not a git repository"),
        "entries: {joined}"
    );
}

#[test]
fn observed_outputs_exclude_synthesis_steps() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"line 1"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "synthesize_answer",
        "stale synthesized answer",
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "respond", "stale delivered answer"));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"line 2"}"#,
    ));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(joined.contains("line 1"), "entries: {joined}");
    assert!(joined.contains("line 2"), "entries: {joined}");
    assert!(
        !joined.contains("stale synthesized answer"),
        "entries: {joined}"
    );
    assert!(
        !joined.contains("stale delivered answer"),
        "entries: {joined}"
    );
}

#[test]
fn structured_field_selector_projects_scalar_from_any_capability_output() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "market_probe"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["market_probe"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.selection.structured_field_selector = Some("quote.price_usd".to_string());
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "market_probe",
        r#"{"quote":{"symbol":"BTC","price_usd":123.45}}"#,
    ));

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("123.45")
    );
}

#[test]
fn structured_field_selector_projects_scalar_from_capability_result_extra() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "system_probe"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["system_probe"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("manager".to_string());
    let agent_run_context = AgentRunContext {
        output_contract: Some(route),
        ..AgentRunContext::default()
    };
    let extra = serde_json::json!({
        "action": "detect",
        "manager": "apt-get",
    });
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_probe",
        r#"{"extra":{"action":"detect","manager":"apt-get"},"text":"untrusted fallback"}"#,
    ));
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "system_probe",
            "step_1",
            &serde_json::json!({"action": "detect"}),
            "manager=apt-get",
            Some(&extra),
        ));

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("apt-get")
    );
}

#[test]
fn config_read_uses_generic_capability_result_selector() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "config_basic"
        enabled = true
        kind = "builtin"
        semantic_tags = []
        "#,
        &["config_basic"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("value".to_string());
    let agent_run_context = AgentRunContext {
        output_contract: Some(route),
        ..AgentRunContext::default()
    };
    let extra = serde_json::json!({
        "action": "extract_field",
        "field_path": "llm.selected_vendor",
        "exists": true,
        "value": "minimax",
        "value_text": "minimax",
        "value_type": "string",
    });
    let mut loop_state = LoopState::new(2);
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "config_basic",
            "step_1",
            &serde_json::json!({"action": "read_field"}),
            "untrusted fallback",
            Some(&extra),
        ));

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("minimax")
    );
}

#[test]
fn config_mutation_exact_field_uses_generic_capability_result_selector() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "config_edit"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["config_edit"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("validated".to_string());
    let agent_run_context = AgentRunContext {
        output_contract: Some(route),
        ..AgentRunContext::default()
    };
    let extra = serde_json::json!({
        "action": "apply_config_change",
        "path": "configs/config.toml",
        "field_path": "skills.skill_switches.example",
        "old_value": null,
        "new_value": true,
        "applied": true,
        "validated": true,
    });
    let mut loop_state = LoopState::new(2);
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "config_edit",
            "step_1",
            &serde_json::json!({"action": "apply_config_change"}),
            "untrusted fallback",
            Some(&extra),
        ));

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("true")
    );
}

#[test]
fn document_title_uses_generic_capability_result_selector() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "system_basic"
        enabled = true
        kind = "builtin"
        semantic_tags = []
        "#,
        &["system_basic"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("title".to_string());
    let agent_run_context = AgentRunContext {
        output_contract: Some(route),
        ..AgentRunContext::default()
    };
    let extra = serde_json::json!({
        "action": "read_range",
        "path": "docs/service_notes.md",
        "field_selector": "title",
        "title": "Service Notes",
        "exists": true,
    });
    let mut loop_state = LoopState::new(2);
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "system_basic",
            "step_1",
            &serde_json::json!({
                "action": "read_range",
                "field_selector": "title"
            }),
            "untrusted fallback",
            Some(&extra),
        ));

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("Service Notes")
    );
}

#[test]
fn document_title_is_not_projected_without_explicit_selector() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "system_basic"
        enabled = true
        kind = "builtin"
        semantic_tags = []
        "#,
        &["system_basic"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        output_contract: Some(route),
        ..AgentRunContext::default()
    };
    let extra = serde_json::json!({
        "action": "read_range",
        "path": "docs/service_notes.md",
        "title": "Service Notes",
        "exists": true,
    });
    let mut loop_state = LoopState::new(2);
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "system_basic",
            "step_1",
            &serde_json::json!({"action": "read_range"}),
            "untrusted fallback",
            Some(&extra),
        ));

    assert!(extract_direct_scalar_from_generic_output_i18n(
        &loop_state,
        &state,
        Some(&agent_run_context)
    )
    .is_none());
}

#[test]
fn basename_projection_requires_explicit_generic_selector() {
    let result = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "system_basic",
        Some("path_batch_facts".to_string()),
        serde_json::json!({"extra": {"basename": "release_checklist.md"}}),
    );
    let results = vec![result];
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);

    assert!(selected_capability_result_scalar_candidate(Some(&route), &results).is_none());

    route.selection.structured_field_selector = Some("basename".to_string());
    assert_eq!(
        selected_capability_result_scalar_candidate(Some(&route), &results).as_deref(),
        Some("release_checklist.md")
    );
}

#[test]
fn grep_match_count_projection_requires_explicit_generic_selector() {
    let result = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "fs_basic",
        Some("grep_text".to_string()),
        serde_json::json!({
            "extra": {
                "query": "missing-token",
                "match_count": 0,
                "matches": []
            }
        }),
    );
    let results = vec![result];
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);

    assert!(selected_capability_result_scalar_candidate(Some(&route), &results).is_none());

    route.selection.structured_field_selector = Some("match_count".to_string());
    assert_eq!(
        selected_capability_result_scalar_candidate(Some(&route), &results).as_deref(),
        Some("0")
    );
}

#[test]
fn hidden_inventory_count_uses_explicit_generic_selector() {
    let result = claw_core::capability_result::CapabilityResultEnvelope::ok(
        "system_basic",
        Some("inventory_dir".to_string()),
        serde_json::json!({
            "extra": {
                "action": "inventory_dir",
                "include_hidden": true,
                "counts": {
                    "total": 3,
                    "hidden": 2
                },
                "names": [".git", ".env", "README.md"]
            }
        }),
    );
    let results = vec![result];
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);

    assert!(selected_capability_result_scalar_candidate(Some(&route), &results).is_none());

    route.selection.structured_field_selector = Some("counts.hidden".to_string());
    assert_eq!(
        selected_capability_result_scalar_candidate(Some(&route), &results).as_deref(),
        Some("2")
    );
}

#[test]
fn database_version_uses_generic_capability_result_selector() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "db_basic"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["db_basic"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("schema_version".to_string());
    let agent_run_context = AgentRunContext {
        output_contract: Some(route),
        ..AgentRunContext::default()
    };
    let extra = serde_json::json!({
        "action": "schema_version",
        "schema_version": 7,
    });
    let mut loop_state = LoopState::new(2);
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "db_basic",
            "step_1",
            &serde_json::json!({"action": "schema_version"}),
            "untrusted fallback",
            Some(&extra),
        ));

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("7")
    );
}

#[test]
fn database_table_list_uses_generic_exact_field_selector() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "db_basic"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["db_basic"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("tables".to_string());
    let agent_run_context = AgentRunContext {
        output_contract: Some(route),
        ..AgentRunContext::default()
    };
    let extra = serde_json::json!({
        "action": "list_tables",
        "table_count": 2,
        "tables": ["orders", "users"],
    });
    let wrapped = serde_json::json!({
        "extra": extra,
        "text": "untrusted fallback",
    })
    .to_string();
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "db_basic", &wrapped));
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "db_basic",
            "step_1",
            &serde_json::json!({"action": "list_tables"}),
            "untrusted fallback",
            Some(&extra),
        ));

    assert_eq!(
        extract_direct_answer_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        )
        .as_deref(),
        Some(r#"["orders","users"]"#)
    );
}

#[test]
fn git_fields_use_generic_capability_result_selectors() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "git_basic"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["git_basic"],
    );
    for (selector, expected, action, extra) in [
        (
            "current_branch",
            "main",
            "status",
            serde_json::json!({
                "action": "status",
                "current_branch": "main",
                "clean": false,
                "changed_count": 2
            }),
        ),
        (
            "clean",
            "false",
            "status",
            serde_json::json!({
                "action": "status",
                "current_branch": "main",
                "clean": false,
                "changed_count": 2
            }),
        ),
        (
            "subject",
            "refactor: simplify delivery",
            "log",
            serde_json::json!({
                "action": "log",
                "subject": "refactor: simplify delivery",
                "commit_count": 1
            }),
        ),
    ] {
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
        route.requires_content_evidence = true;
        route.selection.structured_field_selector = Some(selector.to_string());
        let agent_run_context = AgentRunContext {
            output_contract: Some(route),
            ..AgentRunContext::default()
        };
        let mut loop_state = LoopState::new(2);
        loop_state
            .capability_results
            .push(crate::capability_result::successful_execution_envelope(
                "git_basic",
                "step_1",
                &serde_json::json!({"action": action}),
                "untrusted fallback",
                Some(&extra),
            ));

        assert_eq!(
            extract_direct_scalar_from_generic_output_i18n(
                &loop_state,
                &state,
                Some(&agent_run_context)
            )
            .as_deref(),
            Some(expected),
            "selector={selector}"
        );
    }
}

#[test]
fn archive_members_use_generic_exact_field_selector() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "archive_basic"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["archive_basic"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("members".to_string());
    let agent_run_context = AgentRunContext {
        output_contract: Some(route),
        ..AgentRunContext::default()
    };
    let extra = serde_json::json!({
        "action": "list",
        "member_count": 2,
        "members": ["notes.txt", "nested/config.ini"],
    });
    let mut loop_state = LoopState::new(2);
    loop_state
        .capability_results
        .push(crate::capability_result::successful_execution_envelope(
            "archive_basic",
            "step_1",
            &serde_json::json!({"action": "list"}),
            "untrusted fallback",
            Some(&extra),
        ));

    assert_eq!(
        extract_direct_answer_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        )
        .as_deref(),
        Some(r#"["notes.txt","nested/config.ini"]"#)
    );
}

#[test]
fn archive_paths_use_generic_scalar_field_selector() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "archive_basic"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["archive_basic"],
    );
    for (selector, extra, expected) in [
        (
            "archive",
            serde_json::json!({"action": "pack", "archive": "/tmp/reports.zip"}),
            "/tmp/reports.zip",
        ),
        (
            "dest",
            serde_json::json!({"action": "unpack", "dest": "/tmp/reports"}),
            "/tmp/reports",
        ),
    ] {
        let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
        route.requires_content_evidence = true;
        route.selection.structured_field_selector = Some(selector.to_string());
        let agent_run_context = AgentRunContext {
            output_contract: Some(route),
            ..AgentRunContext::default()
        };
        let mut loop_state = LoopState::new(2);
        loop_state
            .capability_results
            .push(crate::capability_result::successful_execution_envelope(
                "archive_basic",
                "step_1",
                &serde_json::json!({"action": extra["action"]}),
                "untrusted fallback",
                Some(&extra),
            ));

        assert_eq!(
            extract_direct_scalar_from_generic_output_i18n(
                &loop_state,
                &state,
                Some(&agent_run_context)
            )
            .as_deref(),
            Some(expected),
            "{selector}"
        );
    }
}

#[test]
fn scalar_output_does_not_guess_an_unselected_structured_field() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "crypto"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["crypto"],
    );
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "crypto",
        r#"{"quote":{"symbol":"BTC","price_usd":123.45}}"#,
    ));

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        ),
        None
    );
}

#[test]
fn multi_count_observation_guard_lists_all_count_rows() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","path":"crates","resolved_path":"/repo/crates","recursive":false,"counts":{"total":13,"files":0,"dirs":13,"hidden":0}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"count_inventory","path":"crates/skills","resolved_path":"/repo/crates/skills","recursive":false,"counts":{"total":35,"files":0,"dirs":35,"hidden":0}}"#,
    ));
    let guard = multi_count_observation_guard_entry(&loop_state).expect("multi-count guard");

    assert!(
        guard.contains("delivery_constraint=cover_all_observed_count_rows"),
        "guard: {guard}"
    );
    assert!(guard.contains("observed_count_rows=2"), "guard: {guard}");
    assert!(
        guard.contains("observed_count.1.path=/repo/crates"),
        "guard: {guard}"
    );
    assert!(
        guard.contains("observed_count.1.count_total=13"),
        "guard: {guard}"
    );
    assert!(
        guard.contains("observed_count.2.path=/repo/crates/skills"),
        "guard: {guard}"
    );
    assert!(
        guard.contains("observed_count.2.count_total=35"),
        "guard: {guard}"
    );
}

#[test]
fn compound_listing_content_delivery_guard_lists_observed_names() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["archive","release_checklist.md","service_notes.md"]}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n3|1. Verify configuration loads correctly."}"#,
    ));
    let route = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);

    let guard = compound_listing_content_delivery_guard_entry(&loop_state, Some(&route))
        .expect("compound guard");

    assert!(guard.contains("current_task_observed_listing_names"));
    assert!(guard.contains("archive, release_checklist.md, service_notes.md"));
    assert!(guard.contains("current_task_observed_content_excerpt: present"));
}

#[test]
fn names_only_inventory_direct_answer_does_not_need_llm_synthesis() {
    let state = AppState::test_default_with_fixture_provider();
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: String::new(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                output_contract: None,
                steps: vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_capability".to_string(),
                        skill: "filesystem.list_names".to_string(),
                        args: serde_json::json!({
                            "path": "document",
                            "names_only": true,
                            "max_entries": 5,
                            "sort_by": "name",
                        }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "synthesize_answer".to_string(),
                        skill: String::new(),
                        args: serde_json::json!({}),
                        depends_on: vec!["step_1".to_string()],
                        why: String::new(),
                    },
                ],
                planner_notes: String::new(),
                plan_kind: crate::PlanKind::Single,
                raw_plan_text: String::new(),
            }),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names_only":true,"names":["full_suite_trace_note.txt","gen-1778122040.png","gen-1778122536.png","hello.sh","hello_from_manual_test.sh"],"names_by_kind":{"files":["full_suite_trace_note.txt","gen-1778122040.png","gen-1778122536.png","hello.sh","hello_from_manual_test.sh"],"dirs":[],"other":[]},"path":"document","resolved_path":"/workspace/document"}"#,
    ));

    assert_eq!(
        extract_direct_answer_from_generic_output_i18n(&loop_state, &state, Some(&agent_run_context))
            .as_deref(),
        Some(
            "full_suite_trace_note.txt\ngen-1778122040.png\ngen-1778122536.png\nhello.sh\nhello_from_manual_test.sh"
        )
    );
}

#[test]
fn names_only_inventory_free_shape_defers_to_llm_synthesis() {
    let state = AppState::test_default_with_fixture_provider();
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: String::new(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                output_contract: None,
                steps: vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_capability".to_string(),
                        skill: "filesystem.list_names".to_string(),
                        args: serde_json::json!({
                            "path": "logs",
                            "names_only": true,
                            "max_entries": 2,
                        }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "synthesize_answer".to_string(),
                        skill: String::new(),
                        args: serde_json::json!({}),
                        depends_on: vec!["step_1".to_string()],
                        why: String::new(),
                    },
                ],
                planner_notes: String::new(),
                plan_kind: crate::PlanKind::Single,
                raw_plan_text: String::new(),
            }),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names_only":true,"names":["clawd.run.log","model_io.log"],"path":"logs","resolved_path":"/workspace/logs"}"#,
    ));

    assert!(
        extract_direct_answer_from_generic_output_i18n(&loop_state, &state, Some(&agent_run_context))
            .is_none()
    );
}

#[test]
fn dirs_only_inventory_names_by_kind_can_direct_answer_observation_only_plan() {
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: String::new(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                output_contract: None,
                steps: vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_capability".to_string(),
                    skill: "filesystem.list_dir".to_string(),
                    args: serde_json::json!({
                        "path": "scripts/nl_tests/fixtures/device_local",
                        "dirs_only": true,
                        "names_only": true,
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }],
                planner_notes: String::new(),
                plan_kind: crate::PlanKind::Single,
                raw_plan_text: String::new(),
            }),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":5,"files":0,"hidden":0,"total":5},"dirs_only":true,"files_only":false,"names_by_kind":{"dirs":["configs","data","docs","logs","tmp"],"files":[],"other":[]},"path":"/repo/scripts/nl_tests/fixtures/device_local","resolved_path":"/repo/scripts/nl_tests/fixtures/device_local","sort_by":"name"},"text":"{}"}"#,
    ));

    assert_eq!(
        super::extract_answer_from_observed_output(&loop_state, Some(&agent_run_context))
            .as_deref(),
        Some("configs\ndata\ndocs\nlogs\ntmp")
    );
}

#[test]
fn observed_entries_project_wrapped_inventory_names_by_kind_files() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":0,"files":5,"hidden":0,"total":5},"dirs_only":false,"entries":[],"files_only":true,"include_hidden":false,"names_by_kind":{"dirs":[],"files":["act_plan.log","clawd-codex-current.log","clawd-codex-style-live.log","clawd-dev-live.log","clawd-dev.log"],"other":[]},"path":"logs","resolved_path":"/home/guagua/rustclaw/logs","size_summary":{"largest_file":{"kind":"file","name":"model_io.log.2026-07-09","path":"logs/model_io.log.2026-07-09","size_bytes":532246887},"matched_file_count":49,"smallest_file":{"kind":"file","name":"nl_delayed_minimax_retry_20260616_121155.log","path":"logs/nl_delayed_minimax_retry_20260616_121155.log","size_bytes":60},"total_file_size_bytes":2290871096},"sort_by":"name"}}"#,
    ));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(
        joined.contains("inventory_dir path=/home/guagua/rustclaw/logs sort_by=name total=5 files=5 dirs=0 hidden=0"),
        "entries: {joined}"
    );
    assert!(
        joined.contains("file_entries=act_plan.log,clawd-codex-current.log,clawd-codex-style-live.log,clawd-dev-live.log,clawd-dev.log"),
        "entries: {joined}"
    );
    assert!(
        joined.contains("size_summary.largest_file name=model_io.log.2026-07-09"),
        "entries: {joined}"
    );
}

#[test]
fn observed_outputs_keep_latest_content_read_for_same_path() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","resolved_path":"/tmp/model_io.log","excerpt":"old head evidence"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","resolved_path":"/tmp/model_io.log","excerpt":"new tail evidence"}"#,
    ));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(!joined.contains("old head evidence"), "entries: {joined}");
    assert!(joined.contains("new tail evidence"), "entries: {joined}");
}

fn chat_wrapped_unclassified_route(response_shape: OutputResponseShape) -> IntentOutputContract {
    IntentOutputContract {
            exact_sentence_count: None,
            response_shape,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/workspace/project".to_string(),
            selection: crate::OutputSelectionContract::default(),
        }
}

#[test]
fn execution_failed_step_guard_prefers_failed_machine_fields_over_success_stdout() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    route.locator_kind = OutputLocatorKind::None;
    route.locator_hint.clear();
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(3);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "RC_RENDER_KO_OK\n"));
    loop_state.executed_step_results.push(error_step(
        "step_2",
        "run_cmd",
        &crate::skills::structured_skill_error_from_parts(
            "run_cmd",
            "nonzero_exit",
            "Command failed with exit code 127",
            Some("linux"),
            Some(serde_json::json!({
                "command": "definitely_missing_command_rustclaw_render_ko_0605",
                "exit_category": "command_not_found",
                "exit_classification_source": "exit_code",
                "exit_code": 127,
                "stderr": "bash: line 1: definitely_missing_command_rustclaw_render_ko_0605: command not found\n",
                "stdout": serde_json::Value::Null,
            })),
        ),
    ));
    loop_state.executed_step_results.push(error_step(
        "step_4",
        "run_cmd",
        &crate::skills::structured_skill_error_from_parts(
            "run_cmd",
            "nonzero_exit",
            "Command failed with exit code 127",
            Some("linux"),
            Some(serde_json::json!({
                "command": "definitely_missing_command_rustclaw_render_ko_0605",
                "exit_category": "command_not_found",
                "exit_classification_source": "exit_code",
                "exit_code": 127,
                "stderr": "bash: line 1: definitely_missing_command_rustclaw_render_ko_0605: command not found\n",
                "stdout": serde_json::Value::Null,
            })),
        ),
    ));

    let guard = execution_failed_step_guard_entry(&loop_state, ctx.output_contract()).unwrap();

    assert!(route_disallows_direct_observation_passthrough(&route));
    assert!(guard.contains("final_answer_shape=failed_step_with_evidence"));
    assert!(guard.contains("successful_step_outputs_are_not_final_answer=true"));
    assert!(guard.contains("success_step.1.output_is_not_answer=RC_RENDER_KO_OK"));
    assert!(guard.contains("failed_step.1.step_id=step_2"));
    assert!(guard.contains("failed_step.1.skill=run_cmd"));
    assert!(
        guard.contains("failed_step.1.command=definitely_missing_command_rustclaw_render_ko_0605"),
        "guard: {guard}"
    );
    assert!(guard.contains("failed_step.1.exit_category=command_not_found"));
    assert!(guard.contains("failed_step.1.exit_code=127"));
    assert!(!guard.contains("step_4"), "guard: {guard}");
}

#[test]
fn execution_failed_step_guard_skips_contract_policy_gap_errors() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    route.locator_kind = OutputLocatorKind::None;
    route.locator_hint.clear();
    let ctx = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(error_step(
        "step_1",
        "make_dir",
        r#"__RC_SKILL_ERROR__:{"error_kind":"contract_action_rejected","error_text":"planned tool step was not allowed for this request","extra":{"failure_attribution":"contract_gap"},"skill":"make_dir"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "run_cmd", "note.txt alpha beta removed\n"));

    let guard = execution_failed_step_guard_entry(&loop_state, ctx.output_contract());

    assert!(
        guard.is_none(),
        "contract policy gaps are loop recovery signals, not final failed-step evidence: {guard:?}"
    );
}

#[test]
fn scalar_path_observed_route_rejects_content_evidence_contract() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.selection.structured_field_selector = Some("path".to_string());
    route.requires_content_evidence = true;

    assert!(route_requests_exact_scalar_path(&route));
    assert!(!route_allows_path_batch_scalar_path_observed_answer(&route));

    route.requires_content_evidence = false;
    assert!(route_allows_path_batch_scalar_path_observed_answer(&route));
}

#[test]
fn observed_output_route_policy_uses_direct_output_contract() {
    let mut scalar_path_route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    scalar_path_route.selection.structured_field_selector = Some("path".to_string());
    scalar_path_route.requires_content_evidence = false;
    assert!(route_requests_exact_scalar_path(&scalar_path_route));
    assert!(route_allows_path_batch_scalar_path_observed_answer(
        &scalar_path_route
    ));

    scalar_path_route.requires_content_evidence = true;
    assert!(!route_allows_path_batch_scalar_path_observed_answer(
        &scalar_path_route
    ));

    let mut file_names_route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    file_names_route.semantic_kind = OutputSemanticKind::FileNames;
    assert!(route_prefers_plain_fs_search_paths(&file_names_route));
    assert!(route_allows_raw_listing_direct_answer(Some(
        &file_names_route
    )));

    let mut failed_step_route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    failed_step_route.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    failed_step_route.locator_kind = OutputLocatorKind::None;
    failed_step_route.locator_hint.clear();
    assert!(route_disallows_direct_observation_passthrough(
        &failed_step_route
    ));

}

#[test]
fn scalar_count_answer_detects_non_numeric_diagnostic_line() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.semantic_kind = OutputSemanticKind::ScalarCount;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "configs/config_copy".to_string();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "0\n\nfind: /workspace/configs/config_copy: No such file or directory\n",
    ));

    let diagnostic = scalar_count_diagnostic_line_for_answer("0", Some(&route), &loop_state);

    assert_eq!(
        diagnostic.as_deref(),
        Some("find: /workspace/configs/config_copy: No such file or directory")
    );
    let answer = scalar_count_diagnostic_machine_answer(diagnostic.as_deref().unwrap());
    assert!(answer.contains("message_key=clawd.msg.scalar_count.unreliable"));
    assert!(answer.contains("reason_code=count_unreliable_diagnostic"));
    assert!(answer.contains("final_answer_shape=scalar_count_unavailable"));
    assert!(answer.contains(
        "diagnostic=find: /workspace/configs/config_copy: No such file or directory"
    ));
}

fn reuse_active_context(user_request: &str) -> AgentRunContext {
    AgentRunContext {
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskAppend),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        user_request: Some(user_request.to_string()),
        ..Default::default()
    }
}

#[test]
fn recent_generated_output_extracts_internal_merge_block() {
    let merged = "Current task:\nlook at that docs dir\n\nMost recent generated output:\narchive\nrelease_checklist.md\nservice_notes.md\n\nContinuity rules:\n- keep scope\n\nNew user instruction:\ncount only";

    assert_eq!(
        recent_generated_output_from_user_request(merged).as_deref(),
        Some("archive\nrelease_checklist.md\nservice_notes.md")
    );
}

#[test]
fn cross_turn_observed_entries_require_reuse_active_context() {
    let merged = "Current task:\nlook at that docs dir\n\nMost recent generated output:\narchive\nrelease_checklist.md\nservice_notes.md\n\nContinuity rules:\n- keep scope";
    let loop_state = LoopState::new(1);
    let allowed = reuse_active_context(merged);

    let entries = cross_turn_observed_output_entries(&loop_state, Some(&allowed));
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("prior_turn_observed_output"));
    assert!(entries[0].contains("archive"));
    assert!(!entries[0].contains("Continuity rules"));

    let standalone = AgentRunContext {
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        user_request: Some(merged.to_string()),
        ..Default::default()
    };
    assert!(cross_turn_observed_output_entries(&loop_state, Some(&standalone)).is_empty());
}

#[test]
fn direct_scalar_ignores_exit_zero_prefix() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "machine_probe", "exit=0\nready\n"));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("ready")
    );
}

#[test]
fn direct_scalar_extracts_system_basic_runtime_status_value() {
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route_result.locator_kind = OutputLocatorKind::None;
    route_result.locator_hint.clear();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..Default::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"runtime_status","kind":"current_user","value":"guagua","field_value":"guagua","command_output":"guagua"}"#,
    ));

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("guagua")
    );
}

#[test]
fn observed_entries_include_structured_extract_field_outputs() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#,
        ));

    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 2);
    assert!(entries[0].contains("name: react-example"));
    assert!(entries[1].contains("package.name: clawd"));
}

#[test]
fn direct_scalar_ignores_shell_locale_warning_noise() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "/tmp/rustclaw-workspace\n\nbash: warning: setlocale: LC_ALL: cannot change locale (C.UTF-8): No such file or directory\n",
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("/tmp/rustclaw-workspace")
    );
}

#[test]
fn direct_scalar_reads_extract_field_value_from_structured_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("rustclaw")
    );
}

#[test]
fn direct_scalar_reads_read_field_value_from_structured_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","exists":true,"field_path":"package.name","value_text":"react-example","value":"react-example","value_type":"string"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("react-example")
    );
}

#[test]
fn direct_scalar_defers_container_read_field_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","exists":true,"field_path":"scripts","value":{"build":"echo build","dev":"echo dev"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\"}","value_type":"object"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None),
        None
    );
}

#[test]
fn direct_scalar_returns_container_read_field_json_for_scalar_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","exists":true,"field_path":"package.version","value":{"workspace":true},"value_text":"{\"workspace\":true}","value_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "Cargo.toml".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(r#"{"workspace":true}"#)
    );
}

#[test]
fn direct_scalar_preserves_resolved_extract_field_label_for_non_exact_match() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"model.vendor","resolved_field_path":"llm.selected_vendor","match_strategy":"missing_parent_leaf_key_suffix","value_text":"minimax","value":"minimax","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "configs/config.toml".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("llm.selected_vendor: minimax")
    );
}

#[test]
fn direct_scalar_reads_array_identity_field_value_without_label() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"archive_basic.group","resolved_field_path":"skills[name=archive_basic].group","match_strategy":"array_item_key_path","value_text":"system","value":"system","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "configs/skills_registry.toml".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("system")
    );
}

#[test]
fn direct_answer_reads_array_identity_extract_field_value_without_label() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"skills.[name=archive_basic].group","resolved_field_path":"skills.[name=archive_basic].group","match_strategy":"exact_path","value_text":"system","value":"system","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "configs/skills_registry.toml".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("system")
    );
}

#[test]
fn direct_answer_reads_config_basic_extract_field_value() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"run_cmd.planner_kind","value_text":"tool","value":"tool","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "configs/skills_registry.toml".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("run_cmd.planner_kind: tool")
    );
    assert!(has_observed_answer_candidates(&loop_state));
}

#[test]
fn direct_answer_reads_config_basic_read_fields_values() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_fields","path":"package.json","resolved_path":"/tmp/package.json","count":2,"results":[{"field_path":"name","exists":true,"value_type":"string","value_text":"react-example","value":"react-example"},{"field_path":"version","exists":true,"value_type":"string","value_text":"1.0.0","value":"1.0.0"}]}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "package.json".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("name: react-example\nversion: 1.0.0")
    );
}

#[test]
fn structured_keys_without_explicit_selector_defers_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"structured_keys","exists":true,"container_type":"object","count":3,"keys":["app","features","paths"],"field_path":""}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint =
        "scripts/nl_tests/fixtures/device_local/configs/app_config.toml".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_defers_container_extract_field_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "package.json".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_formats_schema_enum_extract_field_with_resolved_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"target","resolved_field_path":"properties.reference_resolution.properties.target","match_strategy":"unique_bare_key","value":{"type":"string","enum":["none","current_action_result","current_turn_locator"]},"value_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint =
        "prompts/schemas/agent_loop_decision_envelope.schema.json".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("schema enum should be formatted without synthesis");

    assert!(answer.contains("properties.reference_resolution.properties.target"));
    assert!(answer.contains("`none`"));
    assert!(answer.contains("`current_turn_locator`"));
}

#[test]
fn direct_answer_formats_config_basic_validate_result_as_pass_fail() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"validate_structured","path":"configs/config.toml","resolved_path":"/tmp/configs/config.toml","format":"toml","valid":true,"root_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "configs/config.toml".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        original_user_request: Some(
            "Validate configs/config.toml and answer pass or fail.".to_string(),
        ),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("message_key=clawd.msg.validate_structured_pass\nreason_code=validate_structured_pass\nfinal_answer_shape=structured_validation\nvalid=true\nformat=toml")
    );
}

#[test]
fn direct_scalar_formats_config_validation_result_in_request_language() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"validate_structured","path":"configs/config.toml","resolved_path":"/tmp/configs/config.toml","format":"toml","valid":true,"root_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.semantic_kind = OutputSemanticKind::ConfigValidation;
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "configs/config.toml".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        original_user_request: Some("只检查 configs/config.toml 是否是合法 TOML。".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("message_key=clawd.msg.validate_structured_pass\nreason_code=validate_structured_pass\nfinal_answer_shape=structured_validation\nvalid=true\nformat=toml")
    );
}

#[test]
fn direct_scalar_defers_multiple_structured_scalars_without_semantic_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","path":"UI/package.json","resolved_path":"/tmp/UI/package.json","field_path":"name","resolved_field_path":"name","exists":true,"value_type":"string","value_text":"react-example","value":"react-example"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "config_basic",
            r#"{"action":"read_field","path":"crates/clawd/Cargo.toml","resolved_path":"/tmp/crates/clawd/Cargo.toml","field_path":"package.name","resolved_field_path":"package.name","exists":true,"value_type":"string","value_text":"clawd","value":"clawd"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint =
        "UI/package.json|crates/clawd/Cargo.toml".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        original_user_request: Some(
            "Read two structured fields, then provide one final line.".to_string(),
        ),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
    assert!(extract_direct_scalar_from_generic_output_i18n(
        &loop_state,
        &AppState::test_default_with_fixture_provider(),
        Some(&agent_run_context)
    )
    .is_none());
}

#[test]
fn structured_pair_answer_does_not_infer_fields_from_read_file_outputs() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "read_file",
        r#"{"name":"react-example","version":"0.0.0"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "read_file",
        r#"[package]
name = "clawd"
version.workspace = true
"#,
    ));
    let route_result = IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                selection: crate::OutputSelectionContract::default(),
            };
    let _agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        super::recent_structured_scalar_observation_count(&loop_state),
        0
    );
}

#[test]
fn direct_scalar_reports_missing_extract_field_as_readable_message() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("message_key=clawd.msg.extract_field_missing\nreason_code=extract_field_missing\nfinal_answer_shape=missing_structured_field\nexists=false\nfield_path=name")
    );
}

#[test]
fn internal_missing_sentinel_uses_structured_extract_field_evidence() {
    let state = AppState::test_default_with_fixture_provider();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"package.name","value_text":"","value":null,"value_type":"null"}"#,
        ));

    assert_eq!(
        replace_internal_missing_sentinel_with_structured_observation(
            "<missing>",
            &state,
            &loop_state,
            None
        )
        .as_deref(),
        Some("message_key=clawd.msg.extract_field_missing\nreason_code=extract_field_missing\nfinal_answer_shape=missing_structured_field\nexists=false\nfield_path=package.name")
    );
    assert_eq!(
        replace_internal_missing_sentinel_with_structured_observation(
            "package.name: <missing>",
            &state,
            &loop_state,
            None
        )
        .as_deref(),
        Some("message_key=clawd.msg.extract_field_missing\nreason_code=extract_field_missing\nfinal_answer_shape=missing_structured_field\nexists=false\nfield_path=package.name")
    );
}

#[test]
fn direct_scalar_missing_field_language_uses_original_request_before_resolved_prompt() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "package.json".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        original_user_request: Some("读取 package.json 里的 name 字段，只输出值".to_string()),
        user_request: Some(
            "Read the name field from package.json and output only its value.".to_string(),
        ),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context),
        )
        .as_deref(),
        Some("message_key=clawd.msg.extract_field_missing\nreason_code=extract_field_missing\nfinal_answer_shape=missing_structured_field\nexists=false\nfield_path=name")
    );
}
