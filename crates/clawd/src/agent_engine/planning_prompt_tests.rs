use super::build_incremental_plan_prompt;
use crate::agent_engine::{attempt_ledger::build_attempt_ledger_compact, LoopState};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use serde_json::json;
use std::path::Path;

#[test]
fn planner_overlays_expand_high_cardinality_placeholders_once() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    let contracts = [
        (
            "single_plan_execution_prompt.md",
            [
                "__GOAL__",
                "__USER_REQUEST__",
                "__TOOL_SPEC__",
                "__SKILL_PLAYBOOKS__",
                "__RECENT_ASSISTANT_REPLIES__",
            ]
            .as_slice(),
        ),
        (
            "loop_incremental_plan_prompt.md",
            [
                "__GOAL__",
                "__USER_REQUEST__",
                "__TOOL_SPEC__",
                "__SKILL_PLAYBOOKS__",
                "__RECENT_ASSISTANT_REPLIES__",
                "__HISTORY_COMPACT__",
                "__ATTEMPT_LEDGER__",
                "__LAST_ROUND_OUTPUT__",
            ]
            .as_slice(),
        ),
    ];

    for (relative_path, placeholders) in contracts {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read prompt overlay");
        for placeholder in placeholders {
            assert_eq!(
                prompt.matches(placeholder).count(),
                1,
                "{relative_path} must expand {placeholder} exactly once"
            );
        }
    }
}

#[test]
fn planner_overlays_require_runtime_observation_for_policy_projections() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read prompt overlay");
        assert!(
            prompt.contains(
                "Current permission, policy, risk, confirmation, sandbox, or approval projections are runtime observations"
            ),
            "{relative_path} must require a runtime-owned policy projection"
        );
        assert!(
            prompt.contains("never replace it with a guessed `respond`"),
            "{relative_path} must reject planner-invented permission decisions"
        );
        assert!(
            prompt.contains("Prefer a matching dedicated read-only preview/preflight capability"),
            "{relative_path} must prefer the runtime-owned preview contract"
        );
        assert!(
            prompt.contains(
                "Never simulate a no-mutation preview by running a mutating shell command"
            ),
            "{relative_path} must reject side-effecting preview simulation"
        );
    }
}

#[test]
fn planner_overlays_keep_independent_reads_out_of_shell_pipelines() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read prompt overlay");
        assert!(
            prompt.contains("Independent known targets are not cross-step dependencies"),
            "{relative_path} must distinguish independent reads from dependent execution"
        );
        assert!(
            prompt.contains("Do not collapse independent reads into `run_cmd`"),
            "{relative_path} must keep known read targets on dedicated capabilities"
        );
        assert!(
            prompt.contains(
                "This exception never applies to independent targets whose paths or selectors are already known"
            ),
            "{relative_path} must close the shell dependency exception"
        );
    }
}

#[test]
fn planner_overlays_reserve_scalar_shape_for_one_atomic_result() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read prompt overlay");
        assert!(
            prompt.contains(
                "`response_shape=\"scalar\"` is valid only when the complete final answer is exactly one atomic value"
            ),
            "{relative_path} must keep scalar projection atomic"
        );
        assert!(
            prompt.contains(
                "compound result and must use `free` or `strict` so every requested deliverable survives final"
            ),
            "{relative_path} must preserve compound final answers"
        );
    }
}

#[test]
fn incremental_prompt_carries_structured_failed_attempt_for_planner_repair() {
    let mut loop_state = LoopState::new(3);
    let err = crate::skills::structured_skill_error_from_parts(
        "fs_basic",
        "missing_required_field",
        "missing_required_field",
        None,
        Some(json!({
            "error_code": "missing_required_field",
            "missing_evidence_fields": ["path"],
            "message_key": "clawd.skill.missing_required_field"
        })),
    );
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(err),
        started_at: 100,
        finished_at: 110,
    });

    let attempt_ledger = build_attempt_ledger_compact(&loop_state);
    let prompt = build_incremental_plan_prompt(
        "ledger=__ATTEMPT_LEDGER__\nlast=__LAST_ROUND_OUTPUT__\nround=__ROUND__",
        "read project file",
        "read project file",
        "turn_analysis",
        "tool_spec",
        "skill_playbooks",
        "",
        "auto",
        "zh-CN",
        "rustclaw",
        2,
        "history",
        &attempt_ledger,
        "last round failed",
        "linux",
        "bash",
        "/workspace",
    );

    assert!(prompt.contains("\"tool_or_skill\": \"fs_basic\""));
    assert!(prompt.contains("\"status\": \"error\""));
    assert!(prompt.contains("\"error_code\": \"missing_required_field\""));
    assert!(prompt.contains("\"missing_evidence\": ["));
    assert!(prompt.contains("\"path\""));
    assert!(prompt.contains("\"recovery_action\": \"collect_missing_evidence\""));
    assert!(prompt.contains("\"repair_class\": \"loop_bounded_recovery\""));
    assert!(prompt.contains("\"next_recovery_kind\": \"wait_background\""));
    assert!(prompt.contains("\"forbidden_repeat_signature\""));
    assert!(prompt.contains("round=2"));
}
