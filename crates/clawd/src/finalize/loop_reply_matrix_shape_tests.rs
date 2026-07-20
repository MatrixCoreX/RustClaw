use super::*;

#[test]
fn matrix_shape_replaces_stale_file_token_delivery_with_observed_directory_listing() {
    let state = test_state();
    let task = claimed_task("task-stale-file-token-listing");
    let mut route = free_route_result();
    route.delivery_required = true;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.locator_kind = crate::OutputLocatorKind::None;
    route.locator_hint.clear();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "directory inventory".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_capability".to_string(),
                skill: "filesystem.list_dir".to_string(),
                args: serde_json::json!({
                    "path": "logs",
                    "names_only": true,
                    "max_entries": 5
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        &serde_json::json!({
            "action": "inventory_dir",
            "path": "logs",
            "resolved_path": "/home/guagua/rustclaw/logs",
            "names": [
                "act_plan.log",
                "agent_rollout_metrics",
                "base_skill_contracts_20260516_100540",
                "base_skill_contracts_20260516_112927",
                "base_skill_contracts_20260527_042323"
            ],
            "entries": [
                {"kind": "file", "name": "act_plan.log", "path": "logs/act_plan.log"},
                {"kind": "dir", "name": "agent_rollout_metrics", "path": "logs/agent_rollout_metrics"},
                {"kind": "dir", "name": "base_skill_contracts_20260516_100540", "path": "logs/base_skill_contracts_20260516_100540"},
                {"kind": "dir", "name": "base_skill_contracts_20260516_112927", "path": "logs/base_skill_contracts_20260516_112927"},
                {"kind": "dir", "name": "base_skill_contracts_20260527_042323", "path": "logs/base_skill_contracts_20260527_042323"}
            ]
        })
        .to_string(),
    ));
    let mut delivery = vec!["FILE:/home/guagua/rustclaw/logs/model_io.log".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "directory inventory",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(
        delivery,
        vec![
            "act_plan.log\nagent_rollout_metrics\nbase_skill_contracts_20260516_100540\nbase_skill_contracts_20260516_112927\nbase_skill_contracts_20260527_042323"
                .to_string()
        ]
    );
    assert_eq!(
        loop_state.last_user_visible_respond,
        delivery.first().cloned()
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_shape_replaces_stale_file_token_delivery_with_bounded_read_excerpt() {
    let state = test_state();
    let task = claimed_task("task-stale-file-token-read-range");
    let mut route = free_route_result();
    route.delivery_required = true;
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "/tmp/README.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "bounded file excerpt".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_capability".to_string(),
                skill: "filesystem.read_text_range".to_string(),
                args: serde_json::json!({
                    "path": "/tmp/README.md",
                    "mode": "head",
                    "n": 5
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "extra": {
                "action": "read_range",
                "mode": "head",
                "path": "/tmp/README.md",
                "resolved_path": "/tmp/README.md",
                "requested_n": 5,
                "start_line": 1,
                "end_line": 5,
                "total_lines": 9,
                "excerpt": "1|# Device Local Fixture\n2|\n3|This directory contains stable local files for RustClaw NL regression tests.\n4|\n5|- `configs/app_config.toml`: sample runtime config"
            }
        })
        .to_string(),
    ));
    let mut delivery = vec!["FILE:/tmp/README.md".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "bounded file excerpt",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(
        delivery,
        vec![
            "# Device Local Fixture\n\nThis directory contains stable local files for RustClaw NL regression tests.\n\n- `configs/app_config.toml`: sample runtime config"
                .to_string()
        ]
    );
    assert_eq!(
        loop_state.last_user_visible_respond,
        delivery.first().cloned()
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_shape_keeps_direct_file_delivery_respond_over_bounded_read_excerpt() {
    let state = test_state();
    let task = claimed_task("task-direct-file-token-read-range-kept");
    let mut route = free_route_result();
    route.delivery_required = true;
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "/tmp/README.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "deliver file token".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_capability".to_string(),
                    skill: "filesystem.read_text_range".to_string(),
                    args: serde_json::json!({"path": "/tmp/README.md", "mode": "head", "n": 5}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({"content": "FILE:/tmp/README.md"}),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","mode":"head","path":"/tmp/README.md","requested_n":5,"excerpt":"1|# Device Local Fixture"}}"#,
    ));
    let mut delivery = vec!["FILE:/tmp/README.md".to_string()];
    let original = delivery.clone();
    let mut finalizer_summary = None;

    assert!(
        !super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "deliver file token",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );
    assert_eq!(delivery, original);
    assert!(finalizer_summary.is_none());
}

#[test]
fn matrix_shape_keeps_file_token_when_plan_uses_runtime_file_selection_template() {
    let state = test_state();
    let task = claimed_task("task-file-token-template-kept");
    let mut route = free_route_result();
    route.delivery_required = true;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.locator_kind = crate::OutputLocatorKind::None;
    route.locator_hint.clear();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.round_traces.push(crate::task_journal::TaskJournalRoundTrace {
        round_no: 1,
        goal: "deliver selected file".to_string(),
        execution_recipe_summary: None,
        plan_result: Some(plan_result_with_steps(vec![
            crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_capability".to_string(),
                skill: "filesystem.list_dir".to_string(),
                args: serde_json::json!({"path": "logs", "names_only": true}),
                depends_on: Vec::new(),
                why: String::new(),
            },
            crate::PlanStep {
                step_id: "step_2".to_string(),
                action_type: "respond".to_string(),
                skill: "respond".to_string(),
                args: serde_json::json!({"content": "FILE:{{last_output.lines().next().unwrap()}}"}),
                depends_on: vec!["step_1".to_string()],
                why: String::new(),
            },
        ])),
        verify_result: None,
    });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"inventory_dir","path":"logs","names":["act_plan.log","model_io.log"]}"#,
    ));
    let mut delivery = vec!["FILE:/home/guagua/rustclaw/logs/model_io.log".to_string()];
    let original = delivery.clone();
    let mut finalizer_summary = None;

    assert!(
        !super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "deliver selected file",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );
    assert_eq!(delivery, original);
    assert!(finalizer_summary.is_none());
}

#[test]
fn active_bound_inventory_path_overrides_bare_path_directory_listing_contract() {
    let state = test_state();
    let task = claimed_task("task-active-bound-inventory-path");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "scripts/nl_tests/fixtures/locator_smart/case_only".to_string();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        cross_turn_recent_execution_context: Some("### ACTIVE_EXECUTION_ANCHOR\nfollowup_source_request: find report\nfollowup_op_kind: Read\nfollowup_bound_target: case_only/report.md\nobserved_bound_target: case_only/report.md".to_string()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":0,"files":1,"total":1},"entries":[{"kind":"file","name":"Report.MD","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","size_bytes":33}],"names":["Report.MD"],"names_by_kind":{"dirs":[],"files":["Report.MD"],"other":[]},"path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/case_only","resolved_path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/case_only"}"#,
    ));

    let (answer, _) = direct_path_from_active_bound_inventory(&loop_state, Some(&ctx))
        .expect("active bound target should select matching inventory entry path");
    assert_eq!(
        answer,
        "scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD"
    );

    let mut delivery = vec!["Report.MD".to_string()];
    let mut finalizer_summary = None;
    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "scripts/nl_tests/fixtures/locator_smart/case_only",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );
    assert_eq!(
        delivery,
        vec!["scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD"]
    );
}

#[test]
fn matrix_shape_guard_replaces_unstructured_strict_list_with_observed_list() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-list");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "document".to_string();
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":2,"ext":"md","results":["alpha.md","beta.md"],"root":"document"}"#,
    ));
    let mut delivery = vec!["document 目录下有 alpha.md 和 beta.md。".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "列出 document 下的 md 文件名，只输出列表",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(delivery, vec!["alpha.md\nbeta.md"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("alpha.md\nbeta.md")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_shape_guard_replaces_scalar_count_field_placeholder_with_observed_value() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-scalar-count");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "logs".to_string();
    route.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"count_inventory","counts":{"by_extension":{"log":2},"dirs":0,"files":2,"hidden":0,"total":2,"total_size_bytes":2698},"path":"logs","recursive":false,"resolved_path":"logs"},"text":"{\"action\":\"count_inventory\",\"counts\":{\"by_extension\":{\"log\":2},\"dirs\":0,\"files\":2,\"hidden\":0,\"total\":2,\"total_size_bytes\":2698},\"path\":\"logs\",\"recursive\":false,\"resolved_path\":\"logs\"}"}"#,
    ));
    let mut delivery = vec!["count".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "count direct entries",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(delivery, vec!["2"]);
    assert_eq!(loop_state.last_user_visible_respond.as_deref(), Some("2"));
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_strict_list_shape_builds_list_from_observed_json() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "document".to_string();
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_ext","count":2,"ext":"md","results":["document/beta.md","document/alpha.md"],"root":"document"}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("matrix list answer");

    assert_eq!(answer, "alpha.md\nbeta.md");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_strict_list_ignores_inventory_json_hidden_in_visible_text() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "document".to_string();
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    let hidden_payload = serde_json::json!({
        "action": "find_ext",
        "count": 2,
        "ext": "md",
        "results": ["document/beta.md", "document/alpha.md"],
        "root": "document"
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "status": "ok",
            "text": hidden_payload
        })
        .to_string(),
    ));

    assert!(super::super::matrix_strict_list_observed_answer(&route, &loop_state).is_none());
}

#[test]
fn matrix_file_paths_inventory_uses_paths_and_applies_selector_limit() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string();
    route.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.selection.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    route.selection.list_selector.limit = Some(3);
    route.selection.list_selector.include_metadata = Some(false);
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"/repo/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3","resolved_path":"/repo/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3","sort_by":"size_desc","entries":[{"kind":"file","name":"x_abcd_log.txt","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","size_bytes":22},{"kind":"file","name":"zz_abcd_backup.log","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log","size_bytes":21},{"kind":"file","name":"abcd_report.md","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","size_bytes":20},{"kind":"file","name":"my_abcd.txt","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt","size_bytes":20}],"names":["x_abcd_log.txt","zz_abcd_backup.log","abcd_report.md","my_abcd.txt"],"names_by_kind":{"dirs":[],"files":["x_abcd_log.txt","zz_abcd_backup.log","abcd_report.md","my_abcd.txt"],"other":[]}}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("file path inventory answer");

    assert_eq!(
        answer,
        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md"
    );
    assert!(!answer.contains(" 22"));
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_path_list_inventory_uses_planner_semantic_kind() {
    let state = test_state();
    let task = claimed_task("task-fs-path-list-inventory-capability-shape");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.selection.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    route.selection.list_selector.limit = Some(2);
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"workspace","entries":[{"kind":"file","name":"alpha.md","path":"workspace/alpha.md"},{"kind":"dir","name":"docs","path":"workspace/docs"},{"kind":"file","name":"beta.md","path":"workspace/beta.md"}],"names_by_kind":{"dirs":["docs"],"files":["alpha.md","beta.md"],"other":[]}}"#,
    ));
    let mut delivery = vec!["found entries".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "find entries",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(delivery, vec!["workspace/alpha.md\nworkspace/beta.md"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("workspace/alpha.md\nworkspace/beta.md")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_filesystem_find_entries_contract_builds_path_list() {
    let state = test_state();
    let task = claimed_task("task-fs-find-capability-ref-path-list");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::FilePaths;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_entries","results":["plan/a.md","plan/b.md","docs/c.md"]}"#,
    ));
    let mut delivery = vec!["I found a few matching markdown files.".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "find matching files",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(delivery, vec!["docs/c.md\nplan/a.md\nplan/b.md"]);
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_path_list_collects_grep_text_name_results_from_wrapped_extra() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::FilePaths;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"grep_text","query":"abcd","count":0,"match_count":0,"matches":[],"name_count":4,"name_results":["scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"]}}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("path list should collect grep_text name_results");

    assert_eq!(
        answer,
        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"
    );
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_file_name_list_prefers_wrapped_names_over_size_summary_synthesis() {
    let state = test_state();
    let task = claimed_task("task-matrix-file-name-list-wrapped-names");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "document".to_string();
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":0,"files":5,"hidden":0,"total":5},"dirs_only":false,"entries":[],"files_only":true,"names":["full_suite_trace_note.txt","gen-1778122040.png","gen-1778122536.png","hello.sh","hello_from_manual_test.sh"],"names_by_kind":{"dirs":[],"files":["full_suite_trace_note.txt","gen-1778122040.png","gen-1778122536.png","hello.sh","hello_from_manual_test.sh"],"other":[]},"names_only":true,"path":"/home/guagua/rustclaw/document","resolved_path":"/home/guagua/rustclaw/document","size_summary":{"largest_file":{"kind":"file","name":"rust_icon_pixel.png","path":"document/rust_icon_pixel.png","size_bytes":2024},"smallest_file":{"kind":"file","name":"manual_fixture_note.txt","path":"document/manual_fixture_note.txt","size_bytes":32}},"sort_by":"name"},"text":"{\"action\":\"inventory_dir\",\"counts\":{\"dirs\":0,\"files\":5,\"hidden\":0,\"total\":5},\"entries\":[],\"files_only\":true,\"names\":[\"full_suite_trace_note.txt\",\"gen-1778122040.png\",\"gen-1778122536.png\",\"hello.sh\",\"hello_from_manual_test.sh\"],\"names_by_kind\":{\"dirs\":[],\"files\":[\"full_suite_trace_note.txt\",\"gen-1778122040.png\",\"gen-1778122536.png\",\"hello.sh\",\"hello_from_manual_test.sh\"],\"other\":[]},\"names_only\":true,\"path\":\"/home/guagua/rustclaw/document\",\"resolved_path\":\"/home/guagua/rustclaw/document\",\"size_summary\":{\"largest_file\":{\"kind\":\"file\",\"name\":\"rust_icon_pixel.png\",\"path\":\"document/rust_icon_pixel.png\",\"size_bytes\":2024},\"smallest_file\":{\"kind\":\"file\",\"name\":\"manual_fixture_note.txt\",\"path\":\"document/manual_fixture_note.txt\",\"size_bytes\":32}},\"sort_by\":\"name\"}"}"#,
    ));
    let mut delivery = vec![
        "目录 /home/guagua/rustclaw/document 共 5 个文件（按名称排序），观察到的条目中只有以下 2 个文件名被显式给出：\nrust_icon_pixel.png\nmanual_fixture_note.txt"
            .to_string(),
    ];
    let mut finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        ..Default::default()
    });

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "list first file names",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(
        delivery,
        vec![
            "full_suite_trace_note.txt\ngen-1778122040.png\ngen-1778122536.png\nhello.sh\nhello_from_manual_test.sh"
        ]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(
            "full_suite_trace_note.txt\ngen-1778122040.png\ngen-1778122536.png\nhello.sh\nhello_from_manual_test.sh"
        )
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn generic_observed_machine_projection_uses_inventory_names_by_kind() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","names_by_kind":{"dirs":[],"files":["full_suite_trace_note.txt","gen-1778122040.png","hello.sh"],"other":[]},"path":"document","sort_by":"name"}}"#,
    ));

    let (answer, summary) = super::super::generic_observed_machine_projection_answer(&loop_state)
        .expect("inventory names should project");

    assert_eq!(
        answer,
        "full_suite_trace_note.txt\ngen-1778122040.png\nhello.sh"
    );
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.format_ok, Some(true));
}

#[test]
fn generic_observed_machine_projection_uses_grep_matches() {
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"grep_text","matches":[{"line":16,"path":"logs/app.log","text":"2026-04-01 10:08:44 ERROR provider timeout"}],"query":"ERROR"}}"#,
    ));

    let (answer, summary) = super::super::generic_observed_machine_projection_answer(&loop_state)
        .expect("grep matches should project");

    assert_eq!(answer, "16:2026-04-01 10:08:44 ERROR provider timeout");
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.format_ok, Some(true));
}

#[test]
fn matrix_strict_list_shape_builds_directory_names_from_inventory_dirs() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryNames;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","dirs_only":true,"names":["configs","data","docs"],"names_by_kind":{"dirs":["configs","data","docs"],"files":["README.md"],"other":[]},"entries":[{"kind":"dir","name":"configs","path":"scripts/nl_tests/fixtures/device_local/configs"},{"kind":"file","name":"README.md","path":"scripts/nl_tests/fixtures/device_local/README.md"}],"path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local"},"text":"{\"action\":\"inventory_dir\",\"dirs_only\":true,\"names\":[\"configs\",\"data\",\"docs\"],\"names_by_kind\":{\"dirs\":[\"configs\",\"data\",\"docs\"],\"files\":[\"README.md\"],\"other\":[]},\"path\":\"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local\"}"}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("directory names list answer");

    assert_eq!(answer, "configs\ndata\ndocs");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn name_list_renderer_uses_file_names_contract() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","entries":[{"kind":"file","name":"README.md","path":"workspace/README.md"},{"kind":"dir","name":"crates","path":"workspace/crates"}],"names_by_kind":{"dirs":["crates"],"files":["README.md"],"other":[]},"path":"workspace"}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("capability-owned file name list");

    assert_eq!(answer, "README.md");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn name_list_renderer_uses_directory_names_contract() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryNames;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","entries":[{"kind":"file","name":"README.md","path":"workspace/README.md"},{"kind":"dir","name":"crates","path":"workspace/crates"},{"kind":"directory","name":"docs","path":"workspace/docs"}],"names_by_kind":{"dirs":["crates","docs"],"files":["README.md"],"other":[]},"path":"workspace"}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("capability-owned directory name list");

    assert_eq!(answer, "crates\ndocs");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn name_list_contract_requires_observed_projection() {
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryNames;
    route.response_shape = crate::OutputResponseShape::Strict;

    assert!(super::super::route_requires_observed_output_projection(
        &route
    ));
}

#[test]
fn path_list_contract_requires_observed_projection() {
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.response_shape = crate::OutputResponseShape::Strict;

    assert!(super::super::route_requires_observed_output_projection(
        &route
    ));
}

#[test]
fn structured_keys_contract_builds_observed_list() {
    let state = test_state();
    let task = claimed_task("task-config-key-list-capability-shape");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::StructuredKeys;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"list_keys","keys":["model","skills","runtime"]}"#,
    ));
    let mut delivery = vec!["config has several sections".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "list config keys",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(delivery, vec!["model\nruntime\nskills"]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("model\nruntime\nskills")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_strict_list_shape_builds_hidden_entry_list_from_inventory() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":2,"hidden":2,"total":3},"entries":[{"hidden":true,"kind":"dir","name":".git","path":".git"},{"hidden":true,"kind":"file","name":".gitignore","path":".gitignore"},{"hidden":false,"kind":"file","name":"README.md","path":"README.md"}],"include_hidden":true,"names":[".git",".gitignore","README.md"],"path":"."}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("matrix hidden entries answer");

    assert_eq!(answer, ".git\n.gitignore");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_strict_list_shape_respects_hidden_entry_selector_limit() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    route.selection.list_selector.limit = Some(3);
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req","status":"ok","text":"{\"action\":\"inventory_dir\"}","error_text":null,"extra":{"action":"inventory_dir","counts":{"dirs":3,"files":2,"hidden":5,"total":5},"entries":[],"include_hidden":true,"names":[".agents",".codex",".git",".gitignore",".pids","README.md"],"path":"."}}"#,
    ));

    let (answer, summary) = super::super::matrix_strict_list_observed_answer(&route, &loop_state)
        .expect("matrix hidden entries answer");

    assert_eq!(answer, ".agents\n.codex\n.git");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_grouped_name_list_shape_builds_groups_from_names_by_kind() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "workspace".to_string();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":5,"files":2,"total":7},"names_by_kind":{"files":["README.md","package.json"],"dirs":["configs","data","docs","logs","tmp"],"other":[]},"path":"workspace"}"#,
    ));

    let (answer, summary) =
        super::super::matrix_grouped_name_list_observed_answer(&route, &loop_state)
            .expect("matrix grouped name answer");

    assert_eq!(
        answer,
        "dirs:\n- configs\n- data\n- docs\n- logs\n- tmp\nfiles:\n- package.json\n- README.md"
    );
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_grouped_name_list_shape_preserves_observed_sort_order() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "scripts".to_string();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.selection.list_selector.sort_by = Some("name_desc".to_string());
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","sort_by":"name_desc","counts":{"dirs":0,"files":5,"total":5},"names_by_kind":{"files":["version_info.sh","verify_task_termination.sh","test_qwen_api.sh","test_qwen_5_channels.py","test_minimax_curl.sh"],"dirs":[],"other":[]},"path":"scripts"}"#,
    ));

    let (answer, summary) =
        super::super::matrix_grouped_name_list_observed_answer(&route, &loop_state)
            .expect("matrix grouped name answer");

    assert_eq!(
        answer,
        "files:\n- version_info.sh\n- verify_task_termination.sh\n- test_qwen_api.sh\n- test_qwen_5_channels.py\n- test_minimax_curl.sh"
    );
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_grouped_name_list_shape_reads_wrapped_inventory_extra() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "workspace".to_string();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":2,"files":3,"total":5},"names_by_kind":{"files":["README.md","Cargo.lock","Cargo.toml"],"dirs":["configs","crates"],"other":[]},"path":"workspace"},"text":"{\"action\":\"inventory_dir\",\"counts\":{\"dirs\":2,\"files\":3,\"total\":5},\"names_by_kind\":{\"files\":[\"README.md\",\"Cargo.lock\",\"Cargo.toml\"],\"dirs\":[\"configs\",\"crates\"],\"other\":[]},\"path\":\"workspace\"}"}"#,
    ));

    assert!(super::super::directory_entry_groups_prefers_observed_groups(&route, &loop_state));
    let (answer, summary) =
        super::super::matrix_grouped_name_list_observed_answer(&route, &loop_state)
            .expect("matrix grouped name answer");

    assert!(answer.contains("Cargo.toml"));
    assert_eq!(
        answer,
        "dirs:\n- configs\n- crates\nfiles:\n- Cargo.lock\n- Cargo.toml\n- README.md"
    );
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn grouped_name_list_renderer_uses_directory_entry_contract() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "workspace".to_string();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":1,"files":2,"total":3},"names_by_kind":{"files":["Cargo.toml","README.md"],"dirs":["crates"],"other":[]},"path":"workspace"}}"#,
    ));

    assert!(super::super::directory_entry_groups_prefers_observed_groups(&route, &loop_state));
    let (answer, summary) =
        super::super::matrix_grouped_name_list_observed_answer(&route, &loop_state)
            .expect("capability-owned grouped-name answer");

    assert_eq!(answer, "dirs:\n- crates\nfiles:\n- Cargo.toml\n- README.md");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn matrix_grouped_name_list_ignores_inventory_json_hidden_in_visible_text() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "workspace".to_string();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let hidden_payload = serde_json::json!({
        "action": "inventory_dir",
        "counts": {"dirs": 2, "files": 3, "total": 5},
        "names_by_kind": {
            "files": ["README.md", "Cargo.lock", "Cargo.toml"],
            "dirs": ["configs", "crates"],
            "other": []
        },
        "path": "workspace"
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "status": "ok",
            "text": hidden_payload
        })
        .to_string(),
    ));

    assert!(!super::super::directory_entry_groups_prefers_observed_groups(&route, &loop_state));
    assert!(super::super::matrix_grouped_name_list_observed_answer(&route, &loop_state).is_none());
}

#[test]
fn mixed_listing_contract_prefers_grounded_synthesis_after_file_read() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let answer = "这个仓库的 UI 更像一个独立前端，因为 UI/package.json 的 name 是 react-example，并且 UI 目录有独立构建脚本。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"total":2},"names_by_kind":{"files":["Cargo.toml"],"dirs":["UI"],"other":[]},"path":"."}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","path":"UI/package.json","excerpt":"1|{\n2|  \"name\": \"react-example\"\n3|}"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", answer));
    loop_state.last_publishable_synthesis_output = Some(answer.to_string());

    let (actual, summary) =
        super::super::latest_grounded_synthesis_for_mixed_listing_contract(&route, &loop_state)
            .expect("mixed evidence synthesis");

    assert_eq!(actual, answer);
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn mixed_listing_synthesis_uses_directory_entry_contract() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let answer = "UI/package.json 显示这个前端包名是 react-example。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"total":2},"names_by_kind":{"files":["Cargo.toml"],"dirs":["UI"],"other":[]},"path":"."}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","path":"UI/package.json","excerpt":"1|{\n2|  \"name\": \"react-example\"\n3|}"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", answer));
    loop_state.last_publishable_synthesis_output = Some(answer.to_string());

    let (actual, summary) =
        super::super::latest_grounded_synthesis_for_mixed_listing_contract(&route, &loop_state)
            .expect("capability-owned mixed evidence synthesis");

    assert_eq!(actual, answer);
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn matrix_shape_guard_replaces_unstructured_grouped_name_list_with_observed_groups() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-grouped-name-list");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "workspace".to_string();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":2,"files":1,"total":3},"names_by_kind":{"files":["README.md"],"dirs":["configs","docs"],"other":[]},"path":"workspace"}"#,
    ));
    let mut delivery = vec!["workspace 下面有 configs、docs 和 README.md。".to_string()];
    let mut finalizer_summary = None;

    assert!(
        super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "list direct children grouped by kind",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(
        delivery,
        vec!["dirs:\n- configs\n- docs\nfiles:\n- README.md"]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("dirs:\n- configs\n- docs\nfiles:\n- README.md")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn matrix_shape_guard_does_not_override_pending_clarify_delivery() {
    let state = test_state();
    let task = claimed_task("task-matrix-shape-guard-pending-clarify");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "workspace".to_string();
    route.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.pending_user_input_required = true;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":2,"files":1,"total":3},"names_by_kind":{"files":["README.md"],"dirs":["configs","docs"],"other":[]},"path":"workspace"}"#,
    ));
    let question = "Which file should I read?";
    let mut delivery = vec![question.to_string()];
    let mut finalizer_summary = None;

    assert!(
        !super::super::replace_delivery_with_matrix_observed_shape_answer(
            &state,
            &task,
            "read the first line of that file",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        )
    );

    assert_eq!(delivery, vec![question.to_string()]);
    assert!(finalizer_summary.is_none());
}

#[test]
fn matrix_shape_replacement_only_triggers_for_bad_finalizer_summary() {
    let good_summary = crate::task_journal::TaskJournalFinalizerSummary {
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        ..Default::default()
    };
    let bad_summary = crate::task_journal::TaskJournalFinalizerSummary {
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        contract_ok: false,
        completion_ok: Some(false),
        grounded_ok: Some(false),
        format_ok: Some(false),
        needs_clarify: Some(true),
        ..Default::default()
    };

    assert!(
        !super::super::finalizer_summary_requires_matrix_observed_replacement(Some(&good_summary))
    );
    assert!(
        super::super::finalizer_summary_requires_matrix_observed_replacement(Some(&bad_summary))
    );
    assert!(!super::super::finalizer_summary_requires_matrix_observed_replacement(None));
}
