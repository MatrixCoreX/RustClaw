use super::*;
use crate::agent_engine::LoopState;
use std::path::{Path, PathBuf};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw-subagent-runtime-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

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
fn subagent_action_projects_workspace_context_evidence() {
    let temp = TempDirGuard::new("context-evidence");
    std::fs::create_dir_all(temp.path().join("plan")).expect("create plan dir");
    let long_agents = format!(
        "runtime boundary\napi_key = should_not_leak\n{}\nlate runtime boundary\nsecret = should_not_leak_late\n",
        "filler line\n".repeat(300)
    );
    std::fs::write(temp.path().join("AGENTS.md"), long_agents).expect("write agents");
    std::fs::write(
        temp.path().join("plan/current.md"),
        "plan boundary\nsubagent review stays read only\n",
    )
    .expect("write plan");

    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 8;
    let config = SubagentRuntimeConfig {
        context_evidence_root: Some(temp.path().to_path_buf()),
        ..SubagentRuntimeConfig::default()
    };
    let args = serde_json::json!({
        "role": "review",
        "objective": "runtime_boundary_alignment_audit",
        "context_refs": ["AGENTS.md", "plan/current.md"],
        "context_slice": {
            "max_context_chars": 1024
        },
        "result_contract": {
            "output_format": "machine_json",
            "content_excerpt": "string"
        }
    });

    let stop_signal =
        record_subagent_action_from_args_with_config(&mut loop_state, 10, 1, &args, &config);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["output_format"], "machine_json");
    assert_eq!(observation["action"], "read_text_range");
    assert_eq!(observation["path"], "AGENTS.md");
    assert_eq!(observation["paths"].as_array().unwrap().len(), 2);
    assert_eq!(observation["context_evidence"]["present"], true);
    assert_eq!(observation["context_evidence"]["available_count"], 2);
    assert_eq!(
        observation["context_evidence"]["items"][0]["path"],
        "AGENTS.md"
    );
    assert_eq!(
        observation["context_evidence"]["items"][1]["path"],
        "plan/current.md"
    );
    let excerpt = observation["content_excerpt"].as_str().unwrap();
    assert!(excerpt.contains("runtime boundary"));
    assert!(excerpt.contains("late runtime boundary"));
    assert!(excerpt.contains("plan boundary"));
    assert!(excerpt.contains("[REDACTED_SENSITIVE_LINE]"));
    assert!(!excerpt.contains("should_not_leak"));
    assert!(!excerpt.contains("should_not_leak_late"));
    assert_eq!(
        observation["context_evidence"]["items"][0]["excerpt_strategy"],
        "head_tail"
    );
    assert_eq!(observation["child_result"]["content_excerpt_present"], true);
}

#[test]
fn subagent_model_child_result_merges_into_runtime_observation() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 3;
    let args = serde_json::json!({
        "role": "review",
        "objective": "machine_boundary_review",
        "context_refs": [],
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 4, 1, &args);
    assert!(stop_signal.is_none());
    let merged = apply_model_assisted_child_result_for_test(
        &mut loop_state,
        4,
        1,
        serde_json::json!({
            "schema_version": 1,
            "owner_layer": "subagent_model_child",
            "output_format": "machine_json",
            "status": "completed",
            "findings": [{"code": "boundary_consistent"}],
            "evidence_refs": ["AGENTS.md"],
            "confidence": 0.77
        }),
    );

    assert!(merged);
    let observation = &loop_state.task_observations[0];
    assert_eq!(
        observation["execution_mode"],
        "model_assisted_readonly_child_run"
    );
    assert_eq!(observation["action"], "subagent_model_child");
    assert_eq!(observation["model_assisted"], true);
    assert_eq!(observation["child_result"]["model_assisted"], true);
    assert_eq!(observation["child_result"]["result_status"], "completed");
    assert_eq!(
        observation["child_model_result"]["findings"][0]["code"],
        "boundary_consistent"
    );
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
        context_evidence_root: None,
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
        context_evidence_root: None,
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
                        "confidence": 0.82,
                        "evidence_refs": ["step_1:evidence"],
                        "text": "ignored user-visible prose"
                    }
                ]
            },
            {
                "role": "verifier",
                "objective": "verify_contract",
                "required": true,
                "budget": {
                    "timeout_ms": 3200
                },
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
    assert_eq!(observation["team_spec"]["spec_kind"], "agent_team_spec");
    assert_eq!(observation["team_spec"]["team_id"], "subagent-batch:5:2");
    assert_eq!(observation["team_spec"]["max_parallel"], 4);
    assert_eq!(observation["team_spec"]["write_permission"], "read_only");
    assert_eq!(
        observation["team_spec"]["conflict_policy"],
        "parent_loop_resolution_required"
    );
    assert_eq!(
        observation["team_spec"]["children"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        observation["team_lifecycle_events"][0]["event_type"],
        "agent_team_started"
    );
    assert!(observation["team_lifecycle_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "subagent_finished"));
    assert_eq!(
        observation["team_lifecycle_events"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["event_type"],
        "agent_team_aggregated"
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
    assert_eq!(observation["aggregation"]["finding_count"], 2);
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["reported_count"],
        1
    );
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["missing_count"],
        1
    );
    assert_eq!(observation["aggregation"]["conflict_count"], 0);
    assert_eq!(
        observation["aggregation"]["main_thread_decision"]["decision_status"],
        "ready_to_synthesize"
    );
    assert_eq!(
        observation["aggregation"]["recommended_next_action"],
        "synthesize_from_child_findings"
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["kind"],
        "file_ref"
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["confidence"],
        0.82
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
        observation["child_requests"][1]["timeout_policy"]["timeout_ms"],
        3200
    );
    assert_eq!(
        observation["child_requests"][1]["timeout_policy"]["terminal_status_on_timeout"],
        "timeout"
    );
    assert_eq!(
        observation["child_requests"][1]["cancellation_policy"]["cancel_scope"],
        "child_run"
    );
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_parallel_readonly_completed"
    );
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
}

#[test]
fn subagent_batch_records_conflicting_findings_for_parent_decision() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 6;
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "inspect_policy_a",
                "findings": [
                    {
                        "kind": "risk_review",
                        "status": "pass",
                        "code": "policy_state",
                        "conflict_group": "policy_state",
                        "confidence": 0.91,
                        "evidence_refs": ["step_1:evidence"]
                    }
                ]
            },
            {
                "role": "review",
                "objective": "inspect_policy_b",
                "findings": [
                    {
                        "kind": "risk_review",
                        "status": "fail",
                        "code": "policy_state",
                        "conflict_group": "policy_state",
                        "confidence": 0.73,
                        "evidence_refs": ["step_2:evidence"]
                    }
                ]
            }
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 11, 4, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["aggregation"]["status"], "completed");
    assert_eq!(observation["aggregation"]["conflict_count"], 1);
    assert_eq!(
        observation["aggregation"]["conflict_summary"]["conflict_groups"][0]["group_ref"],
        "policy_state"
    );
    assert_eq!(
        observation["aggregation"]["conflict_summary"]["conflict_groups"][0]["status_count"],
        2
    );
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["reported_count"],
        2
    );
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["min"],
        0.73
    );
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["max"],
        0.91
    );
    assert_eq!(
        observation["aggregation"]["main_thread_decision"]["decision_owner"],
        "parent_agent_loop"
    );
    assert_eq!(
        observation["aggregation"]["main_thread_decision"]["decision_required"],
        true
    );
    assert_eq!(
        observation["aggregation"]["main_thread_decision"]["decision_status"],
        "needs_conflict_resolution"
    );
    assert_eq!(
        observation["aggregation"]["recommended_next_action"],
        "resolve_child_conflicts"
    );
    assert!(observation["team_lifecycle_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "agent_team_conflict_detected"));
    assert_eq!(observation["child_run_summary"]["conflict_count"], 1);
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
        context_evidence_root: None,
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

#[test]
fn subagent_batch_expected_required_child_failure_dry_run_is_delivered() {
    let mut loop_state = LoopState::new(2);
    let args = serde_json::json!({
        "dry_run": true,
        "expected_failure": true,
        "children": [
            {
                "role": "explorer",
                "objective": "readonly_probe"
            },
            {
                "role": "unsupported_required_probe",
                "objective": "required_failure_probe",
                "required": true
            }
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 5, 1, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["status"], "accepted");
    assert_eq!(observation["result_status"], "completed_expected_failure");
    assert_eq!(
        observation["outcome_code"],
        "subagent_expected_required_child_failure_observed"
    );
    assert_eq!(observation["dry_run"], true);
    assert_eq!(observation["expected_failure"], true);
    assert_eq!(observation["expected_failure_delivery"], true);
    assert_eq!(observation["actual_required_child_failed"], true);
    assert_eq!(observation["actual_failure_isolated"], false);
    assert_eq!(observation["failure_isolated"], true);
    assert_eq!(
        observation["aggregation"]["status"],
        "failed_required_child"
    );
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_required_child_failed"
    );
    assert_eq!(
        observation["scheduler"]["status"],
        "expected_required_child_failure_observed"
    );
    assert_eq!(
        observation["merge_contract"]["parent_result_status"],
        "completed_expected_failure"
    );
}
