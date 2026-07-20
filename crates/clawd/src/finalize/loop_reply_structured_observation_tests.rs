use super::*;
use crate::finalize::loop_reply::deterministic_structured_container_summary_answer;

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
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/test_bundle.zip".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Scalar;
    route.semantic_kind = OutputSemanticKind::ScalarCount;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
fn structured_container_summary_returns_machine_fields_for_empty_object() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"metadata","format":"json","path":"package.json","resolved_field_path":"metadata","value":{},"value_text":"{}","value_type":"object"}"#,
    ));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let answer = deterministic_structured_container_summary_answer(
        &state,
        "Summarize metadata.",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured container answer");

    assert!(answer.contains("message_key=clawd.msg.structured_container.observed"));
    assert!(answer.contains("reason_code=structured_container_observed"));
    assert!(answer.contains("field_path=metadata"));
    assert!(answer.contains("container_kind=object"));
    assert!(answer.contains("item_count=0"));
    assert!(answer.contains("is_empty=true"));
}

#[test]
fn structured_container_summary_returns_machine_fields_for_empty_array() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"scripts.test","format":"json","path":"package.json","resolved_field_path":"scripts.test","value":[],"value_text":"[]","value_type":"array"}"#,
    ));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let answer = deterministic_structured_container_summary_answer(
        &state,
        "Summarize scripts.test.",
        &loop_state,
        Some(&ctx),
    )
    .expect("structured container answer");

    assert!(answer.contains("message_key=clawd.msg.structured_container.observed"));
    assert!(answer.contains("reason_code=structured_container_observed"));
    assert!(answer.contains("field_path=scripts.test"));
    assert!(answer.contains("container_kind=array"));
    assert!(answer.contains("item_count=0"));
    assert!(answer.contains("is_empty=true"));
}

#[test]
fn direct_db_basic_observed_answer_returns_machine_fields_for_empty_rows() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"columns":["id","name"],"rows":[]}"#,
    ));

    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, summary) =
        direct_db_basic_observed_answer(&state, "List sqlite rows.", &loop_state, Some(&ctx))
            .expect("empty db rows fallback");

    assert!(answer.contains("message_key=clawd.msg.db.rows.observed"));
    assert!(answer.contains("reason_code=db_rows_observed"));
    assert!(answer.contains("row_count=0"));
    assert!(answer.contains("column_count=2"));
    assert!(answer.contains("column.1=id"));
    assert!(answer.contains("column.2=name"));
    assert_eq!(summary.format_ok, Some(true));
}

#[test]
fn direct_db_basic_observed_answer_handles_planner_selected_list_tables_without_route_evidence() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"extra":{"field_value":{"table_count":3,"tables":["orders","service_logs","users"]},"result":{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]},"table_count":3,"tables":["orders","service_logs","users"]},"text":"{\"columns\":[\"name\"],\"rows\":[{\"name\":\"orders\"},{\"name\":\"service_logs\"},{\"name\":\"users\"}]}"}"#,
    ));

    let mut route = free_route_result();
    route.requires_content_evidence = false;
    route.response_shape = OutputResponseShape::Free;
    route.locator_kind = OutputLocatorKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };

    let (answer, summary) = direct_db_basic_observed_answer(
        &state,
        "List sqlite tables and output only names.",
        &loop_state,
        Some(&ctx),
    )
    .expect("db tables fallback");

    assert_eq!(answer, "orders\nservice_logs\nusers");
    assert_eq!(summary.format_ok, Some(true));
    assert_eq!(summary.needs_clarify, Some(false));
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
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/README.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(direct_structured_observed_answer(None, &loop_state, Some(&ctx)).is_none());
}

#[test]
fn direct_structured_observed_answer_uses_names_only_inventory_despite_synthesis_plan() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"files":3,"total":3},"entries":[],"files_only":true,"names":["alpha.txt","beta.txt","gamma.txt"],"names_only":true,"path":"/tmp/docs","resolved_path":"/tmp/docs","sort_by":"name"},"text":"{}"}"#,
    ));
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list_names".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_raw_text(
                r#"{"steps":[{"type":"call_tool","tool":"fs_basic"},{"type":"synthesize_answer","evidence_refs":["last_output"]}]}"#,
            )),
            verify_result: None,
        });
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/docs".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_structured_observed_answer(None, &loop_state, Some(&ctx))
        .expect("names-only inventory should be a complete structured answer");

    assert_eq!(answer, "alpha.txt\nbeta.txt\ngamma.txt");
    assert_eq!(summary.contract_ok, true);
}

#[test]
fn direct_structured_observed_answer_uses_dirs_only_inventory_despite_synthesis_plan() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":3,"files":0,"total":3},"dirs_only":true,"entries":[{"kind":"dir","name":"configs"},{"kind":"dir","name":"data"},{"kind":"dir","name":"logs"}],"names":["configs","data","logs"],"names_by_kind":{"dirs":["configs","data","logs"],"files":[],"other":[]},"path":"/tmp/device","resolved_path":"/tmp/device","sort_by":"name"},"text":"{}"}"#,
    ));
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list_dirs".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_raw_text(
                r#"{"steps":[{"type":"call_tool","tool":"fs_basic"},{"type":"synthesize_answer","evidence_refs":["last_output"]}]}"#,
            )),
            verify_result: None,
        });
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/device".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let (answer, summary) = direct_structured_observed_answer(None, &loop_state, Some(&ctx))
        .expect("dirs-only inventory should be a complete structured answer");

    assert_eq!(answer, "configs\ndata\nlogs");
    assert_eq!(summary.contract_ok, true);
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
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/config.toml".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let (answer, _) = deterministic_structured_file_validation_from_read_range(
        &state,
        "Vérifie seulement si ce fichier est un TOML valide.",
        &loop_state,
        Some(&ctx),
    )
    .expect("config validation contract fallback");
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
    route.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
