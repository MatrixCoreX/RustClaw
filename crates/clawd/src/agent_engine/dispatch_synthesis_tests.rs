use super::*;
use crate::agent_engine::{AgentRunContext, LoopState};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, PlanKind,
    ResumeBehavior, RiskCeiling, ScheduleKind,
};
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

fn route_result_with_contract(
    response_shape: OutputResponseShape,
    delivery_required: bool,
) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "local code strict json".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        wants_file_delivery: delivery_required,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape,
            requires_content_evidence: true,
            delivery_required,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: if delivery_required {
                OutputDeliveryIntent::DirectoryBatchFiles
            } else {
                OutputDeliveryIntent::None
            },
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

fn agent_context_for_route(route_result: crate::RouteResult) -> AgentRunContext {
    AgentRunContext {
        route_result: Some(route_result),
        execution_recipe_hint: None,
        execution_recipe_plan_hint: None,
        turn_analysis: None,
        boundary_envelope: None,
        context_bundle_summary: None,
        session_alias_bindings: Vec::new(),
        auto_locator_path: None,
        original_user_request: None,
        user_request: None,
        cross_turn_recent_execution_context: None,
    }
}

fn agent_context_with_required_machine_fields(fields: serde_json::Value) -> AgentRunContext {
    let mut context = agent_context_for_route(route_result_with_contract(
        OutputResponseShape::Strict,
        false,
    ));
    context.turn_analysis = Some(crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(json!({ "required_machine_fields": fields })),
        attachment_processing_required: false,
    });
    context
}

#[test]
fn reusable_terminal_json_after_later_observation_preserves_prior_success_answer() {
    let terminal_answer = r#"{"created_files":["calc_core.py","test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "EXIT_CODE=0\nRan 2 tests in 0.000s\nOK",
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "synthesize_answer", terminal_answer));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "respond", terminal_answer));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"calc_core.py","excerpt":"1|def add(a,b): return a+b"}}"#,
    ));

    assert_eq!(
        reusable_terminal_json_after_later_observation(&loop_state).as_deref(),
        Some(terminal_answer)
    );
}

#[test]
fn reusable_terminal_json_requires_later_nonterminal_observation() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "synthesize_answer",
        r#"{"status":"ok"}"#,
    ));

    assert!(reusable_terminal_json_after_later_observation(&loop_state).is_none());
}

#[test]
fn reusable_terminal_json_rejects_unresolved_machine_values() {
    for answer in [
        r#"{"test_status":"not_observed"}"#,
        r#"{"created_files":["<missing>"]}"#,
        r#"{"created_files":null}"#,
        r#"{"answer":"{{last_output}}"}"#,
        r#"{"steps":[]}"#,
    ] {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "respond", answer));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "fs_basic",
            r#"{"extra":{"action":"read_range"}}"#,
        ));

        assert!(
            reusable_terminal_json_after_later_observation(&loop_state).is_none(),
            "answer should not be reusable: {answer}"
        );
    }
}

#[test]
fn strict_json_projection_answer_rejects_unresolved_machine_values() {
    let context = agent_context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status"
    ]));
    let answer = r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"not_observed"}"#;

    assert!(!strict_json_projection_answer_satisfies_request(
        "Return JSON with changed_files, test_command, test_status.",
        answer,
        Some(&context),
    ));
}

#[test]
fn strict_json_projection_answer_rejects_structural_error_code_tokens() {
    let context = agent_context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes"
    ]));
    let answer = r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed","functions":["safe_div"],"error_codes":["ok","division_by_zero"]}"#;

    assert!(!strict_json_projection_answer_satisfies_request(
        "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
        answer,
        Some(&context),
    ));
}

#[test]
fn local_code_task_projection_prefers_structured_required_machine_fields_over_user_surface() {
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
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 2 tests in 0.000s\nOK\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b"}}"#,
    ));
    let context = agent_context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status"
    ]));

    let answer = local_code_task_strict_json_projection(
        "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
        &loop_state,
        Some(&context),
    )
    .expect("projection should use structured required fields");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value.as_object().map(|object| object.len()),
        Some(3),
        "structured required_machine_fields should own the requested output shape"
    );
    assert!(value.get("functions").is_none());
    assert!(value.get("error_codes").is_none());
    assert_eq!(
        value["changed_files"],
        serde_json::json!(["/workspace/calc_core.py", "/workspace/test_calc_core.py"])
    );
    assert_eq!(value["test_command"], "python3 test_calc_core.py");
    assert_eq!(value["test_status"], "passed");
}

#[test]
fn local_code_task_projection_supports_verification_command_and_diff_summary() {
    let mut loop_state = LoopState::new(2);
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
        "......\n----------------------------------------------------------------------\nRan 6 tests in 0.000s\n\nOK\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","resolved_path":"/workspace/calc_core.py","size_bytes":70},"path":"/workspace/calc_core.py"},{"exists":true,"fact":{"kind":"file","resolved_path":"/workspace/test_calc_core.py","size_bytes":604},"path":"/workspace/test_calc_core.py"}]}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|\n4|def subtract(a, b):\n5|    return a - b"}}"#,
    ));
    loop_state.task_observations.push(json!({
        "owner_layer": "agent_hooks",
        "stage": "post_tool_use",
        "tool_or_skill": "run_cmd",
        "status": "ok",
        "args": {
            "command": "python3 test_calc_core.py",
            "cwd": "/workspace"
        }
    }));
    let context = agent_context_with_required_machine_fields(json!([
        "changed_files",
        "verification_command",
        "diff_summary"
    ]));

    let answer = local_code_task_strict_json_projection(
        "Report changed files, verification command, and concise diff summary.",
        &loop_state,
        Some(&context),
    )
    .expect("projection should include verification command and diff summary");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["changed_files"],
        serde_json::json!(["/workspace/calc_core.py", "/workspace/test_calc_core.py"])
    );
    assert_eq!(value["verification_command"], "python3 test_calc_core.py");
    assert_eq!(
        value["diff_summary"][0]["summary_code"],
        "source_file_updated"
    );
    assert_eq!(value["diff_summary"][0]["size_bytes"], 70);
    assert_eq!(
        value["diff_summary"][0]["functions"],
        serde_json::json!(["add", "subtract"])
    );
    assert_eq!(
        value["diff_summary"][1]["summary_code"],
        "test_file_updated"
    );
    assert_eq!(value["diff_summary"][1]["size_bytes"], 604);
    assert!(strict_json_projection_answer_satisfies_request(
        "Report changed files, verification command, and concise diff summary.",
        &answer,
        Some(&context),
    ));
}

#[test]
fn local_code_task_projection_builds_created_files_test_command_and_status() {
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
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "run_cmd", "All tests passed.\n"));

    let answer = local_code_task_strict_json_projection(
        "最后只输出 JSON，包含 created_files、test_command、test_status。",
        &loop_state,
        None,
    )
    .expect("projection should be grounded");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["created_files"],
        serde_json::json!(["/workspace/calc_core.py", "/workspace/test_calc_core.py"])
    );
    assert_eq!(value["test_command"], "python3 test_calc_core.py");
    assert_eq!(value["test_status"], "passed");
    assert!(strict_json_projection_answer_satisfies_request(
        "最后只输出 JSON，包含 created_files、test_command、test_status。",
        &answer,
        None,
    ));
}

#[test]
fn local_code_task_projection_uses_legacy_write_file_as_changed_file_evidence() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "write_file",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "run_cmd", "All tests passed.\n"));

    let answer = local_code_task_strict_json_projection(
        "Return JSON with changed_files, test_command, test_status.",
        &loop_state,
        None,
    )
    .expect("legacy write_file should project changed files");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["changed_files"],
        serde_json::json!(["/workspace/calc_core.py"])
    );
    assert_eq!(value["test_command"], "python3 test_calc_core.py");
    assert_eq!(value["test_status"], "passed");
}

#[test]
fn local_code_task_projection_preserves_multiple_validation_commands() {
    let mut loop_state = LoopState::new(2);
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
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def safe_div(a, b):\n4|    return {'ok': false, 'error_code': 'division_by_zero'}"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_4", "run_cmd", "All tests passed.\n"));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "run_cmd",
        "{'ok': False, 'error_code': 'division_by_zero'}\n",
    ));

    let answer = local_code_task_strict_json_projection(
        "最后只输出 JSON，包含 changed_files、test_command、test_status、functions、error_codes。",
        &loop_state,
        None,
    )
    .expect("projection should include both validation commands");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["test_command"],
        serde_json::json!([
            "python3 test_calc_core.py",
            "python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1,0))\nPY"
        ])
    );
    assert_eq!(value["functions"], serde_json::json!(["add", "safe_div"]));
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
}

#[test]
fn local_code_task_projection_includes_failing_command_repair_fields() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "cd /workspace/project && python3 test_calc_core.py; echo \"EXIT_CODE=$?\"".to_string(),
    );
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "observe failing command, fix code, and validate".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: "observe failing command, fix code, and validate".to_string(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                steps: vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "cd /workspace/project && python3 test_calc_core.py; echo \"EXIT_CODE=$?\"",
                        }),
                        depends_on: Vec::new(),
                        why: "observe failure".to_string(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "cd /workspace/project && python3 test_calc_core.py; echo \"EXIT_CODE=$?\"",
                        }),
                        depends_on: vec!["step_1".to_string()],
                        why: "validate fix".to_string(),
                    },
                ],
                planner_notes: String::new(),
                plan_kind: PlanKind::Single,
                raw_plan_text: String::new(),
            }),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/project/calc_core.py","resolved_path":"/workspace/project/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/project/test_calc_core.py","resolved_path":"/workspace/project/test_calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "EXIT_CODE=1\nAssertionError: expected division_by_zero\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/project/calc_core.py","resolved_path":"/workspace/project/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "run_cmd",
        "All tests passed\nEXIT_CODE=0\n",
    ));
    let context = agent_context_with_required_machine_fields(json!([
        "project_dir",
        "changed_files",
        "failed_command",
        "failure_observed",
        "failure_evidence",
        "fix_summary",
        "test_command",
        "test_status"
    ]));

    let answer = local_code_task_strict_json_projection(
        "Return JSON with project_dir, changed_files, failed_command, failure_observed, failure_evidence, fix_summary, test_command, test_status.",
        &loop_state,
        Some(&context),
    )
    .expect("projection should include failure and repair evidence");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(value["failure_observed"], true);
    assert_eq!(
        value["failed_command"],
        "cd /workspace/project && python3 test_calc_core.py; echo \"EXIT_CODE=$?\""
    );
    assert_eq!(value["failure_evidence"]["exit_code"], 1);
    assert_eq!(
        value["fix_summary"]["status_code"],
        "post_failure_validation_passed"
    );
    assert_eq!(value["fix_summary"]["validation_exit_code"], 0);
    assert_eq!(value["test_status"], "passed");
    assert!(strict_json_projection_answer_satisfies_request(
        "Return JSON with project_dir, changed_files, failed_command, failure_observed, failure_evidence, fix_summary, test_command, test_status.",
        &answer,
        Some(&context),
    ));
}

#[test]
fn local_code_task_projection_uses_plan_trace_run_cmd_commands() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "update and validate local code".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: "update and validate local code".to_string(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                steps: vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "python3 test_calc_core.py",
                        }),
                        depends_on: Vec::new(),
                        why: "validate".to_string(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1, 0))\nPY",
                        }),
                        depends_on: vec!["step_1".to_string()],
                        why: "probe".to_string(),
                    },
                ],
                planner_notes: String::new(),
                plan_kind: PlanKind::Single,
                raw_plan_text: String::new(),
            }),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "run_cmd", "ok\n"));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "{'ok': False, 'error_code': 'division_by_zero'}\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b): return a + b\n2|def sub(a, b): return a - b\n3|def mul(a, b): return a * b\n4|def safe_div(a, b):\n5|    return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    let context = agent_context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes"
    ]));

    let answer = local_code_task_strict_json_projection(
        "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
        &loop_state,
        Some(&context),
    )
    .expect("projection should recover run_cmd commands from plan trace");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["test_command"],
        serde_json::json!([
            "python3 test_calc_core.py",
            "python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1, 0))\nPY"
        ])
    );
    assert_eq!(value["test_status"], "passed");
    assert_eq!(
        value["functions"],
        serde_json::json!(["add", "sub", "mul", "safe_div"])
    );
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
}

#[test]
fn local_code_task_projection_allows_strict_json_despite_delivery_hint() {
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
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 2 tests in 0.000s\nOK\n",
    ));
    let strict_delivery_context = agent_context_for_route(route_result_with_contract(
        OutputResponseShape::Strict,
        true,
    ));

    let answer = local_code_task_strict_json_projection(
        "最后只输出 JSON，包含 created_files、test_command、test_status。",
        &loop_state,
        Some(&strict_delivery_context),
    )
    .expect("strict json projection should survive delivery hint drift");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");
    assert_eq!(
        value["created_files"],
        serde_json::json!(["/workspace/calc_core.py", "/workspace/test_calc_core.py"])
    );
    assert_eq!(value["test_command"], "python3 test_calc_core.py");
    assert_eq!(value["test_status"], "passed");

    let file_token_context = agent_context_for_route(route_result_with_contract(
        OutputResponseShape::FileToken,
        true,
    ));
    assert!(local_code_task_strict_json_projection(
        "最后只输出 JSON，包含 created_files、test_command、test_status。",
        &loop_state,
        Some(&file_token_context),
    )
    .is_none());

    let mut executable_file_token_route =
        route_result_with_contract(OutputResponseShape::FileToken, true);
    executable_file_token_route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    executable_file_token_route.route_reason =
        "executable_contract_preserved_for_agent_loop".to_string();
    let executable_file_token_context = agent_context_for_route(executable_file_token_route);
    let answer = local_code_task_strict_json_projection(
        "最后只输出 JSON，包含 created_files、test_command、test_status。",
        &loop_state,
        Some(&executable_file_token_context),
    )
    .expect("executable local code JSON should survive noisy file-token shape");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");
    assert_eq!(
        value["created_files"],
        serde_json::json!(["/workspace/calc_core.py", "/workspace/test_calc_core.py"])
    );
}

#[test]
fn local_code_task_projection_uses_current_request_fields_before_context_blocks() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "cd /workspace/project && python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/project/calc_core.py","resolved_path":"/workspace/project/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b\n7|def safe_div(a, b):\n8|    if b == 0:\n9|        return {\"ok\": False, \"error_code\": \"division_by_zero\"}\n10|    return {\"ok\": True, \"value\": a / b}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"/workspace/project/test_calc_core.py","resolved_path":"/workspace/project/test_calc_core.py","excerpt":"1|from calc_core import add, sub, mul, safe_div\n2|assert safe_div(1, 0) == {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 5 tests in 0.001s\nOK\n",
    ));
    let mut route = route_result_with_contract(OutputResponseShape::FileToken, true);
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.route_reason = "executable_contract_preserved_for_agent_loop".to_string();
    let context = agent_context_for_route(route);
    let augmented_user_text = "读取刚才项目的 calc_core.py 和 test_calc_core.py，确认当前有哪些函数、safe_div 的除零错误码是什么，并重新运行 python3 test_calc_core.py。最后只输出 JSON，包含 project_dir、functions、error_codes、test_status、evidence_files。\n\n### ACTIVE_TASK_CONTEXT\nlast_primary_task_output:\n{\"changed_files\":[\"/workspace/project/calc_core.py\"],\"test_command\":\"python3 test_calc_core.py\",\"test_status\":\"passed\"}";

    let answer =
        local_code_task_strict_json_projection(augmented_user_text, &loop_state, Some(&context))
            .expect("inspect-only local code request should project strict JSON from readbacks");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");
    let keys = value
        .as_object()
        .expect("object")
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        keys,
        [
            "error_codes".to_string(),
            "evidence_files".to_string(),
            "functions".to_string(),
            "project_dir".to_string(),
            "test_status".to_string(),
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(
        value["functions"],
        serde_json::json!(["add", "sub", "mul", "safe_div"])
    );
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
    assert_eq!(value["test_status"], "passed");
    assert!(strict_json_projection_answer_satisfies_request(
        augmented_user_text,
        &answer,
        Some(&context),
    ));
}

#[test]
fn local_code_task_projection_uses_successful_write_plan_content_for_code_fields() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "write and validate local code".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: "write and validate local code".to_string(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                steps: vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "fs_basic".to_string(),
                        args: json!({
                            "action": "write_text",
                            "path": "/workspace/calc_core.py",
                            "content": "def add(a, b):\n    return a + b\n\n\ndef safe_div(a, b):\n    if b == 0:\n        return {\"ok\": False, \"error_code\": \"division_by_zero\"}\n    return {\"ok\": True, \"value\": a / b}\n",
                        }),
                        depends_on: Vec::new(),
                        why: "write source".to_string(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "python3 test_calc_core.py",
                        }),
                        depends_on: vec!["step_1".to_string()],
                        why: "validate".to_string(),
                    },
                ],
                planner_notes: String::new(),
                plan_kind: PlanKind::Single,
                raw_plan_text: String::new(),
            }),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "run_cmd", "ok\n"));
    let context = agent_context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes"
    ]));

    let answer = local_code_task_strict_json_projection(
        "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
        &loop_state,
        Some(&context),
    )
    .expect("successful write action content should provide code field evidence");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(value["functions"], serde_json::json!(["add", "safe_div"]));
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
    assert_eq!(value["test_status"], "passed");
}

#[test]
fn local_code_task_projection_uses_last_machine_field_segment_only() {
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
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "run_cmd",
        "Ran 2 tests in 0.000s\nOK\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b"}}"#,
    ));

    let answer = local_code_task_strict_json_projection(
        "Create tests covering both functions. Return JSON with created_files, test_command, test_status.",
        &loop_state,
        None,
    )
    .expect("projection should use the final machine-field segment");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");
    assert_eq!(
        value.as_object().map(|object| object.len()),
        Some(3),
        "ordinary prose mentioning functions must not add an unrequested JSON field"
    );
    assert!(value.get("functions").is_none());
    assert_eq!(
        value["created_files"],
        serde_json::json!(["/workspace/calc_core.py", "/workspace/test_calc_core.py"])
    );
    assert_eq!(value["test_command"], "python3 test_calc_core.py");
    assert_eq!(value["test_status"], "passed");
}

#[test]
fn local_code_task_projection_refuses_unobserved_content_fields() {
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
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "run_cmd", "All tests passed.\n"));

    assert!(local_code_task_strict_json_projection(
        "最后只输出 JSON，包含 changed_files、test_command、test_status、functions。",
        &loop_state,
        None,
    )
    .is_none());
}

#[test]
fn local_code_task_projection_uses_readbacks_for_functions_errors_and_evidence_files() {
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
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def safe_div(a, b):\n4|    return {'ok': false, 'error_code': 'division_by_zero'}"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "run_cmd", "All tests passed.\n"));

    let answer = local_code_task_strict_json_projection(
        "最后只输出 JSON，包含 changed_files、test_command、test_status、functions、error_codes、evidence_files。",
        &loop_state,
        None,
    )
    .expect("projection should use readbacks");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["changed_files"],
        serde_json::json!(["/workspace/calc_core.py"])
    );
    assert_eq!(value["functions"], serde_json::json!(["add", "safe_div"]));
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
    assert_eq!(
        value["evidence_files"],
        serde_json::json!(["/workspace/calc_core.py"])
    );
}

#[test]
fn local_code_task_projection_uses_post_write_readbacks_for_functions() {
    let mut loop_state = LoopState::new(2);
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
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|def test_add_positive(self):\n2|    pass\n3|def test_sub_positive(self):\n4|    pass"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "run_cmd",
        "Ran 3 tests in 0.000s\nOK\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_6",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_7",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|def test_add(self):\n2|    pass\n3|def test_sub(self):\n4|    pass\n5|def test_mul(self):\n6|    pass"}}"#,
    ));

    let answer = local_code_task_strict_json_projection(
        "最后只输出 JSON，包含 changed_files、test_command、test_status、functions。",
        &loop_state,
        None,
    )
    .expect("projection should use post-write readbacks");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(value["functions"], serde_json::json!(["add", "sub", "mul"]));
}

#[test]
fn local_code_task_projection_supplements_partial_source_readback_from_test_imports() {
    let mut loop_state = LoopState::new(2);
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
        r#"{"extra":{"action":"read_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|import sys\n2|from calc_core import add, sub, mul, safe_div\n3|from other_module import ignored\n4|def test_safe_div_zero(): pass"}}"#,
    ));

    let context = agent_context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes"
    ]));
    let answer = local_code_task_strict_json_projection(
        "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
        &loop_state,
        Some(&context),
    )
    .expect("projection should combine source and test import evidence");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["functions"],
        serde_json::json!(["add", "sub", "mul", "safe_div"])
    );
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
    assert_eq!(
        value["test_command"],
        serde_json::json!([
            "python3 test_calc_core.py",
            "python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1,0))\nPY"
        ])
    );
}

#[test]
fn local_code_task_projection_prefers_unwritten_source_readback_over_test_functions() {
    let mut loop_state = LoopState::new(2);
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
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b\n7|def safe_div(a, b):\n8|    if b == 0:\n9|        return {\"ok\": False, \"error_code\": \"division_by_zero\"}\n10|    return {\"ok\": True, \"value\": a / b}"}}"#,
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
        r#"{"ok":false,"error_code":"division_by_zero"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub, mul, safe_div\n2|def test_add(): pass\n3|def test_sub(): pass\n4|def test_mul(): pass\n5|def test_safe_div_normal(): pass\n6|def test_safe_div_by_zero(): pass"}}"#,
    ));
    let context = agent_context_with_required_machine_fields(serde_json::json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes",
        "evidence_files"
    ]));

    let answer = local_code_task_strict_json_projection(
        "Return JSON with changed_files, test_command, test_status, functions, error_codes, evidence_files.",
        &loop_state,
        Some(&context),
    )
    .expect("projection should combine source readback with test write readback");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["changed_files"],
        serde_json::json!(["/workspace/test_calc_core.py"])
    );
    assert_eq!(
        value["functions"],
        serde_json::json!(["add", "sub", "mul", "safe_div"])
    );
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
    assert_eq!(
        value["evidence_files"],
        serde_json::json!(["/workspace/test_calc_core.py", "/workspace/calc_core.py"])
    );
}

#[test]
fn local_code_task_projection_excludes_noop_writes_from_changed_files() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "agent_loop.run_cmd_commands".to_string(),
        serde_json::json!(["python3 test_calc_core.py"]).to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","changed":false,"noop":true}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","changed":true,"noop":false}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def safe_div(a, b):\n4|    if b == 0:\n5|        return {\"ok\": False, \"error_code\": \"division_by_zero\"}\n6|    return {\"ok\": True, \"value\": a / b}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, safe_div\n2|assert safe_div(1, 0) == {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_5", "run_cmd", "passed\n"));
    let context = agent_context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes"
    ]));

    let answer = local_code_task_strict_json_projection(
        "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
        &loop_state,
        Some(&context),
    )
    .expect("projection should keep no-op write out of changed files");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["changed_files"],
        serde_json::json!(["/workspace/test_calc_core.py"])
    );
    assert_eq!(value["functions"], serde_json::json!(["add", "safe_div"]));
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
}

#[test]
fn local_code_task_projection_uses_code_readbacks_when_no_current_write_exists() {
    let mut loop_state = LoopState::new(2);
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
        "system_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b\n3|def sub(a, b):\n4|    return a - b\n5|def mul(a, b):\n6|    return a * b\n7|def safe_div(a, b):\n8|    if b == 0:\n9|        return {\"ok\": False, \"error_code\": \"division_by_zero\"}\n10|    return {\"ok\": True, \"value\": a / b}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "system_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, sub, mul, safe_div\n2|def test_safe_div_normal(): pass\n3|def test_safe_div_zero(): pass"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "run_cmd", "ALL_TESTS_PASSED\n"));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "run_cmd",
        r#"{"ok":false,"error_code":"division_by_zero"}"#,
    ));
    let context = agent_context_with_required_machine_fields(json!([
        "changed_files",
        "test_command",
        "test_status",
        "functions",
        "error_codes"
    ]));

    let answer = local_code_task_strict_json_projection(
        "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
        &loop_state,
        Some(&context),
    )
    .expect("projection should use readbacks when code is already in target state");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("json");

    assert_eq!(
        value["changed_files"],
        serde_json::json!(["/workspace/calc_core.py", "/workspace/test_calc_core.py"])
    );
    assert_eq!(
        value["functions"],
        serde_json::json!(["add", "sub", "mul", "safe_div"])
    );
    assert_eq!(
        value["error_codes"],
        serde_json::json!(["division_by_zero"])
    );
    assert_eq!(value["test_status"], "passed");
}

#[test]
fn local_code_task_projection_refuses_created_files_without_current_write() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a, b):\n2|    return a + b"}}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "run_cmd", "ALL_TESTS_PASSED\n"));
    let context = agent_context_with_required_machine_fields(json!([
        "created_files",
        "test_command",
        "test_status",
        "functions"
    ]));

    assert!(local_code_task_strict_json_projection(
        "Return JSON with created_files, test_command, test_status, functions.",
        &loop_state,
        Some(&context),
    )
    .is_none());
}
