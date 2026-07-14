use super::*;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use serde_json::json;

fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    }
}

fn task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: "task-local-code-projection".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn context_with_required_machine_fields(fields: serde_json::Value) -> AgentRunContext {
    AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(json!({ "required_machine_fields": fields })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    }
}

#[test]
fn finalizer_attaches_local_code_projection_before_observed_fallback() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 3 tests in 0.000s\nOK\n",
    ));
    let context = context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions"
    ]));
    let mut summary = None;

    assert!(attach_local_code_strict_json_projection(
        &task(),
        "Return JSON with changed_files, test_command, test_status, functions.",
        &mut loop_state,
        Some(&context),
        &mut summary,
    ));

    let answer = loop_state
        .delivery_messages
        .last()
        .expect("projection delivery");
    let value: serde_json::Value = serde_json::from_str(answer).expect("strict json");
    assert_eq!(
        value["changed_files"],
        serde_json::json!(["/workspace/calc_core.py"])
    );
    assert_eq!(value["test_command"], "python3 test_calc_core.py");
    assert_eq!(value["test_status"], "passed");
    assert_eq!(value["functions"], serde_json::json!(["add", "sub", "mul"]));
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.strict_json_projection_publishable")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.strict_json_projection_output")
            .map(String::as_str),
        Some(answer.as_str())
    );
    let summary = summary.expect("finalizer summary");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
    assert_eq!(summary.completion_ok, Some(true));
}

#[test]
fn finalizer_observed_fallback_defers_local_code_json_until_validation() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import safe_div\n2|assert safe_div(1,0)[\"error_code\"] == \"division_by_zero\""}}"#,
    ));
    let context = context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes"
    ]));

    assert!(
        crate::agent_engine::local_code_strict_json_projection_should_defer_finalizer_fallback(
            "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}

#[test]
fn local_code_projection_replaces_stale_auxiliary_observe_delivery() {
    let mut loop_state = LoopState::new(2);
    loop_state.delivery_messages.push(
        "message_key=clawd.msg.fs_search.observed\nreason_code=fs_search_no_match\nmatched=false"
            .to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "run_cmd",
        "Ran 3 tests in 0.000s\nOK\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"grep_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","query":"^def ","matches":[],"match_count":0}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_6",
        "synthesize_answer",
        "message_key=clawd.msg.fs_search.observed\nreason_code=fs_search_no_match\nmatched=false",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_7",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|\n4|def sub(a, b):\n5|    return a - b\n6|\n7|def mul(a, b):\n8|    return a * b"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_8",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub, mul\n2|def test_add(): pass\n3|def test_sub(): pass\n4|def test_mul(): pass"}}"#,
    ));
    let context = context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions"
    ]));
    let mut summary = None;

    assert!(attach_local_code_strict_json_projection(
        &task(),
        "Return JSON with changed_files, test_command, test_status, functions.",
        &mut loop_state,
        Some(&context),
        &mut summary,
    ));

    assert_eq!(loop_state.delivery_messages.len(), 1);
    let value: serde_json::Value =
        serde_json::from_str(loop_state.delivery_messages[0].as_str()).expect("strict json");
    assert_eq!(value["functions"], serde_json::json!(["add", "sub", "mul"]));
    assert_eq!(value["test_status"], "passed");
    assert!(summary.is_some());
}

#[test]
fn local_code_projection_syncs_final_delivery_after_generic_renderers() {
    let mut loop_state = LoopState::new(2);
    loop_state.delivery_messages.push(
        "message_key=clawd.msg.fs_search.observed\nreason_code=fs_search_no_match\nmatched=false"
            .to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 3 tests in 0.000s\nOK\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub, mul\n2|def test_add(): pass\n3|def test_sub(): pass\n4|def test_mul(): pass"}}"#,
    ));
    let context = context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions"
    ]));
    let mut summary = None;
    let mut delivery_deduped = vec![
        "execution_summary=tool_steps_complete".to_string(),
        "message_key=clawd.msg.fs_search.observed\nreason_code=fs_search_no_match\nmatched=false"
            .to_string(),
    ];

    assert!(sync_final_delivery_with_local_code_projection(
        &task(),
        "Return JSON with changed_files, test_command, test_status, functions.",
        &mut loop_state,
        Some(&context),
        &mut summary,
        &mut delivery_deduped,
    ));

    assert_eq!(delivery_deduped.len(), 1);
    let value: serde_json::Value =
        serde_json::from_str(delivery_deduped[0].as_str()).expect("strict json");
    assert_eq!(value["functions"], serde_json::json!(["add", "sub", "mul"]));
    assert_eq!(value["test_status"], "passed");
    assert_eq!(loop_state.delivery_messages, delivery_deduped);
    assert!(summary.is_some());
}

#[test]
fn local_code_projection_replaces_readonly_machine_kv_with_requested_json() {
    let mut loop_state = LoopState::new(2);
    loop_state.delivery_messages.push(
        "project_dir=/workspace functions=[\"add\",\"sub\",\"safe_div\"] error_codes=[\"division_by_zero\"] test_status=passed evidence_files=[\"/workspace/calc_core.py\",\"/workspace/test_calc_core.py\"]"
            .to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def safe_div(a, b):\n6|    if b == 0:\n7|        return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub, safe_div\n2|def test_add(): pass\n3|def test_sub(): pass\n4|def test_safe_div_zero(): pass"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 4 tests in 0.000s\nOK\n",
    ));
    let mut summary = None;
    let mut delivery_deduped = loop_state.delivery_messages.clone();

    assert!(sync_final_delivery_with_local_code_projection(
        &task(),
        "Return JSON with project_dir, functions, error_codes, test_status, evidence_files.",
        &mut loop_state,
        None,
        &mut summary,
        &mut delivery_deduped,
    ));

    assert_eq!(delivery_deduped.len(), 1);
    let value: serde_json::Value =
        serde_json::from_str(delivery_deduped[0].as_str()).expect("strict json");
    assert_eq!(value["project_dir"], "/workspace");
    assert_eq!(
        value["functions"],
        serde_json::json!(["add", "sub", "safe_div"])
    );
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
    assert_eq!(value["test_status"], "passed");
    assert_eq!(
        value["evidence_files"],
        serde_json::json!(["/workspace/calc_core.py", "/workspace/test_calc_core.py"])
    );
    assert!(summary.is_some());
}

#[test]
fn recorded_local_code_projection_syncs_stale_final_delivery() {
    let mut loop_state = LoopState::new(2);
    let projected = r#"{"evidence_files":["/workspace/calc_core.py","/workspace/test_calc_core.py"],"functions":["add","sub","safe_div"],"error_codes":["division_by_zero"],"project_dir":"/workspace","test_status":"passed"}"#;
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_publishable".to_string(),
        "true".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_output".to_string(),
        projected.to_string(),
    );
    let mut delivery_deduped = vec![
        "project_dir=/workspace functions=[\"add\",\"sub\",\"safe_div\"] error_codes=[\"division_by_zero\"] test_status=passed evidence_files=[\"/workspace/calc_core.py\",\"/workspace/test_calc_core.py\"]"
            .to_string(),
    ];
    let mut summary = None;

    assert!(sync_recorded_local_code_projection_if_needed(
        &task(),
        "Return JSON with project_dir, functions, error_codes, test_status, evidence_files.",
        &mut loop_state,
        None,
        &mut summary,
        &mut delivery_deduped,
    ));

    assert_eq!(delivery_deduped, vec![projected.to_string()]);
    assert_eq!(loop_state.delivery_messages, delivery_deduped);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(projected)
    );
    assert!(summary.is_some());
}

#[test]
fn recorded_local_code_projection_does_not_sync_non_code_payload() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_publishable".to_string(),
        "true".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_output".to_string(),
        r#"{"field_path":"llm.selected_vendor","current_value":"minimax"}"#.to_string(),
    );
    let current = "minimax".to_string();
    let mut delivery_deduped = vec![current.clone()];
    let mut summary = None;

    assert!(!sync_recorded_local_code_projection_if_needed(
        &task(),
        "Return current_value.",
        &mut loop_state,
        None,
        &mut summary,
        &mut delivery_deduped,
    ));

    assert_eq!(delivery_deduped, vec![current]);
    assert!(summary.is_none());
}

#[test]
fn latest_synthesis_local_code_projection_replaces_file_read_delivery() {
    let mut loop_state = LoopState::new(2);
    let synthesis = r#"{"changed_files":["/workspace/calc_core.py","/workspace/test_calc_core.py"],"error_codes":["division_by_zero"],"functions":["add","sub","mul","safe_div"],"test_command":["python3 test_calc_core.py","python3 - <<'PY'\nimport calc_core\nresult = calc_core.safe_div(1, 0)\nprint(result)\nassert result == {\"ok\": False, \"error_code\": \"division_by_zero\"}\nPY"],"test_status":"passed"}"#;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "run_cmd", "all tests passed\n"));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "run_cmd",
        r#"{'ok': False, 'error_code': 'division_by_zero'}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_5", "synthesize_answer", synthesis));
    loop_state.executed_step_results.push(ok_step(
        "step_6",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b\n7|def safe_div(a, b):\n8|    if b == 0:\n9|        return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    let mut summary = None;
    let mut delivery_deduped = vec![
        "FILE:/workspace/test_calc_core.py\n\nassert calc_core.safe_div(1, 0)\n\nmissing_file"
            .to_string(),
    ];

    assert!(sync_latest_synthesis_local_code_projection_if_needed(
        &task(),
        "继续刚才这个项目：增加 safe_div(a,b)。最后只输出 JSON，包含 changed_files、test_command、test_status、functions、error_codes。",
        &mut loop_state,
        None,
        &mut summary,
        &mut delivery_deduped,
    ));

    assert_eq!(delivery_deduped, vec![synthesis.to_string()]);
    assert_eq!(loop_state.delivery_messages, delivery_deduped);
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.strict_json_projection_publishable")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.strict_json_projection_output")
            .map(String::as_str),
        Some(synthesis)
    );
    assert!(summary.is_some());
}

#[test]
fn local_code_request_fields_prefer_structured_state_patch_over_user_surface() {
    let context =
        context_with_required_machine_fields(json!(["functions", "error_codes", "test_status"]));
    let user_text =
        "最后只输出 JSON，包含 project_dir、functions、error_codes、test_status、evidence_files。";
    let partial =
        r#"{"functions":["safe_div"],"error_codes":["division_by_zero"],"test_status":"passed"}"#;
    let complete = r#"{"project_dir":"/workspace","functions":["safe_div"],"error_codes":["division_by_zero"],"test_status":"passed","evidence_files":["/workspace/calc_core.py","/workspace/test_calc_core.py"]}"#;

    assert!(
        crate::agent_engine::local_code_strict_json_answer_satisfies_request(
            user_text,
            partial,
            Some(&context),
        )
    );
    assert!(
        !crate::agent_engine::local_code_strict_json_answer_satisfies_request(
            user_text,
            complete,
            Some(&context),
        )
    );
}

#[test]
fn local_code_projection_keeps_existing_satisfying_json_delivery() {
    let mut loop_state = LoopState::new(2);
    let existing = r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed","functions":["add","sub","mul"]}"#;
    loop_state.delivery_messages.push(existing.to_string());
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 3 tests in 0.000s\nOK\n",
    ));
    let context = context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions"
    ]));
    let mut summary = None;

    assert!(!attach_local_code_strict_json_projection(
        &task(),
        "Return JSON with changed_files, test_command, test_status, functions.",
        &mut loop_state,
        Some(&context),
        &mut summary,
    ));

    assert_eq!(loop_state.delivery_messages, vec![existing.to_string()]);
    assert!(summary.is_none());
}

#[test]
fn local_code_projection_marks_equivalent_existing_json_delivery() {
    let mut loop_state = LoopState::new(2);
    let existing = r#"{"functions":["add","sub","mul"],"changed_files":["/workspace/calc_core.py"],"test_status":"passed","test_command":"python3 test_calc_core.py"}"#;
    loop_state.delivery_messages.push(existing.to_string());
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 3 tests in 0.000s\nOK\n",
    ));
    let context = context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions"
    ]));
    let mut summary = None;

    assert!(!attach_local_code_strict_json_projection(
        &task(),
        "Return JSON with changed_files, test_command, test_status, functions.",
        &mut loop_state,
        Some(&context),
        &mut summary,
    ));

    assert_eq!(loop_state.delivery_messages, vec![existing.to_string()]);
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.strict_json_projection_publishable")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.strict_json_projection_output")
            .map(String::as_str),
        Some(existing)
    );
    assert!(summary.is_some());
}

#[test]
fn local_code_projection_replaces_structurally_valid_but_less_complete_json_delivery() {
    let mut loop_state = LoopState::new(2);
    loop_state.delivery_messages.push(
        r#"{"changed_files":["/workspace/calc_core.py","/workspace/test_calc_core.py"],"test_command":["python3 test_calc_core.py","python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1,0))\nPY"],"test_status":"ALL_TESTS_PASSED","functions":["safe_div"],"error_codes":["division_by_zero"]}"#
            .to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.run_cmd_commands".to_string(),
        serde_json::json!([
            "python3 test_calc_core.py",
            "python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1,0))\nPY"
        ])
        .to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"append_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "run_cmd", "ALL_TESTS_PASSED\n"));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "run_cmd",
        r#"{"ok":false,"error_code":"division_by_zero"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"12|def safe_div(a, b):\n13|    if b == 0:\n14|        return {\"ok\": False, \"error_code\": \"division_by_zero\"}\n15|    return {\"ok\": True, \"value\": a / b}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_6",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub, mul, safe_div\n2|def test_add(): pass\n3|def test_sub(): pass\n4|def test_mul(): pass\n5|def test_safe_div_zero(): pass"}}"#,
    ));
    let context = context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes"
    ]));
    let mut summary = None;

    assert!(attach_local_code_strict_json_projection(
        &task(),
        "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
        &mut loop_state,
        Some(&context),
        &mut summary,
    ));

    let value: serde_json::Value =
        serde_json::from_str(loop_state.delivery_messages[0].as_str()).expect("strict json");
    assert_eq!(
        value["functions"],
        serde_json::json!(["add", "sub", "mul", "safe_div"])
    );
    assert_eq!(value["test_status"], "passed");
    assert!(summary.is_some());
}

#[test]
fn local_code_projection_replaces_unresolved_existing_json_delivery() {
    let mut loop_state = LoopState::new(2);
    loop_state.delivery_messages.push(
        r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"not_observed","functions":["add","sub","mul"]}"#
            .to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 3 tests in 0.000s\nOK\n",
    ));
    let context = context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions"
    ]));
    let mut summary = None;

    assert!(attach_local_code_strict_json_projection(
        &task(),
        "Return JSON with changed_files, test_command, test_status, functions.",
        &mut loop_state,
        Some(&context),
        &mut summary,
    ));

    let value: serde_json::Value =
        serde_json::from_str(loop_state.delivery_messages[0].as_str()).expect("strict json");
    assert_eq!(value["test_status"], "passed");
    assert!(summary.is_some());
}

#[test]
fn local_code_projection_replaces_file_delivery_for_current_json_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.delivery_messages.push(
        "FILE:/workspace/project/test_calc_core.py\nfrom calc_core import safe_div".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "cd /workspace/project && python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/project/calc_core.py","resolved_path":"/workspace/project/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def safe_div(a, b):\n4|    if b == 0:\n5|        return {\"ok\": False, \"error_code\": \"division_by_zero\"}\n6|    return {\"ok\": True, \"value\": a / b}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/project/test_calc_core.py","resolved_path":"/workspace/project/test_calc_core.py","excerpt":"1|from calc_core import add, safe_div\n2|assert safe_div(1, 0) == {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 2 tests in 0.001s\nOK\n",
    ));
    let mut summary = None;
    let user_text = "读取刚才项目的 calc_core.py 和 test_calc_core.py，确认当前有哪些函数、safe_div 的除零错误码是什么，并重新运行 python3 test_calc_core.py。最后只输出 JSON，包含 project_dir、functions、error_codes、test_status、evidence_files。\n\n### ACTIVE_TASK_CONTEXT\nlast_primary_task_output:\n{\"changed_files\":[\"/workspace/project/calc_core.py\"],\"test_command\":\"python3 test_calc_core.py\"}";

    assert!(attach_local_code_strict_json_projection(
        &task(),
        user_text,
        &mut loop_state,
        None,
        &mut summary,
    ));

    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert!(!loop_state.delivery_messages[0].contains("FILE:"));
    let value: serde_json::Value =
        serde_json::from_str(&loop_state.delivery_messages[0]).expect("strict json");
    assert!(value.get("changed_files").is_none());
    assert!(value.get("test_command").is_none());
    assert_eq!(value["functions"], serde_json::json!(["add", "safe_div"]));
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
    assert_eq!(value["test_status"], "passed");
}
