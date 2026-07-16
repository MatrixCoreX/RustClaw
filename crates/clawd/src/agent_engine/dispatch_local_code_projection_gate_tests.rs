use serde_json::json;

use super::*;
use crate::agent_engine::AgentRunContext;
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    ResumeBehavior, RiskCeiling, ScheduleKind,
};

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

fn context_with_machine_fields(
    response_shape: OutputResponseShape,
    fields: &[&str],
) -> AgentRunContext {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "local code strict json".to_string(),
        needs_clarify: false,
        route_reason: "executable_contract_preserved_for_agent_loop".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: response_shape == OutputResponseShape::FileToken,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    AgentRunContext {
        route_result: Some(route),
        execution_recipe_hint: None,
        execution_recipe_plan_hint: None,
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(json!({
                "required_machine_fields": fields
            })),
            attachment_processing_required: false,
        }),
        boundary_envelope: None,
        context_bundle_summary: None,
        session_alias_bindings: Vec::new(),
        auto_locator_path: None,
        original_user_request: None,
        user_request: None,
        cross_turn_recent_execution_context: None,
    }
}

fn context_with_required_machine_fields(response_shape: OutputResponseShape) -> AgentRunContext {
    context_with_machine_fields(
        response_shape,
        &[
            "changed_files",
            "test_command",
            "test_status",
            "functions",
            "error_codes",
        ],
    )
}

fn loop_state_with_writes_and_validation() -> LoopState {
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
    loop_state
}

#[test]
fn local_code_projection_defer_gate_triggers_for_missing_post_write_readback() {
    let loop_state = loop_state_with_writes_and_validation();
    let context = context_with_required_machine_fields(OutputResponseShape::Strict);

    assert!(
        local_code_strict_json_projection_should_defer_observed_synthesis(
            "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}

#[test]
fn local_code_projection_defer_gate_triggers_for_legacy_write_file_missing_readback() {
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
    let context = context_with_required_machine_fields(OutputResponseShape::Strict);

    assert!(
        local_code_strict_json_projection_should_defer_observed_synthesis(
            "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}

#[test]
fn local_code_projection_defer_gate_stops_after_readback_completes_projection() {
    let mut loop_state = loop_state_with_writes_and_validation();
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a,b): return a+b\n2|def safe_div(a,b):\n3|    return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, safe_div\n2|def test_safe_div_zero(): pass"}}"#,
    ));
    let context = context_with_required_machine_fields(OutputResponseShape::Strict);

    assert!(
        !local_code_strict_json_projection_should_defer_observed_synthesis(
            "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}

#[test]
fn local_code_projection_defer_gate_requires_validation() {
    let mut loop_state = loop_state_with_writes_and_validation();
    loop_state
        .executed_step_results
        .retain(|step| step.skill != "run_cmd");
    let context = context_with_required_machine_fields(OutputResponseShape::Strict);

    assert!(
        !local_code_strict_json_projection_should_defer_observed_synthesis(
            "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}

#[test]
fn local_code_projection_defer_gate_ignores_file_token_contracts() {
    let loop_state = loop_state_with_writes_and_validation();
    let context = context_with_required_machine_fields(OutputResponseShape::FileToken);

    assert!(
        !local_code_strict_json_projection_should_defer_observed_synthesis(
            "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}

fn loop_state_with_code_readbacks_without_validation() -> LoopState {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py","excerpt":"1|def add(a,b): return a+b\n2|def safe_div(a,b):\n3|    return {\"ok\": False, \"error_code\": \"division_by_zero\"}"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py","excerpt":"1|from calc_core import add, safe_div\n2|def test_safe_div_zero(): pass"}}"#,
    ));
    loop_state
}

#[test]
fn local_code_projection_defer_until_validation_blocks_readback_snippet_candidate() {
    let loop_state = loop_state_with_code_readbacks_without_validation();
    let context = context_with_required_machine_fields(OutputResponseShape::Strict);

    assert!(
        local_code_strict_json_projection_should_defer_until_validation(
            "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}

#[test]
fn local_code_projection_defer_until_validation_blocks_post_write_candidate() {
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
    let context = context_with_required_machine_fields(OutputResponseShape::Strict);

    assert!(
        local_code_strict_json_projection_should_defer_until_validation(
            "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}

#[test]
fn local_code_projection_defer_until_validation_stops_after_validation() {
    let mut loop_state = loop_state_with_code_readbacks_without_validation();
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        "python3 test_calc_core.py".to_string(),
    );
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "run_cmd", "all tests passed\n"));
    let context = context_with_required_machine_fields(OutputResponseShape::Strict);

    assert!(
        !local_code_strict_json_projection_should_defer_until_validation(
            "Return JSON with changed_files, test_command, test_status, functions, error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}

#[test]
fn local_code_projection_defer_until_validation_requires_validation_field() {
    let loop_state = loop_state_with_code_readbacks_without_validation();
    let context =
        context_with_machine_fields(OutputResponseShape::Strict, &["functions", "error_codes"]);

    assert!(
        !local_code_strict_json_projection_should_defer_until_validation(
            "Return JSON with functions and error_codes.",
            &loop_state,
            Some(&context),
        )
    );
}
