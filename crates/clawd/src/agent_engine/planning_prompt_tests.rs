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
fn native_action_protocol_requires_capability_owned_structured_observations() {
    let overlay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts/layers/overlays/native_action_protocol.md");
    let prompt = std::fs::read_to_string(overlay).expect("read native action protocol");
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(prompt.contains("an authoritative structured operation owned by a matching runtime"));
    assert!(prompt.contains("call the `call_capability` function with that capability"));
    assert!(prompt.contains("When a structured parse, validation, preview, inspection"));
    assert!(normalized.contains("call that capability instead of substituting your own inference"));
    assert!(prompt.contains("A self-contained transformation whose"));
    assert!(prompt.contains("no runtime-owned validation, evidence, or effect is"));
    assert!(prompt.contains("A matching validation or guard capability owns the complete check"));
    assert!(prompt.contains("bounded raw reads that cover only part of the target"));
    assert!(prompt.contains("when a structured validator result explicitly requests supplementary"));
    assert!(prompt.contains("Do not call the capability again merely to confirm or"));
    assert!(prompt.contains("restate the same successful result"));
    assert!(prompt.contains("Copy the complete capability name exactly"));
    assert!(prompt.contains("Never derive a capability name by combining a skill name"));
    assert!(prompt.contains("machine arguments for ordering, filtering, or"));
    assert!(prompt.contains("bounded, already ordered observation"));
    assert!(prompt.contains("after context compaction"));
    assert!(prompt.contains("runtime delivery token (`FILE:<path>`"));
    assert!(prompt.contains("speculative claim about channel attachment support"));
    assert!(prompt.contains("A directory listing proves entry names and listed metadata"));
    assert!(prompt.contains("before asserting concrete current keys, members, values"));
    assert!(prompt.contains("is about the current workspace or project"));
    assert!(prompt.contains("inspect authoritative workspace"));
    assert!(prompt.contains("sources before composing it"));
    assert!(prompt.contains("an unobserved project name"));
    assert!(prompt.contains("current repository"));
    assert!(prompt.contains("facts. Direct creative drafting"));
    assert!(prompt.contains("combines list items with a"));
    assert!(prompt.contains("sibling explanation, conclusion, comparison"));
    assert!(prompt.contains("complete compound answer in `content`"));
    assert!(prompt.contains("mix non-empty `content` with list `items`"));
    assert!(prompt.contains("preserve each"));
    assert!(prompt.contains("scalar, object, or array shape"));
    assert!(prompt.contains("agent.subagent"));
    assert!(prompt.contains("agent.subagent_batch"));
    assert!(prompt.contains("agent.subagent_persistent"));
}

#[test]
fn planner_overlays_select_subagents_through_capabilities_only() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "single_plan_execution_prompt.md",
        "loop_incremental_plan_prompt.md",
        "plan_repair_prompt.md",
        "agent_tool_spec.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read planner overlay");
        assert!(
            prompt.contains("agent.subagent"),
            "{relative_path} must expose the registry capability"
        );
        assert!(
            prompt.contains("agent.subagent_batch"),
            "{relative_path} must expose the bounded batch capability"
        );
        assert!(
            !prompt.contains("\"tool\":\"subagent\""),
            "{relative_path} must not revive the retired direct-tool protocol"
        );
    }
}

#[test]
fn answer_verifier_distinguishes_listing_metadata_from_file_contents() {
    let overlay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts/layers/overlays/answer_verifier_prompt.md");
    let prompt = std::fs::read_to_string(overlay).expect("read answer verifier prompt");

    assert!(prompt.contains("Directory listing evidence proves entry names"));
    assert!(prompt.contains("does not prove current file contents"));
    assert!(prompt.contains("clearly generic or approximate type-level description"));
    assert!(prompt.contains("current keys, members, values, scripts, schemas"));
}

#[test]
fn answer_verifier_does_not_invent_configurable_token_mappings() {
    let overlay = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts/layers/overlays/answer_verifier_prompt.md");
    let prompt = std::fs::read_to_string(overlay).expect("read answer verifier prompt");

    assert!(prompt.contains("Do not invent semantic mappings"));
    assert!(prompt.contains("position in an `available_*` array"));
    assert!(prompt.contains("successful runtime call with an allowed replacement token"));
    assert!(prompt
        .contains("does not excuse a candidate that contradicts an explicit observed mapping"));
}

#[test]
fn planner_and_verifier_reject_truncated_search_as_exhaustive_inventory() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    let single = std::fs::read_to_string(overlays.join("single_plan_execution_prompt.md"))
        .expect("read single plan prompt");
    let incremental = std::fs::read_to_string(overlays.join("loop_incremental_plan_prompt.md"))
        .expect("read incremental plan prompt");
    let verifier = std::fs::read_to_string(overlays.join("answer_verifier_prompt.md"))
        .expect("read answer verifier prompt");

    assert!(single.contains("complete direct-child inventories"));
    assert!(single.contains("bounded recursive search"));
    assert!(incremental.contains("truncated=true"));
    assert!(incremental.contains("direct listing capability"));
    assert!(verifier.contains("truncated=true"));
    assert!(verifier.contains("non-truncated direct inventory"));
}

#[test]
fn final_synthesis_prompts_preserve_named_machine_field_shapes() {
    let overlays = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts/layers/overlays");
    for relative_path in [
        "observed_answer_fallback_prompt.md",
        "answer_verifier_prompt.md",
    ] {
        let prompt =
            std::fs::read_to_string(overlays.join(relative_path)).expect("read synthesis prompt");
        assert!(
            prompt.contains("explicitly names machine fields"),
            "{relative_path} must recognize explicit machine-field delivery"
        );
        assert!(
            prompt.contains("scalar, object, or array shape"),
            "{relative_path} must preserve structured value shapes"
        );
        assert!(
            prompt.contains("nested scalar"),
            "{relative_path} must reject collection flattening"
        );
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
    let mut loop_state = LoopState::new();
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
