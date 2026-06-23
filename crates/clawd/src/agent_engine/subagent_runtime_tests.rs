use super::*;
use crate::agent_engine::LoopState;

#[test]
fn subagent_action_records_safe_machine_observation() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;

    let stop_signal = record_subagent_action(
        &mut loop_state,
        3,
        2,
        "review",
        "Review the selected files for risk.",
        &[
            "step_1:evidence".to_string(),
            "unsafe natural ref with spaces".to_string(),
        ],
        SubagentActionOptions::default(),
    );

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["owner_layer"], "subagent_runtime");
    assert_eq!(observation["status"], "accepted");
    assert_eq!(observation["role"], "review");
    assert_eq!(observation["role_metadata"]["role_family"], "reviewer");
    assert_eq!(
        observation["role_metadata"]["tool_permission_profile"],
        "read_only"
    );
    assert_eq!(
        observation["role_metadata"]["result_contract_required"],
        true
    );
    assert_eq!(observation["timeout_policy"]["policy"], "bounded");
    assert_eq!(
        observation["timeout_policy"]["timeout_source"],
        "parent_loop_default"
    );
    assert_eq!(observation["cancellation_policy"]["cancellable"], true);
    assert_eq!(observation["execution_mode"], "inline_readonly_child_run");
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
    assert_eq!(observation["objective_present"], true);
    assert_eq!(observation["context_refs"][0]["ref"], "step_1:evidence");
    assert_eq!(observation["context_refs"][1]["ref"], "");
}

#[test]
fn subagent_action_rejects_unknown_role_as_machine_state() {
    let mut loop_state = LoopState::new(2);

    let stop_signal = record_subagent_action(
        &mut loop_state,
        1,
        1,
        "writer",
        "",
        &[],
        SubagentActionOptions::default(),
    );

    assert_eq!(stop_signal, Some(SUBAGENT_STOP_SIGNAL_INVALID_ROLE));
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["owner_layer"], "subagent_runtime");
    assert_eq!(observation["status"], "rejected");
    assert_eq!(observation["error_code"], "subagent_role_not_allowed");
    assert_eq!(observation["allowed_roles"][0], "observe");
    assert_eq!(observation["allowed_roles"][1], "explorer");
    assert_eq!(observation["allowed_roles"][6], "verifier");
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
}

#[test]
fn subagent_action_from_args_records_child_summary_and_machine_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 4;
    let args = serde_json::json!({
        "role": "test",
        "objective": "Run the scoped verification.",
        "parent_task_id": "task_123",
        "allowed_capabilities": ["filesystem.read", "bad token"],
        "budget": {
            "max_rounds": 1,
            "max_tool_calls": 2,
            "max_context_chars": 4096,
            "timeout_ms": 2500
        },
        "context_slice": {
            "refs": ["step_1:evidence:1", "unsafe ref"],
            "max_context_chars": 4096
        },
        "result_contract": {
            "status": "enum",
            "evidence_refs": "array"
        }
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 7, 3, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["child_run_id"], "subagent:4:3:test");
    assert_eq!(
        observation["allowed_capabilities"][0]["token"],
        "filesystem.read"
    );
    assert_eq!(observation["allowed_capabilities"][1]["token"], "");
    assert_eq!(observation["budget"]["max_tool_calls"], 2);
    assert_eq!(observation["timeout_policy"]["timeout_ms"], 2500);
    assert_eq!(
        observation["timeout_policy"]["terminal_status_on_timeout"],
        "timeout"
    );
    assert_eq!(
        observation["cancellation_policy"]["cancel_scope"],
        "child_run"
    );
    assert_eq!(observation["parent_task_ref"], "task_123");
    assert_eq!(
        observation["context_slice"]["refs"][0]["ref"],
        "step_1:evidence:1"
    );
    assert_eq!(observation["result_contract"]["kind"], "object");
    assert_eq!(
        observation["child_run_summary"]["trace_merge_status"],
        "merged"
    );
    assert_eq!(observation["child_request"]["state"], "completed");
    assert_eq!(
        observation["child_request"]["role_metadata"]["role_family"],
        "verifier"
    );
    assert_eq!(
        observation["child_request"]["timeout_policy"]["timeout_ms"],
        2500
    );
    assert_eq!(
        observation["child_request"]["execution_mode"],
        "inline_readonly_child_run"
    );
    assert_eq!(
        observation["child_request"]["request_ref"],
        "subagent:4:3:test"
    );
    assert_eq!(observation["scheduler"]["status"], "inline_completed");
    assert_eq!(
        observation["scheduler"]["reason_code"],
        "readonly_subagent_inline_execution"
    );
    assert_eq!(observation["scheduler"]["lease_required"], false);
    assert_eq!(observation["scheduler"]["checkpoint_required"], false);
    assert_eq!(
        observation["merge_contract"]["strategy"],
        "append_child_trace_summary"
    );
    assert_eq!(
        observation["merge_contract"]["child_trace_merge_status"],
        "merged"
    );
    assert_eq!(observation["child_result"]["status"], "completed");
    assert_eq!(observation["child_result"]["role_family"], "verifier");
    assert_eq!(
        observation["child_result"]["result_contract_required"],
        true
    );
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_inline_readonly_completed"
    );
    assert_eq!(observation["write_enabled"], false);
}

#[test]
fn subagent_new_role_tokens_preserve_readonly_policy() {
    let mut loop_state = LoopState::new(2);

    let stop_signal = record_subagent_action(
        &mut loop_state,
        1,
        1,
        "worker",
        "Collect bounded evidence.",
        &[],
        SubagentActionOptions::default(),
    );

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["role"], "worker");
    assert_eq!(observation["role_metadata"]["role_family"], "worker");
    assert_eq!(
        observation["role_metadata"]["default_scope"],
        "read_only_worker"
    );
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
    assert_eq!(
        observation["cancellation_policy"]["cancel_status"],
        "cancelled"
    );
}

#[test]
fn subagent_runtime_config_supplies_default_timeout_and_parallel_budget() {
    let mut loop_state = LoopState::new(2);
    let config = SubagentRuntimeConfig {
        allowed_roles: SubagentRole::all_tokens()
            .into_iter()
            .map(str::to_string)
            .collect(),
        max_parallel_readonly: 3,
        default_timeout_ms: Some(15_000),
    };

    let stop_signal = record_subagent_action_with_config(
        &mut loop_state,
        2,
        1,
        "explorer",
        "Collect read-only evidence.",
        &[],
        SubagentActionOptions::default(),
        &config,
    );

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["runtime_config"]["max_parallel_readonly"], 3);
    assert_eq!(observation["budget"]["default_timeout_ms"], 15_000);
    assert_eq!(observation["budget"]["effective_timeout_ms"], 15_000);
    assert_eq!(observation["timeout_policy"]["timeout_ms"], 15_000);
    assert_eq!(
        observation["timeout_policy"]["timeout_source"],
        "agent_guard.subagents.default_timeout_ms"
    );
    assert_eq!(observation["scheduler"]["max_parallel_readonly"], 3);
    assert_eq!(
        observation["child_request"]["runtime_config"]["default_timeout_ms"],
        15_000
    );
}

#[test]
fn subagent_runtime_config_rejects_disabled_role_as_machine_state() {
    let mut loop_state = LoopState::new(2);
    let config = SubagentRuntimeConfig {
        allowed_roles: vec!["observe".to_string()],
        max_parallel_readonly: 1,
        default_timeout_ms: Some(5_000),
    };

    let stop_signal = record_subagent_action_with_config(
        &mut loop_state,
        2,
        1,
        "review",
        "Review evidence.",
        &[],
        SubagentActionOptions::default(),
        &config,
    );

    assert_eq!(stop_signal, Some(SUBAGENT_STOP_SIGNAL_INVALID_ROLE));
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["status"], "rejected");
    assert_eq!(
        observation["error_code"],
        "subagent_role_disabled_by_config"
    );
    assert_eq!(observation["allowed_roles"][0], "observe");
    assert_eq!(observation["runtime_config"]["write_enabled"], false);
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
}

#[test]
fn subagent_batch_records_bounded_parallel_aggregation() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 5;
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "collect_file_refs",
                "context_refs": ["step_1:evidence"],
                "allowed_capabilities": ["filesystem.find_entries"],
                "findings": [
                    {
                        "kind": "file_ref",
                        "status": "found",
                        "message_key": "subagent.file_ref_found",
                        "evidence_refs": ["step_1:evidence"],
                        "text": "ignored user-visible prose"
                    }
                ]
            },
            {
                "role": "verifier",
                "objective": "verify_contract",
                "required": true,
                "context_slice": {
                    "refs": ["step_2:evidence"],
                    "max_context_chars": 2048
                },
                "result_contract": {
                    "status": "enum",
                    "evidence_refs": "array"
                },
                "findings": [
                    {
                        "kind": "contract",
                        "status": "ok",
                        "code": "verified",
                        "evidence_refs": ["step_2:evidence"],
                        "error_text": "ignored user-visible prose"
                    }
                ]
            }
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 9, 2, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(
        observation["execution_mode"],
        "bounded_parallel_readonly_child_runs"
    );
    assert_eq!(
        observation["scheduler"]["status"],
        "bounded_parallel_completed"
    );
    assert_eq!(
        observation["scheduler"]["reason_code"],
        "bounded_parallel_readonly_execution"
    );
    assert_eq!(observation["aggregation"]["status"], "completed");
    assert_eq!(observation["aggregation"]["child_count"], 2);
    assert_eq!(observation["aggregation"]["completed_count"], 2);
    assert_eq!(
        observation["aggregation"]["finding_refs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["kind"],
        "file_ref"
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["message_key"],
        "subagent.file_ref_found"
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["evidence_refs"][0],
        "step_1:evidence"
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|key| key["key"] == "text"),
        false
    );
    assert_eq!(
        observation["child_results"][1]["findings"][0]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|key| key["key"] == "error_text"),
        false
    );
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_parallel_readonly_completed"
    );
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
}

#[test]
fn subagent_batch_isolates_optional_child_failures_and_parallel_limit() {
    let mut loop_state = LoopState::new(2);
    let config = SubagentRuntimeConfig {
        allowed_roles: SubagentRole::all_tokens()
            .into_iter()
            .map(str::to_string)
            .collect(),
        max_parallel_readonly: 1,
        default_timeout_ms: Some(10_000),
    };
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "scheduled_optional_child"
            },
            {
                "role": "writer",
                "objective": "invalid_optional_child"
            },
            {
                "role": "worker",
                "objective": "over_parallel_budget_optional_child"
            }
        ]
    });

    let stop_signal =
        record_subagent_action_from_args_with_config(&mut loop_state, 3, 1, &args, &config);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["aggregation"]["status"], "partial");
    assert_eq!(observation["aggregation"]["completed_count"], 1);
    assert_eq!(observation["aggregation"]["rejected_count"], 1);
    assert_eq!(observation["aggregation"]["skipped_count"], 1);
    assert_eq!(observation["aggregation"]["optional_failed_count"], 2);
    assert_eq!(observation["aggregation"]["required_failed_count"], 0);
    assert_eq!(
        observation["child_results"][1]["error_code"],
        "subagent_role_not_allowed"
    );
    assert_eq!(
        observation["child_results"][2]["error_code"],
        "subagent_parallel_limit_exceeded"
    );
    assert_eq!(observation["failure_isolated"], true);
}

#[test]
fn subagent_batch_required_child_failure_stops_parent_loop() {
    let mut loop_state = LoopState::new(2);
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "optional_success"
            },
            {
                "role": "writer",
                "objective": "required_invalid_child",
                "required": true
            }
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 5, 1, &args);

    assert_eq!(
        stop_signal,
        Some(SUBAGENT_STOP_SIGNAL_REQUIRED_CHILD_FAILED)
    );
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["status"], "failed");
    assert_eq!(
        observation["aggregation"]["status"],
        "failed_required_child"
    );
    assert_eq!(observation["aggregation"]["required_failed_count"], 1);
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_required_child_failed"
    );
    assert_eq!(observation["failure_isolated"], false);
}
