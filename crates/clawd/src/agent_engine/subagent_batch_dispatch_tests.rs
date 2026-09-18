use super::*;

#[test]
fn subagent_batch_records_bounded_parallel_aggregation() {
    let mut loop_state = LoopState::new();
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
                    "runtime_deadline_ms": 3200
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
    assert_eq!(
        observation["aggregation"]["execution_mode"],
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
        observation["child_requests"][1]["timeout_policy"]["runtime_deadline_ms"],
        3200
    );
    assert_eq!(
        observation["child_requests"][1]["timeout_policy"]["terminal_status_on_deadline"],
        "timed_out"
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
    let mut loop_state = LoopState::new();
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
    let mut loop_state = LoopState::new();
    let config = SubagentRuntimeConfig {
        role_definitions: crate::agent_runtime_contract::default_subagent_role_definitions(),
        enabled: true,
        max_concurrent_threads_per_session: 1,
        join_wait_ms: 30_000,
        max_spawn_depth: 2,
        interrupt_message: true,
        legacy_config_key_used: true,
        max_running_threads_global: Some(1),
        max_parallel_readonly: 1,
        default_timeout_ms: Some(10_000),
        context_evidence_root: None,
        resolved_model_policies: std::collections::BTreeMap::new(),
    };
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "scheduled_optional_child"
            },
            {
                "role": "unsupported_writer_probe",
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
    let mut loop_state = LoopState::new();
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "optional_success"
            },
            {
                "role": "unsupported_writer_probe",
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
    let mut loop_state = LoopState::new();
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

#[test]
fn persistent_child_specs_keep_twenty_nodes_and_allocate_each_budget_slice() {
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-persistent-twenty-nodes".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("ui-user".to_string()),
        external_chat_id: Some("ui-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text": "parent task"}).to_string(),
    };
    let children = (0..20)
        .map(|index| {
            serde_json::json!({
                "node_id": format!("node_{index}"),
                "role": "explorer",
                "objective": format!("machine_child_objective:{index}"),
                "context_refs": ["AGENTS.md"],
                "allowed_capabilities": ["filesystem.read_text_range"]
            })
        })
        .collect::<Vec<_>>();
    let args = serde_json::json!({
        "action": "persistent_child_task",
        "children": children
    });
    let mut specs = super::subagent_runtime_persistent::persistent_child_specs(
        &task,
        &args,
        &SubagentRuntimeConfig::default(),
    )
    .expect("materialize every child spec");
    assert_eq!(specs.len(), 20);

    let mut loop_state = LoopState::new();
    install_test_task_budget(&mut loop_state);
    let allocations = super::subagent_runtime_persistent::allocate_persistent_child_budgets(
        &mut loop_state,
        &mut specs,
    )
    .expect("allocate every child budget");

    assert_eq!(allocations.len(), 20);
    assert_eq!(
        loop_state
            .task_budget_slice
            .as_ref()
            .expect("budget slice")
            .allocations
            .len(),
        20
    );
    assert!(specs.iter().all(|spec| spec
        .scope
        .get("budget_allocation_id")
        .and_then(serde_json::Value::as_str)
        .is_some()));
}

#[test]
fn persistent_subagent_registry_action_selects_persistent_runtime() {
    assert!(
        super::subagent_runtime_persistent::persistent_child_task_requested(
            &serde_json::json!({"action": "persistent_child_task"})
        )
    );
    assert!(
        !super::subagent_runtime_persistent::persistent_child_task_requested(
            &serde_json::json!({"execution_mode": "persistent_child_task"})
        )
    );
}

#[test]
fn explicit_inline_registry_action_does_not_fall_into_batch_dispatch() {
    let mut loop_state = LoopState::new();
    let args = serde_json::json!({
        "action": "inline_readonly",
        "role": "review",
        "objective": "inspect_runtime_boundary",
        "children": [
            {"role": "test", "objective": "must_not_replace_single_child"}
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 1, 1, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["execution_mode"], "inline_readonly_child_run");
    assert_eq!(observation["role"], "review");
    assert_eq!(observation["objective_present"], true);
    assert!(observation.get("aggregation").is_none());
}

#[test]
fn explicit_batch_registry_action_uses_bounded_batch_dispatch() {
    let mut loop_state = LoopState::new();
    let args = serde_json::json!({
        "action": "bounded_parallel_readonly",
        "children": [
            {"role": "review", "objective": "inspect_runtime_boundary"},
            {"role": "test", "objective": "inspect_test_boundary"}
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 1, 1, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(
        observation["execution_mode"],
        "bounded_parallel_readonly_child_runs"
    );
    assert_eq!(observation["aggregation"]["child_count"], 2);
}
