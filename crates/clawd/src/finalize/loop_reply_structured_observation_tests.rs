use super::*;

#[test]
fn scalar_path_only_matrix_answer_projects_ambiguous_find_name_candidates() {
    let state = test_state();
    let task = claimed_task("task-scalar-path-list-from-find-name");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"find_name","count":4,"results":["scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"],"root":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":4,"facts":[{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","resolved_path":"/repo/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md"},"path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md"},{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt","resolved_path":"/repo/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"},"path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"},{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","resolved_path":"/repo/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"},"path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"},{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log","resolved_path":"/repo/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"},"path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"}],"include_missing":true}"#,
    ));
    let mut route = scalar_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "abcd".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = super::deterministic_matrix_observed_shape_answer(
        &state,
        &task,
        "find abcd",
        &loop_state,
        Some(&ctx),
    )
    .expect("scalar path candidate list");

    assert_eq!(
        answer,
        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"
    );
    let delivery_messages = vec![answer.clone()];
    let journal = crate::finalize::build_from_loop_state(
        &task,
        "find abcd",
        &loop_state,
        Some(&ctx),
        Some(summary.clone()),
        crate::task_journal::delivery_payload_consistent(&answer, &delivery_messages),
        &answer,
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    assert!(
        crate::answer_verifier::structurally_satisfies_answer_contract(&route, &journal, &answer)
    );
    assert_eq!(summary.format_ok, Some(true));
}

#[test]
fn direct_structured_observed_answer_defers_implicit_metadata_path_facts() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"tmp/test_bundle.zip","resolved_path":"/tmp/test_bundle.zip","size_bytes":272,"modified_ts":1776352013},"path":"/tmp/test_bundle.zip"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/test_bundle.zip".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_structured_observed_answer(Some(&state), &loop_state, Some(&ctx)).is_none());
    assert!(super::latest_path_batch_facts_has_implicit_metadata_fields(
        &loop_state
    ));
}

#[test]
fn direct_db_basic_observed_answer_uses_latest_rows_after_synthesis_failure() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
    ));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("synthesis failed".to_string()),
        started_at: 1,
        finished_at: 2,
    });
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "db_basic",
        r#"{"columns":["id","name"],"rows":[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}]}"#,
    ));

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, summary) = direct_db_basic_observed_answer(
        &state,
        "Read id and name from users limit 2.",
        &loop_state,
        Some(&ctx),
    )
    .expect("db rows fallback");

    assert!(answer.contains("id: 1"));
    assert!(answer.contains("name: Alice"));
    assert!(answer.contains("id: 2"));
    assert!(answer.contains("name: Bob"));
    assert!(!answer.contains("orders"));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn direct_db_basic_observed_answer_counts_rows_for_scalar_count_contract() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
    ));

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, summary) = direct_db_basic_observed_answer(
        &state,
        "统计 SQLite 数据库的表数量，只输出数字",
        &loop_state,
        Some(&ctx),
    )
    .expect("scalar count fallback");

    assert_eq!(answer, "3");
    assert_eq!(summary.format_ok, Some(true));
}

#[test]
fn direct_structured_observed_answer_defers_when_plan_requested_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# Device Local Fixture\n2|\n3|This directory contains stable local files for tests."}"#,
    ));
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read then summarize".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_raw_text(
                r#"{"steps":[{"type":"call_tool","tool":"fs_basic"},{"type":"synthesize_answer","evidence_refs":["last_output"]}]}"#,
            )),
            verify_result: None,
        });
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/README.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_structured_observed_answer(None, &loop_state, Some(&ctx)).is_none());
}

#[test]
fn direct_structured_observed_answer_keeps_passthrough_without_synthesis_plan() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","excerpt":"1|[app]\n2|name = \"fixture\""}"#,
    ));
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/config.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let (answer, _) = direct_structured_observed_answer(None, &loop_state, Some(&ctx))
        .expect("direct passthrough without synthesis plan");

    assert_eq!(answer, "[app]\nname = \"fixture\"");
}

#[test]
fn broad_structured_read_drops_separator_and_validates_file() {
    let state = test_state();
    let path = std::env::temp_dir().join(format!(
        "rustclaw_structured_validation_{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "[memory]\nconfig_path = \"configs/memory.toml\"\n")
        .expect("write temp toml");
    let path_text = path.to_string_lossy().to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec![
        "=============================================================================".to_string(),
    ];
    loop_state.last_user_visible_respond = Some(
        "=============================================================================".to_string(),
    );
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "read_range",
            "mode": "head",
            "requested_n": 120,
            "path": path_text,
            "resolved_path": path_text,
            "excerpt": "1|# ============================================================================="
        })
        .to_string(),
    ));

    assert!(
        discard_non_answer_separator_delivery_for_broad_structured_read("task", &mut loop_state)
    );
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.last_user_visible_respond.is_none());

    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let (answer, summary) = deterministic_structured_file_validation_from_read_range(
        &state,
        "Vérifie seulement si ce fichier est un TOML valide.",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured validation fallback");
    assert!(
        answer.contains("format=toml") && answer.contains("validation_status=pass"),
        "answer: {answer}"
    );
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );

    let mut route = free_route_result();
    route.route_reason = "capability_ref=config.validate".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let (answer, _) = deterministic_structured_file_validation_from_read_range(
        &state,
        "Vérifie seulement si ce fichier est un TOML valide.",
        &loop_state,
        Some(&ctx),
    )
    .expect("capability_ref structured validation fallback");
    assert!(
        answer.contains("format=toml") && answer.contains("validation_status=pass"),
        "answer: {answer}"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn broad_structured_read_validation_does_not_replace_directory_summary() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"head","path":"UI/package.json","resolved_path":"UI/package.json","excerpt":"1|{\n2|  \"name\": \"react-example\"\n3|}"}"#,
    ));
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(deterministic_structured_file_validation_from_read_range(
        &state,
        "Summarize the directory and use the package name as context.",
        &loop_state,
        Some(&ctx),
    )
    .is_none());
}
