use serde_json::{json, Value};

use super::*;

#[test]
fn trace_json_includes_pollable_machine_event_stream() {
    let mut journal = TaskJournal::for_task("task-events", "ask", "inspect");
    journal.record_task_lifecycle(json!({
        "state": "background",
        "next_action_kind": "poll_async_job",
        "next_action_ref": "job-1"
    }));
    journal.rounds.push(TaskJournalRoundTrace {
        round_no: 1,
        goal: "inspect".to_string(),
        plan_result: Some(test_plan(
            crate::PlanKind::Single,
            vec![test_plan_step(
                "step_1",
                "call_capability",
                "filesystem.list_entries",
                json!({"path": "."}),
            )],
        )),
        ..Default::default()
    });
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        r#"{"status":"ok","output_path":"reports/out.txt"}"#,
    ));
    journal.push_task_observation(json!({"source": "fs_basic", "status": "ok"}));
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);

    let trace = journal.to_trace_json();
    let events = trace
        .get("event_stream")
        .and_then(Value::as_array)
        .expect("event_stream");
    let event_types = events
        .iter()
        .filter_map(|event| event.get("event_type").and_then(Value::as_str))
        .collect::<Vec<_>>();

    assert_eq!(
        event_types,
        vec![
            "task_lifecycle",
            "agent_round",
            "tool_started",
            "tool_step",
            "tool_finished",
            "task_observation",
            "task_final"
        ]
    );
    assert_eq!(events[0].get("seq").and_then(Value::as_u64), Some(1));
    assert_eq!(
        events[2].pointer("/payload/phase").and_then(Value::as_str),
        Some("started")
    );
    assert_eq!(
        events[2]
            .pointer("/payload/evidence_ref")
            .and_then(Value::as_str),
        Some("step_1")
    );
    assert_eq!(
        events[3].pointer("/payload/status").and_then(Value::as_str),
        Some("ok")
    );
    assert_eq!(
        events[3]
            .pointer("/payload/action_kind")
            .and_then(Value::as_str),
        Some("call_capability")
    );
    assert_eq!(
        events[3]
            .pointer("/payload/requested_capability")
            .and_then(Value::as_str),
        Some("filesystem.list_entries")
    );
    assert_eq!(
        events[3]
            .pointer("/payload/resolved_tool_or_skill")
            .and_then(Value::as_str),
        Some("fs_basic")
    );
    assert_eq!(
        events[3]
            .pointer("/payload/resolution_source")
            .and_then(Value::as_str),
        Some("capability_resolver")
    );
    assert_eq!(
        events[3]
            .pointer("/payload/artifact_ref_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        events[3]
            .pointer("/payload/artifact_refs/0/ref")
            .and_then(Value::as_str),
        Some("reports/out.txt")
    );
    assert_eq!(
        events[4].pointer("/payload/phase").and_then(Value::as_str),
        Some("finished")
    );
    assert_eq!(
        events[4].pointer("/payload/status").and_then(Value::as_str),
        Some("ok")
    );
}

#[test]
fn trace_json_projects_checkpoint_as_machine_event() {
    let mut journal = TaskJournal::for_task("task-checkpoint-event", "ask", "long task");
    journal.record_task_checkpoint(json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-event",
        "resume_entrypoint": "poll_async_job",
        "completed_side_effect_refs": ["write_file:tmp/report.txt"],
        "pending_async_job": {
            "job_id": "job-event",
            "poll_ref": "local_process:123",
            "cancel_ref": "local_process:123",
            "message_key": "async_job_running"
        }
    }));

    let trace = journal.to_trace_json();
    let events = trace
        .get("event_stream")
        .and_then(Value::as_array)
        .expect("event_stream");
    let event = events
        .iter()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("checkpoint_created"))
        .expect("checkpoint_created event");

    assert_eq!(
        event
            .pointer("/payload/checkpoint_id")
            .and_then(Value::as_str),
        Some("ckpt-event")
    );
    assert_eq!(
        event
            .pointer("/payload/checkpoint_ref")
            .and_then(Value::as_str),
        Some("task_checkpoint:ckpt-event")
    );
    assert_eq!(
        event
            .pointer("/payload/evidence_ref")
            .and_then(Value::as_str),
        Some("task_checkpoint:ckpt-event")
    );
    assert_eq!(
        event
            .pointer("/payload/completed_side_effect_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        event
            .pointer("/payload/requires_idempotency_guard")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        event
            .pointer("/payload/pending_async_job_id")
            .and_then(Value::as_str),
        Some("job-event")
    );
}

#[test]
fn trace_json_projects_ask_transitions_as_task_events() {
    let mut journal = TaskJournal::for_task("task-transition", "ask", "run");
    journal.transitions.push(crate::AskTransition::new(
        Some(crate::AskState::Received),
        crate::AskState::Routing,
        "received_to_routing",
        1781800000000,
        None,
    ));
    journal.transitions.push(crate::AskTransition::new(
        Some(crate::AskState::Executing),
        crate::AskState::Executing,
        "next_agent_round",
        1781800001000,
        Some(2),
    ));

    let trace = journal.to_trace_json();
    let events = trace
        .get("event_stream")
        .and_then(Value::as_array)
        .expect("event_stream");
    let transitions = events
        .iter()
        .filter(|event| event.get("event_type").and_then(Value::as_str) == Some("task_transition"))
        .collect::<Vec<_>>();

    assert_eq!(transitions.len(), 2);
    assert_eq!(
        transitions[0]
            .pointer("/payload/task_id")
            .and_then(Value::as_str),
        Some("task-transition")
    );
    assert_eq!(
        transitions[0]
            .pointer("/payload/state_from")
            .and_then(Value::as_str),
        Some("received")
    );
    assert_eq!(
        transitions[0]
            .pointer("/payload/state_to")
            .and_then(Value::as_str),
        Some("routing")
    );
    assert_eq!(
        transitions[0]
            .pointer("/payload/reason_code")
            .and_then(Value::as_str),
        Some("received_to_routing")
    );
    assert_eq!(
        transitions[0]
            .pointer("/payload/transition_index")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        transitions[0]
            .pointer("/payload/transition_ref")
            .and_then(Value::as_str),
        Some("task_transition:1")
    );
    assert_eq!(
        transitions[0]
            .pointer("/payload/evidence_ref")
            .and_then(Value::as_str),
        Some("task_transition:1")
    );
    assert_eq!(
        transitions[0]
            .pointer("/payload/evidence_refs/0")
            .and_then(Value::as_str),
        Some("task_transition:1")
    );
    assert_eq!(
        transitions[1]
            .pointer("/payload/round_no")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        transitions[1]
            .pointer("/payload/evidence_ref")
            .and_then(Value::as_str),
        Some("task_transition:2")
    );
}

#[test]
fn trace_json_projects_provider_prompt_metrics_as_provider_events() {
    let mut journal = TaskJournal::for_task("task-provider-events", "ask", "inspect");
    let mut by_prompt = std::collections::HashMap::new();
    by_prompt.insert(
        "normalizer".to_string(),
        crate::LlmPromptBucket {
            count: 1,
            elapsed_ms: 42,
            provider_attempt_count: 3,
            provider_retry_count: 2,
            provider_retryable_error_count: 2,
            provider_final_error_count: 1,
            provider_last_retry_error_kinds: std::collections::BTreeMap::from([(
                "timeout".to_string(),
                1,
            )]),
            provider_final_error_kinds: std::collections::BTreeMap::from([(
                "rate_limited".to_string(),
                1,
            )]),
            prompt_truncation_count: 1,
            prompt_bytes_before_max: Some(157_037),
            prompt_bytes_budget_min: Some(125_200),
            prompt_bytes_after_max: Some(125_180),
            prompt_truncated_bytes_total: 31_857,
        },
    );
    journal.record_llm_by_prompt(by_prompt);

    let trace = journal.to_trace_json();
    let events = trace
        .get("event_stream")
        .and_then(Value::as_array)
        .expect("event_stream");
    let event = events
        .iter()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("provider_call"))
        .expect("provider_call event");

    assert_eq!(
        event
            .pointer("/payload/prompt_label")
            .and_then(Value::as_str),
        Some("normalizer")
    );
    assert_eq!(
        event
            .pointer("/payload/provider_attempt_count")
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        event
            .pointer("/payload/provider_final_error_kinds/rate_limited")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        event
            .pointer("/payload/prompt_bytes_after_max")
            .and_then(Value::as_u64),
        Some(125_180)
    );
}

#[test]
fn trace_json_projects_http_download_artifact_ref_to_tool_event() {
    let mut journal = TaskJournal::for_task("task-http-artifact", "ask", "download");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "get",
                    "downloaded": true,
                    "output_path": "document/http/download/api.body",
                    "artifact_path": "document/http/download/api.body",
                    "size_bytes": 128
                },
                "text": "status=200\noutput_path=document/http/download/api.body"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    assert_eq!(
        trace
            .pointer("/step_results/0/artifact_ref_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        trace
            .pointer("/step_results/0/artifact_refs/0/ref")
            .and_then(Value::as_str),
        Some("document/http/download/api.body")
    );
    let event = trace
        .pointer("/event_stream")
        .and_then(Value::as_array)
        .and_then(|events| {
            events
                .iter()
                .find(|event| event.get("event_type").and_then(Value::as_str) == Some("tool_step"))
        })
        .expect("tool_step event");
    assert_eq!(
        event
            .pointer("/payload/artifact_refs/0/ref")
            .and_then(Value::as_str),
        Some("document/http/download/api.body")
    );
}

#[test]
fn trace_json_projects_coding_evidence_as_machine_event() {
    let mut journal = TaskJournal::for_task("task-coding-evidence", "ask", "patch");
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "run_cmd",
        json!({
            "extra": {
                "files_read": ["src/main.rs"],
                "changed_files": ["src/lib.rs"],
                "final_diff_summary": {
                    "summary_code": "patched"
                }
            },
            "text": "exit=0 command=cargo fmt --all"
        })
        .to_string(),
    ));
    journal.step_results.push(TaskJournalStepTrace::new(
        "step_2",
        "run_cmd",
        crate::executor::StepExecutionStatus::Error,
        Some("exit=101 command=cargo test -p clawd".to_string()),
        Some(
            r#"__RC_SKILL_ERROR__:{"skill":"run_cmd","error_kind":"exit_status","error_text":"failed","text":null}"#
                .to_string(),
        ),
    ));
    journal.push_task_observation(json!({
        "owner_layer": "coding_loop",
        "retry_count": 1
    }));

    let trace = journal.to_trace_json();
    let coding_checkpoints = trace
        .pointer("/event_stream")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter(|event| {
                    event.get("event_type").and_then(Value::as_str) == Some("coding_checkpoint")
                })
                .collect::<Vec<_>>()
        })
        .expect("coding checkpoint events");
    assert_eq!(coding_checkpoints.len(), 3);
    assert_eq!(
        coding_checkpoints[0]
            .pointer("/payload/checkpoint_kind")
            .and_then(Value::as_str),
        Some("file_edit_group")
    );
    assert_eq!(
        coding_checkpoints[0]
            .pointer("/payload/changed_files/0")
            .and_then(Value::as_str),
        Some("src/lib.rs")
    );
    assert_eq!(
        coding_checkpoints[1]
            .pointer("/payload/checkpoint_kind")
            .and_then(Value::as_str),
        Some("verification_command")
    );
    assert_eq!(
        coding_checkpoints[1]
            .pointer("/payload/verification_command")
            .and_then(Value::as_str),
        Some("cargo fmt --all")
    );
    assert_eq!(
        coding_checkpoints[2]
            .pointer("/payload/verification_command")
            .and_then(Value::as_str),
        Some("cargo test -p clawd")
    );
    assert_eq!(
        coding_checkpoints[2]
            .pointer("/payload/verification_failure_kinds/0")
            .and_then(Value::as_str),
        Some("test")
    );

    let contract_event = trace
        .pointer("/event_stream")
        .and_then(Value::as_array)
        .and_then(|events| {
            events.iter().find(|event| {
                event.get("event_type").and_then(Value::as_str) == Some("coding_task_contract")
            })
        })
        .expect("coding_task_contract event");
    assert_eq!(
        contract_event
            .pointer("/payload/contract_ref")
            .and_then(Value::as_str),
        Some("coding_task_contract:summary")
    );
    assert_eq!(
        contract_event
            .pointer("/payload/files_read/0")
            .and_then(Value::as_str),
        Some("src/main.rs")
    );
    assert_eq!(
        contract_event
            .pointer("/payload/files_changed/0")
            .and_then(Value::as_str),
        Some("src/lib.rs")
    );
    assert_eq!(
        contract_event
            .pointer("/payload/commands_run_count")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        contract_event
            .pointer("/payload/tests_run/0")
            .and_then(Value::as_str),
        Some("cargo test -p clawd")
    );
    assert_eq!(
        contract_event
            .pointer("/payload/final_diff_summary/value/summary_code")
            .and_then(Value::as_str),
        Some("patched")
    );

    let event = trace
        .pointer("/event_stream")
        .and_then(Value::as_array)
        .and_then(|events| {
            events.iter().find(|event| {
                event.get("event_type").and_then(Value::as_str) == Some("coding_evidence")
            })
        })
        .expect("coding_evidence event");

    assert_eq!(
        event
            .pointer("/payload/evidence_ref")
            .and_then(Value::as_str),
        Some("coding_evidence:summary")
    );
    assert_eq!(
        event
            .pointer("/payload/files_read/0")
            .and_then(Value::as_str),
        Some("src/main.rs")
    );
    assert_eq!(
        event
            .pointer("/payload/evidence_refs/0")
            .and_then(Value::as_str),
        Some("step_1")
    );
    assert_eq!(
        event
            .pointer("/payload/evidence_refs/1")
            .and_then(Value::as_str),
        Some("step_2")
    );
    assert_eq!(
        event
            .pointer("/payload/changed_files/0")
            .and_then(Value::as_str),
        Some("src/lib.rs")
    );
    assert_eq!(
        event.pointer("/payload/commands/0").and_then(Value::as_str),
        Some("cargo fmt --all")
    );
    assert_eq!(
        event.pointer("/payload/tests/0").and_then(Value::as_str),
        Some("cargo test -p clawd")
    );
    assert_eq!(
        event
            .pointer("/payload/verification_command_count")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        event
            .pointer("/payload/verification_commands/0")
            .and_then(Value::as_str),
        Some("cargo fmt --all")
    );
    assert_eq!(
        event
            .pointer("/payload/verification_commands/1")
            .and_then(Value::as_str),
        Some("cargo test -p clawd")
    );
    assert_eq!(
        event
            .pointer("/payload/failures/0/step_id")
            .and_then(Value::as_str),
        Some("step_2")
    );
    assert_eq!(
        event
            .pointer("/payload/failures/0/error_code")
            .and_then(Value::as_str),
        Some("exit_status")
    );
    assert_eq!(
        event
            .pointer("/payload/verification_status")
            .and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        event
            .pointer("/payload/verification_failure_kinds/0")
            .and_then(Value::as_str),
        Some("test")
    );
    assert_eq!(
        event
            .pointer("/payload/diff_summaries/0/value/summary_code")
            .and_then(Value::as_str),
        Some("patched")
    );
    assert_eq!(
        event
            .pointer("/payload/retry_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert!(event
        .pointer("/payload/unverified_risk")
        .is_some_and(Value::is_null));
}

#[test]
fn trace_json_projects_agent_hook_observations_as_hook_events() {
    let mut journal = TaskJournal::for_task("task-hook-events", "ask", "inspect");
    journal.push_task_observation(json!({
        "schema_version": 1,
        "owner_layer": "agent_hooks",
        "stage": "pre_tool_use",
        "decision": "allow",
        "reason_code": "pre_tool_use_allowed",
        "action_ref": "fs_basic.list_dir",
        "tool_or_skill": "fs_basic"
    }));

    let trace = journal.to_trace_json();
    let events = trace
        .get("event_stream")
        .and_then(Value::as_array)
        .expect("event_stream");

    assert_eq!(
        events[0].get("event_type").and_then(Value::as_str),
        Some("agent_hook")
    );
    assert_eq!(
        events[0]
            .pointer("/payload/decision")
            .and_then(Value::as_str),
        Some("allow")
    );
    assert_eq!(
        events[0]
            .pointer("/payload/action_ref")
            .and_then(Value::as_str),
        Some("fs_basic.list_dir")
    );
}

#[test]
fn trace_json_projects_tool_step_error_machine_fields() {
    let mut journal = TaskJournal::for_task("task-error-events", "ask", "inspect");
    journal.step_results.push(TaskJournalStepTrace::new(
        "step_1",
        "archive_basic",
        crate::executor::StepExecutionStatus::Error,
        None,
        Some(
            r#"__RC_SKILL_ERROR__:{"skill":"archive_basic","error_kind":"contract_action_rejected","error_text":"blocked","text":null}"#
                .to_string(),
        ),
    ));

    let trace = journal.to_trace_json();
    let events = trace
        .get("event_stream")
        .and_then(Value::as_array)
        .expect("event_stream");
    let event = events
        .iter()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("tool_step"))
        .expect("tool_step event");

    assert_eq!(
        event.pointer("/payload/error_kind").and_then(Value::as_str),
        Some("contract_action_rejected")
    );
    assert_eq!(
        event
            .pointer("/payload/failure_attribution")
            .and_then(Value::as_str),
        Some("contract_gap")
    );
}

#[test]
fn trace_json_projects_subagent_observations_as_subagent_events() {
    let mut journal = TaskJournal::for_task("task-subagent", "ask", "subagent");
    journal.push_task_observation(json!({
        "schema_version": 1,
        "owner_layer": "subagent_runtime",
        "status": "accepted",
        "role": "review",
        "child_run_summary": {
            "status": "completed",
            "result_status": "completed",
            "trace_merge_status": "merged"
        },
        "child_result": {
            "status": "completed",
            "outcome_code": "subagent_inline_readonly_completed"
        },
        "write_enabled": false,
        "external_publish_enabled": false,
    }));

    let trace = journal.to_trace_json();
    let events = trace
        .get("event_stream")
        .and_then(Value::as_array)
        .expect("event_stream");
    let event = events
        .iter()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("subagent"))
        .expect("subagent event");

    assert_eq!(
        event
            .pointer("/payload/owner_layer")
            .and_then(Value::as_str),
        Some("subagent_runtime")
    );
    assert_eq!(
        event.pointer("/payload/role").and_then(Value::as_str),
        Some("review")
    );
    assert_eq!(
        event
            .pointer("/payload/write_enabled")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        event
            .pointer("/payload/external_publish_enabled")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        event
            .pointer("/payload/child_result/outcome_code")
            .and_then(Value::as_str),
        Some("subagent_inline_readonly_completed")
    );
}
