use super::*;
use crate::finalize::loop_reply::deterministic_structured_container_summary_answer;

#[test]
fn direct_structured_observed_answer_defers_implicit_metadata_path_facts() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new();
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"tmp/test_bundle.zip","resolved_path":"/tmp/test_bundle.zip","size_bytes":272,"modified_ts":1776352013},"path":"/tmp/test_bundle.zip"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/test_bundle.zip".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(direct_structured_observed_answer(Some(&state), &loop_state, Some(&ctx)).is_none());
}

#[test]
fn structured_container_summary_returns_machine_fields_for_empty_object() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new();
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"metadata","format":"json","path":"package.json","resolved_field_path":"metadata","value":{},"value_text":"{}","value_type":"object"}"#,
    ));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
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
    let mut loop_state = crate::agent_engine::LoopState::new();
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"scripts.test","format":"json","path":"package.json","resolved_field_path":"scripts.test","value":[],"value_text":"[]","value_type":"array"}"#,
    ));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
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
fn direct_structured_observed_answer_defers_when_plan_requested_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new();
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
    let mut loop_state = crate::agent_engine::LoopState::new();
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
    let mut loop_state = crate::agent_engine::LoopState::new();
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
fn direct_structured_observed_answer_defers_generic_content_to_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new();
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

    assert!(direct_structured_observed_answer(None, &loop_state, Some(&ctx)).is_none());
}

#[test]
fn broad_structured_read_drops_separator() {
    let path = std::env::temp_dir().join(format!(
        "rustclaw_structured_validation_{}.toml",
        std::process::id()
    ));
    std::fs::write(&path, "[memory]\nconfig_path = \"configs/memory.toml\"\n")
        .expect("write temp toml");
    let path_text = path.to_string_lossy().to_string();
    let mut loop_state = crate::agent_engine::LoopState::new();
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

    let _ = std::fs::remove_file(path);
}
