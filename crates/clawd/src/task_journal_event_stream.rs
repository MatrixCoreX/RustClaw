use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::{
    capability_resolution_source, next_requested_capability, requested_capability_sequence,
    step_action_kind, TaskJournal, TaskJournalFinalStatus,
};

fn task_event_json(seq: &mut u64, event_type: &str, payload: Value) -> Value {
    *seq += 1;
    json!({
        "seq": *seq,
        "event_type": event_type,
        "owner_layer": "task_journal",
        "payload": payload,
    })
}

pub(super) fn task_event_stream_json(journal: &TaskJournal) -> Vec<Value> {
    let mut seq = 0;
    let mut events = Vec::new();
    let mut coding_evidence = CodingEvidenceSignals::default();
    if let Some(lifecycle) = journal.task_lifecycle.as_ref() {
        events.push(task_event_json(
            &mut seq,
            "task_lifecycle",
            lifecycle.clone(),
        ));
    }
    events.push(task_event_json(
        &mut seq,
        "task_goal",
        super::task_journal_goal::task_goal_summary_json(journal),
    ));
    append_context_budget_events(&mut seq, &mut events, journal);
    if let Some(checkpoint) = journal.task_checkpoint.as_ref() {
        events.push(task_event_json(
            &mut seq,
            "checkpoint_created",
            checkpoint_event_payload(checkpoint),
        ));
    }
    for (index, transition) in journal.transitions.iter().enumerate() {
        let transition_ref = format!("task_transition:{}", index + 1);
        events.push(task_event_json(
            &mut seq,
            "task_transition",
            json!({
                "transition_index": index,
                "transition_ref": transition_ref.as_str(),
                "evidence_ref": transition_ref.as_str(),
                "evidence_refs": [transition_ref.as_str()],
                "task_id": journal.task_id.as_deref(),
                "state_from": transition.from.map(crate::AskState::as_str),
                "state_to": transition.to.as_str(),
                "reason_code": crate::truncate_for_log(&transition.reason),
                "at_ms": transition.at_ms,
                "round_no": transition.round_no,
            }),
        ));
    }
    for round in &journal.rounds {
        events.push(task_event_json(
            &mut seq,
            "agent_round",
            json!({
                "round_no": round.round_no,
                "has_plan": round.plan_result.is_some(),
                "has_verify": round.verify_result.is_some(),
                "execution_recipe_summary_present": round.execution_recipe_summary.is_some(),
            }),
        ));
        append_planner_and_verifier_events(&mut seq, &mut events, round);
    }
    append_provider_call_events(&mut seq, &mut events, journal);
    let mut requested = requested_capability_sequence(journal);
    for (index, step) in journal.step_results.iter().enumerate() {
        let requested = next_requested_capability(&mut requested, step);
        let action_kind = step_action_kind(step, requested.as_ref());
        let step_trace = super::step_trace_json(step, requested.as_ref(), None);
        collect_coding_evidence_from_step(step, &step_trace, &mut coding_evidence);
        events.push(task_event_json(
            &mut seq,
            "tool_started",
            tool_lifecycle_event_payload(
                index,
                step,
                requested.as_ref(),
                &action_kind,
                &step_trace,
                "started",
            ),
        ));
        events.push(task_event_json(
            &mut seq,
            "tool_step",
            json!({
                "index": index,
                "step_id": step.step_id,
                "action_kind": action_kind,
                "skill": step.skill,
                "requested_action_type": requested.as_ref().map(|value| value.action_type.as_str()),
                "requested_capability": requested.as_ref().map(|value| value.capability.as_str()),
                "requested_action_ref": requested
                    .as_ref()
                    .and_then(|value| value.action_ref.as_deref()),
                "executed_skill": step.skill,
                "resolved_tool_or_skill": step.skill,
                "resolved_capability": requested
                    .as_ref()
                    .filter(|value| value.action_type == "call_capability")
                    .map(|value| value.capability.as_str()),
                "resolution_source": requested
                    .as_ref()
                    .map(|value| capability_resolution_source(&value.action_type))
                    .unwrap_or("step_trace_fallback"),
                "status": step.status.as_str(),
                "error_kind": step_trace.get("error_kind"),
                "failure_attribution": step_trace.get("failure_attribution"),
                "output_evidence_count": step_trace.get("output_evidence_count"),
                "artifact_ref_count": step_trace.get("artifact_ref_count"),
                "artifact_refs": step_trace.get("artifact_refs").cloned().unwrap_or_else(|| json!([])),
                "structured_workspace_mutation": step_trace.get("structured_workspace_mutation"),
                "patch_id": step_trace.pointer("/structured_workspace_mutation/patch_id"),
                "mutation_id": step_trace.pointer("/structured_workspace_mutation/mutation_id"),
                "checkpoint_id": step_trace.pointer("/structured_workspace_mutation/checkpoint_id"),
                "compensates_checkpoint_id": step_trace.pointer("/structured_workspace_mutation/compensates_checkpoint_id"),
                "compensates_patch_id": step_trace.pointer("/structured_workspace_mutation/compensates_patch_id"),
                "compensates_mutation_id": step_trace.pointer("/structured_workspace_mutation/compensates_mutation_id"),
                "isolation_root": step_trace.pointer("/structured_workspace_mutation/isolation_root"),
                "reversible": step_trace.pointer("/structured_workspace_mutation/reversible")
                    .or_else(|| step_trace.pointer("/mutation_reversibility/reversible")),
                "reversibility_status": step_trace.pointer("/mutation_reversibility/status"),
                "reversibility_reason_code": step_trace.pointer("/mutation_reversibility/reason_code"),
                "additions": step_trace.pointer("/structured_workspace_mutation/additions"),
                "deletions": step_trace.pointer("/structured_workspace_mutation/deletions"),
                "changed_hunks": step_trace.pointer("/structured_workspace_mutation/changed_hunks"),
                "started_at": step.started_at,
                "finished_at": step.finished_at,
            }),
        ));
        events.push(task_event_json(
            &mut seq,
            "tool_finished",
            tool_lifecycle_event_payload(
                index,
                step,
                requested.as_ref(),
                &action_kind,
                &step_trace,
                "finished",
            ),
        ));
    }
    for (index, observation) in journal.task_observations.iter().enumerate() {
        collect_coding_evidence_value(observation, &mut coding_evidence, None, 0);
        if observation
            .get("owner_layer")
            .and_then(Value::as_str)
            .map(str::trim)
            == Some("agent_hooks")
        {
            let mut payload = observation.clone();
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("index".to_string(), json!(index));
            }
            events.push(task_event_json(&mut seq, "agent_hook", payload));
        } else if observation
            .get("owner_layer")
            .and_then(Value::as_str)
            .map(str::trim)
            == Some("mcp_runtime")
        {
            let mut payload = observation.clone();
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("index".to_string(), json!(index));
            }
            events.push(task_event_json(&mut seq, "mcp_tool_call", payload));
        } else if observation
            .get("owner_layer")
            .and_then(Value::as_str)
            .map(str::trim)
            == Some("subagent_runtime")
        {
            let mut payload = observation.clone();
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("index".to_string(), json!(index));
            }
            events.push(task_event_json(&mut seq, "subagent", payload.clone()));
            append_subagent_team_lifecycle_events(&mut seq, &mut events, &payload);
        } else {
            events.push(task_event_json(
                &mut seq,
                "task_observation",
                json!({
                    "index": index,
                    "observation": observation,
                }),
            ));
        }
    }
    if coding_evidence.has_signals() {
        append_coding_progress_events(&mut seq, &mut events, &coding_evidence);
        append_coding_checkpoint_events(&mut seq, &mut events, &coding_evidence);
        events.push(task_event_json(
            &mut seq,
            "coding_task_contract",
            coding_evidence.to_contract_payload(),
        ));
        events.push(task_event_json(
            &mut seq,
            "coding_evidence",
            coding_evidence.to_payload(),
        ));
    }
    if journal.final_status.is_some() || journal.final_stop_signal.is_some() {
        events.push(task_event_json(
            &mut seq,
            "task_final",
            json!({
                "final_status": journal.final_status.map(TaskJournalFinalStatus::as_str),
                "final_stop_signal": journal.final_stop_signal.as_deref(),
                "final_failure_attribution": journal.final_failure_attribution.as_deref(),
            }),
        ));
    }
    events
}

fn append_planner_and_verifier_events(
    seq: &mut u64,
    events: &mut Vec<Value>,
    round: &super::TaskJournalRoundTrace,
) {
    if let Some(plan) = round.plan_result.as_ref() {
        events.push(task_event_json(
            seq,
            "planner_finished",
            json!({
                "round_no": round.round_no,
                "plan_kind": plan.plan_kind.as_str(),
                "step_count": plan.steps.len(),
                "missing_slot_count": plan.missing_slots.len(),
                "needs_confirmation": plan.needs_confirmation,
                "actions": plan.steps.iter().map(|step| json!({
                    "step_id": step.step_id,
                    "action_type": step.action_type,
                    "capability_or_tool": step.skill,
                    "dependency_count": step.depends_on.len(),
                })).collect::<Vec<_>>(),
            }),
        ));
    }
    let Some(verify) = round.verify_result.as_ref() else {
        return;
    };
    events.push(task_event_json(
        seq,
        "plan_verification",
        json!({
            "round_no": round.round_no,
            "mode": verify.mode.as_str(),
            "approved": verify.approved,
            "needs_confirmation": verify.needs_confirmation,
            "issue_count": verify.issues.len(),
            "issues": verify.issues.iter().map(|issue| json!({
                "step_id": issue.step_id,
                "reason_code": issue.kind.reason_code(),
                "status_code": issue.kind.status_code(),
                "message_key": issue.kind.message_key(),
                "missing_fields": issue.missing_fields,
            })).collect::<Vec<_>>(),
        }),
    ));
    events.push(task_event_json(
        seq,
        if verify.needs_confirmation {
            "permission_request"
        } else {
            "permission_decision"
        },
        json!({
            "round_no": round.round_no,
            "decision": verify.permission_decision,
        }),
    ));
}

fn append_context_budget_events(seq: &mut u64, events: &mut Vec<Value>, journal: &TaskJournal) {
    if let Some(report) = super::task_journal_context_budget::context_budget_report_json(
        journal.context_bundle_summary.as_deref(),
    ) {
        events.push(task_event_json(seq, "context_budget", report));
    }
    let Some(records) = super::task_journal_context_compaction::transcript_compaction_records_json(
        &journal.task_observations,
    ) else {
        return;
    };
    let record_count = records.as_array().map(Vec::len).unwrap_or(0);
    events.push(task_event_json(
        seq,
        "context_compaction",
        json!({
            "schema_version": 1,
            "record_count": record_count,
            "records": records,
        }),
    ));
}

fn append_subagent_team_lifecycle_events(
    seq: &mut u64,
    events: &mut Vec<Value>,
    observation: &Value,
) {
    let Some(items) = observation
        .get("team_lifecycle_events")
        .and_then(Value::as_array)
    else {
        return;
    };
    for item in items.iter().take(64) {
        let Some(event_type) = item
            .get("event_type")
            .and_then(Value::as_str)
            .and_then(subagent_team_event_type)
        else {
            continue;
        };
        events.push(task_event_json(seq, event_type, item.clone()));
    }
}

fn subagent_team_event_type(value: &str) -> Option<&'static str> {
    match value.trim() {
        "agent_team_started" => Some("agent_team_started"),
        "subagent_started" => Some("subagent_started"),
        "subagent_finished" => Some("subagent_finished"),
        "subagent_failed" => Some("subagent_failed"),
        "agent_team_aggregated" => Some("agent_team_aggregated"),
        "agent_team_conflict_detected" => Some("agent_team_conflict_detected"),
        _ => None,
    }
}

fn checkpoint_event_payload(checkpoint: &Value) -> Value {
    let checkpoint_id = checkpoint
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let checkpoint_ref = checkpoint_id.map(|value| format!("task_checkpoint:{value}"));
    let completed_side_effect_count = checkpoint
        .get("completed_side_effect_refs")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    json!({
        "checkpoint_id": checkpoint_id,
        "checkpoint_ref": checkpoint_ref.as_deref(),
        "evidence_ref": checkpoint_ref.as_deref(),
        "evidence_refs": checkpoint_ref.iter().map(String::as_str).collect::<Vec<_>>(),
        "resume_entrypoint": checkpoint.get("resume_entrypoint").and_then(Value::as_str),
        "completed_side_effect_count": completed_side_effect_count,
        "requires_idempotency_guard": completed_side_effect_count > 0,
        "pending_async_job_id": checkpoint.pointer("/pending_async_job/job_id").and_then(Value::as_str),
        "poll_ref": checkpoint.pointer("/pending_async_job/poll_ref").and_then(Value::as_str),
        "cancel_ref": checkpoint.pointer("/pending_async_job/cancel_ref").and_then(Value::as_str),
        "message_key": checkpoint.pointer("/pending_async_job/message_key").and_then(Value::as_str),
    })
}

fn tool_lifecycle_event_payload(
    index: usize,
    step: &super::TaskJournalStepTrace,
    requested: Option<&super::RequestedPlanCapability>,
    action_kind: &str,
    step_trace: &Value,
    phase: &'static str,
) -> Value {
    json!({
        "index": index,
        "phase": phase,
        "step_id": step.step_id,
        "step_ref": step.step_id,
        "evidence_ref": step.step_id,
        "evidence_refs": [step.step_id.as_str()],
        "action_kind": action_kind,
        "skill": step.skill,
        "requested_action_type": requested.map(|value| value.action_type.as_str()),
        "requested_capability": requested.map(|value| value.capability.as_str()),
        "requested_action_ref": requested.and_then(|value| value.action_ref.as_deref()),
        "resolved_tool_or_skill": step.skill,
        "resolved_capability": requested
            .filter(|value| value.action_type == "call_capability")
            .map(|value| value.capability.as_str()),
        "resolution_source": requested
            .map(|value| capability_resolution_source(&value.action_type))
            .unwrap_or("step_trace_fallback"),
        "status": step.status.as_str(),
        "error_kind": step_trace.get("error_kind"),
        "failure_attribution": step_trace.get("failure_attribution"),
        "output_evidence_count": step_trace.get("output_evidence_count"),
        "artifact_ref_count": step_trace.get("artifact_ref_count"),
        "structured_workspace_mutation": step_trace.get("structured_workspace_mutation"),
        "patch_id": step_trace.pointer("/structured_workspace_mutation/patch_id"),
        "mutation_id": step_trace.pointer("/structured_workspace_mutation/mutation_id"),
        "checkpoint_id": step_trace.pointer("/structured_workspace_mutation/checkpoint_id"),
        "compensates_checkpoint_id": step_trace.pointer("/structured_workspace_mutation/compensates_checkpoint_id"),
        "compensates_patch_id": step_trace.pointer("/structured_workspace_mutation/compensates_patch_id"),
        "compensates_mutation_id": step_trace.pointer("/structured_workspace_mutation/compensates_mutation_id"),
        "isolation_root": step_trace.pointer("/structured_workspace_mutation/isolation_root"),
        "reversible": step_trace.pointer("/structured_workspace_mutation/reversible")
            .or_else(|| step_trace.pointer("/mutation_reversibility/reversible")),
        "reversibility_status": step_trace.pointer("/mutation_reversibility/status"),
        "reversibility_reason_code": step_trace.pointer("/mutation_reversibility/reason_code"),
        "additions": step_trace.pointer("/structured_workspace_mutation/additions"),
        "deletions": step_trace.pointer("/structured_workspace_mutation/deletions"),
        "changed_hunks": step_trace.pointer("/structured_workspace_mutation/changed_hunks"),
        "started_at": step.started_at,
        "finished_at": step.finished_at,
    })
}

#[derive(Default)]
struct CodingEvidenceSignals {
    files_read: BTreeSet<String>,
    changed_files: BTreeSet<String>,
    commands: BTreeSet<String>,
    verification_commands: BTreeSet<String>,
    tests: BTreeSet<String>,
    evidence_refs: BTreeSet<String>,
    workspace_checkpoint_ids: BTreeSet<String>,
    patch_ids: BTreeSet<String>,
    mutation_ids: BTreeSet<String>,
    diff_summaries: Vec<Value>,
    failures: Vec<Value>,
    verification_failure_kinds: BTreeSet<String>,
    retry_count: u64,
}

impl CodingEvidenceSignals {
    fn has_signals(&self) -> bool {
        !self.files_read.is_empty()
            || !self.changed_files.is_empty()
            || !self.commands.is_empty()
            || !self.tests.is_empty()
            || !self.diff_summaries.is_empty()
            || !self.workspace_checkpoint_ids.is_empty()
            || !self.failures.is_empty()
            || self.retry_count > 0
    }

    fn to_payload(&self) -> Value {
        let unverified_risk = if !self.changed_files.is_empty() && self.tests.is_empty() {
            Value::String("tests_not_observed".to_string())
        } else {
            Value::Null
        };
        let verification_status = coding_verification_status(self);
        json!({
            "schema_version": 1,
            "evidence_ref": "coding_evidence:summary",
            "evidence_refs": self.evidence_refs.iter().cloned().collect::<Vec<_>>(),
            "files_read_count": self.files_read.len(),
            "files_read": self.files_read.iter().cloned().collect::<Vec<_>>(),
            "changed_file_count": self.changed_files.len(),
            "changed_files": self.changed_files.iter().cloned().collect::<Vec<_>>(),
            "command_count": self.commands.len(),
            "commands": self.commands.iter().cloned().collect::<Vec<_>>(),
            "verification_command_count": self.verification_commands.len(),
            "verification_commands": self.verification_commands.iter().cloned().collect::<Vec<_>>(),
            "test_count": self.tests.len(),
            "tests": self.tests.iter().cloned().collect::<Vec<_>>(),
            "diff_summary_count": self.diff_summaries.len(),
            "diff_summaries": self.diff_summaries.clone(),
            "workspace_checkpoint_ids": self.workspace_checkpoint_ids.iter().cloned().collect::<Vec<_>>(),
            "patch_ids": self.patch_ids.iter().cloned().collect::<Vec<_>>(),
            "mutation_ids": self.mutation_ids.iter().cloned().collect::<Vec<_>>(),
            "failure_count": self.failures.len(),
            "failures": self.failures.clone(),
            "verification_status": verification_status,
            "verification_failure_kind_count": self.verification_failure_kinds.len(),
            "verification_failure_kinds": self.verification_failure_kinds.iter().cloned().collect::<Vec<_>>(),
            "retry_count": self.retry_count,
            "unverified_risk": unverified_risk,
        })
    }

    fn to_contract_payload(&self) -> Value {
        let unverified_risk = if !self.changed_files.is_empty() && self.tests.is_empty() {
            Value::String("tests_not_observed".to_string())
        } else {
            Value::Null
        };
        let final_diff_summary = self.diff_summaries.last().cloned().unwrap_or(Value::Null);
        json!({
            "schema_version": 1,
            "contract_ref": "coding_task_contract:summary",
            "evidence_ref": "coding_task_contract:summary",
            "evidence_refs": self.evidence_refs.iter().cloned().collect::<Vec<_>>(),
            "files_read_count": self.files_read.len(),
            "files_read": self.files_read.iter().cloned().collect::<Vec<_>>(),
            "files_changed_count": self.changed_files.len(),
            "files_changed": self.changed_files.iter().cloned().collect::<Vec<_>>(),
            "changed_file_count": self.changed_files.len(),
            "changed_files": self.changed_files.iter().cloned().collect::<Vec<_>>(),
            "commands_run_count": self.commands.len(),
            "commands_run": self.commands.iter().cloned().collect::<Vec<_>>(),
            "command_count": self.commands.len(),
            "commands": self.commands.iter().cloned().collect::<Vec<_>>(),
            "verification_command_count": self.verification_commands.len(),
            "verification_commands": self.verification_commands.iter().cloned().collect::<Vec<_>>(),
            "tests_run_count": self.tests.len(),
            "tests_run": self.tests.iter().cloned().collect::<Vec<_>>(),
            "test_count": self.tests.len(),
            "tests": self.tests.iter().cloned().collect::<Vec<_>>(),
            "failure_count": self.failures.len(),
            "failures": self.failures.clone(),
            "retry_count": self.retry_count,
            "final_diff_summary": final_diff_summary,
            "diff_summary_count": self.diff_summaries.len(),
            "diff_summaries": self.diff_summaries.clone(),
            "workspace_checkpoint_ids": self.workspace_checkpoint_ids.iter().cloned().collect::<Vec<_>>(),
            "patch_ids": self.patch_ids.iter().cloned().collect::<Vec<_>>(),
            "mutation_ids": self.mutation_ids.iter().cloned().collect::<Vec<_>>(),
            "verification_status": coding_verification_status(self),
            "verification_failure_kind_count": self.verification_failure_kinds.len(),
            "verification_failure_kinds": self.verification_failure_kinds.iter().cloned().collect::<Vec<_>>(),
            "unverified_risk": unverified_risk,
        })
    }
}

fn coding_verification_status(signals: &CodingEvidenceSignals) -> &'static str {
    if !signals.failures.is_empty() {
        "failed"
    } else if !signals.verification_commands.is_empty() {
        "verified"
    } else if !signals.changed_files.is_empty() {
        "unverified"
    } else {
        "not_applicable"
    }
}

fn append_coding_progress_events(
    seq: &mut u64,
    events: &mut Vec<Value>,
    signals: &CodingEvidenceSignals,
) {
    let evidence_refs = signals.evidence_refs.iter().cloned().collect::<Vec<_>>();
    if !signals.diff_summaries.is_empty() {
        events.push(task_event_json(
            seq,
            "workspace_diff",
            json!({
                "schema_version": 1,
                "evidence_refs": evidence_refs,
                "changed_files": signals.changed_files.iter().cloned().collect::<Vec<_>>(),
                "diff_summary_count": signals.diff_summaries.len(),
                "diff_summaries": signals.diff_summaries,
                "workspace_checkpoint_ids": signals.workspace_checkpoint_ids.iter().cloned().collect::<Vec<_>>(),
                "patch_ids": signals.patch_ids.iter().cloned().collect::<Vec<_>>(),
                "mutation_ids": signals.mutation_ids.iter().cloned().collect::<Vec<_>>(),
            }),
        ));
    }
    if !signals.changed_files.is_empty()
        || !signals.verification_commands.is_empty()
        || !signals.failures.is_empty()
    {
        events.push(task_event_json(
            seq,
            "verification",
            json!({
                "schema_version": 1,
                "evidence_refs": evidence_refs,
                "status": coding_verification_status(signals),
                "verification_commands": signals.verification_commands.iter().cloned().collect::<Vec<_>>(),
                "failure_count": signals.failures.len(),
                "failure_kinds": signals.verification_failure_kinds.iter().cloned().collect::<Vec<_>>(),
                "workspace_checkpoint_ids": signals.workspace_checkpoint_ids.iter().cloned().collect::<Vec<_>>(),
                "patch_ids": signals.patch_ids.iter().cloned().collect::<Vec<_>>(),
                "mutation_ids": signals.mutation_ids.iter().cloned().collect::<Vec<_>>(),
            }),
        ));
    }
    if signals.retry_count > 0 {
        events.push(task_event_json(
            seq,
            "retry",
            json!({
                "schema_version": 1,
                "evidence_refs": evidence_refs,
                "retry_count": signals.retry_count,
            }),
        ));
    }
}

fn append_coding_checkpoint_events(
    seq: &mut u64,
    events: &mut Vec<Value>,
    signals: &CodingEvidenceSignals,
) {
    if !signals.changed_files.is_empty() {
        let checkpoint_ref = "coding_checkpoint:file_edit_group";
        events.push(task_event_json(
            seq,
            "coding_checkpoint",
            json!({
                "schema_version": 1,
                "checkpoint_kind": "file_edit_group",
                "checkpoint_ref": checkpoint_ref,
                "evidence_ref": checkpoint_ref,
                "evidence_refs": signals.evidence_refs.iter().cloned().collect::<Vec<_>>(),
                "changed_file_count": signals.changed_files.len(),
                "changed_files": signals.changed_files.iter().cloned().collect::<Vec<_>>(),
                "workspace_checkpoint_ids": signals.workspace_checkpoint_ids.iter().cloned().collect::<Vec<_>>(),
                "patch_ids": signals.patch_ids.iter().cloned().collect::<Vec<_>>(),
                "mutation_ids": signals.mutation_ids.iter().cloned().collect::<Vec<_>>(),
                "verification_status": coding_verification_status(signals),
            }),
        ));
    }
    if coding_verification_status(signals) == "verified"
        && !signals.workspace_checkpoint_ids.is_empty()
    {
        events.push(task_event_json(
            seq,
            "coding_checkpoint",
            json!({
                "schema_version": 1,
                "checkpoint_kind": "verified_workspace_checkpoint",
                "checkpoint_ref": "coding_checkpoint:verified_workspace_checkpoint",
                "checkpoint_id": signals.workspace_checkpoint_ids.iter().next(),
                "patch_id": signals.patch_ids.iter().next(),
                "mutation_id": signals.mutation_ids.iter().next(),
                "evidence_ref": "coding_checkpoint:verified_workspace_checkpoint",
                "evidence_refs": signals.evidence_refs.iter().cloned().collect::<Vec<_>>(),
                "workspace_checkpoint_ids": signals.workspace_checkpoint_ids.iter().cloned().collect::<Vec<_>>(),
                "patch_ids": signals.patch_ids.iter().cloned().collect::<Vec<_>>(),
                "mutation_ids": signals.mutation_ids.iter().cloned().collect::<Vec<_>>(),
                "verification_status": "verified",
            }),
        ));
    }
    for (index, command) in signals.verification_commands.iter().take(32).enumerate() {
        let checkpoint_ref = format!("coding_checkpoint:verification_command:{}", index + 1);
        events.push(task_event_json(
            seq,
            "coding_checkpoint",
            json!({
                "schema_version": 1,
                "checkpoint_kind": "verification_command",
                "checkpoint_ref": checkpoint_ref,
                "evidence_ref": checkpoint_ref,
                "evidence_refs": signals.evidence_refs.iter().cloned().collect::<Vec<_>>(),
                "command_index": index + 1,
                "verification_command": command,
                "verification_command_count": signals.verification_commands.len(),
                "verification_status": coding_verification_status(signals),
                "verification_failure_kind_count": signals.verification_failure_kinds.len(),
                "verification_failure_kinds": signals
                    .verification_failure_kinds
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            }),
        ));
    }
}

fn collect_coding_evidence_from_step(
    step: &super::TaskJournalStepTrace,
    step_trace: &Value,
    signals: &mut CodingEvidenceSignals,
) {
    let step_ref = step.step_id.trim();
    let step_ref = (!step_ref.is_empty()).then_some(step_ref);
    collect_coding_evidence_value(step_trace, signals, step_ref, 0);
    if let Some(output) = step.output_excerpt.as_deref().and_then(parse_json_value) {
        collect_coding_evidence_value(&output, signals, step_ref, 0);
    } else if let Some(output) = step.output_excerpt.as_deref() {
        collect_command_machine_tokens(output, signals, step_ref);
    }
    if let Some(error) = step.error_excerpt.as_deref().and_then(parse_json_value) {
        collect_coding_evidence_value(&error, signals, step_ref, 0);
    } else if let Some(error) = step.error_excerpt.as_deref() {
        collect_command_machine_tokens(error, signals, step_ref);
    }
}

fn collect_coding_evidence_value(
    value: &Value,
    signals: &mut CodingEvidenceSignals,
    evidence_ref: Option<&str>,
    depth: usize,
) {
    if depth > 10 {
        return;
    }
    match value {
        Value::Object(map) => {
            collect_workspace_patch_fields(map, signals, evidence_ref);
            collect_read_file_fields(map, signals, evidence_ref);
            collect_changed_file_fields(map, signals, evidence_ref);
            collect_command_fields(map, signals, evidence_ref);
            collect_diff_summary_fields(map, signals, evidence_ref);
            collect_failure_fields(map, signals, evidence_ref);
            collect_retry_fields(map, signals);
            for value in map.values() {
                collect_coding_evidence_value(value, signals, evidence_ref, depth + 1);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_coding_evidence_value(item, signals, evidence_ref, depth + 1);
            }
        }
        Value::String(text) => {
            if let Some(value) = parse_json_value(text) {
                collect_coding_evidence_value(&value, signals, evidence_ref, depth + 1);
            } else {
                collect_command_machine_tokens(text, signals, evidence_ref);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn collect_workspace_patch_fields(
    map: &serde_json::Map<String, Value>,
    signals: &mut CodingEvidenceSignals,
    evidence_ref: Option<&str>,
) {
    if !matches!(
        map.get("source").and_then(Value::as_str),
        Some("workspace_patch" | "workspace_mutation")
    ) {
        return;
    }
    let before = signals.workspace_checkpoint_ids.len()
        + signals.patch_ids.len()
        + signals.mutation_ids.len();
    if let Some(value) = map
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_bounded_single_line_token(value))
    {
        signals.workspace_checkpoint_ids.insert(value.to_string());
    }
    if let Some(value) = map
        .get("patch_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_bounded_single_line_token(value))
    {
        signals.patch_ids.insert(value.to_string());
    }
    if let Some(value) = map
        .get("mutation_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_bounded_single_line_token(value))
    {
        signals.mutation_ids.insert(value.to_string());
    }
    if signals.workspace_checkpoint_ids.len() + signals.patch_ids.len() + signals.mutation_ids.len()
        > before
    {
        record_evidence_ref(signals, evidence_ref);
    }
}

fn collect_read_file_fields(
    map: &serde_json::Map<String, Value>,
    signals: &mut CodingEvidenceSignals,
    evidence_ref: Option<&str>,
) {
    let mut paths = BTreeSet::new();
    for key in [
        "files_read",
        "read_files",
        "opened_files",
        "input_files",
        "source_files",
    ] {
        collect_path_tokens(map.get(key), &mut paths);
    }
    if !paths.is_empty() {
        signals.files_read.extend(paths);
        record_evidence_ref(signals, evidence_ref);
    }
}

fn parse_json_value(text: &str) -> Option<Value> {
    serde_json::from_str::<Value>(text.trim()).ok()
}

fn collect_changed_file_fields(
    map: &serde_json::Map<String, Value>,
    signals: &mut CodingEvidenceSignals,
    evidence_ref: Option<&str>,
) {
    let mut paths = BTreeSet::new();
    for key in [
        "changed_files",
        "files_changed",
        "modified_files",
        "created_files",
        "deleted_files",
        "touched_files",
    ] {
        collect_path_tokens(map.get(key), &mut paths);
    }
    if !paths.is_empty() {
        signals.changed_files.extend(paths);
        record_evidence_ref(signals, evidence_ref);
    }
}

fn collect_path_tokens(value: Option<&Value>, out: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(path)) => {
            let path = path.trim();
            if is_bounded_single_line_token(path) {
                out.insert(path.to_string());
            }
        }
        Some(Value::Object(map)) => {
            for key in ["path", "file", "file_path", "resolved_path"] {
                collect_path_tokens(map.get(key), out);
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect_path_tokens(Some(item), out);
            }
        }
        Some(Value::Null | Value::Bool(_) | Value::Number(_)) | None => {}
    }
}

fn collect_command_fields(
    map: &serde_json::Map<String, Value>,
    signals: &mut CodingEvidenceSignals,
    evidence_ref: Option<&str>,
) {
    if let Some(command) = map.get("command").and_then(Value::as_str) {
        collect_command_token(command, signals, evidence_ref);
    }
    if let Some(command) = map
        .get("args")
        .and_then(Value::as_object)
        .and_then(|args| args.get("command"))
        .and_then(Value::as_str)
    {
        collect_command_token(command, signals, evidence_ref);
    }
    if let Some(command) = map.get("test_command").and_then(Value::as_str) {
        collect_command_token(command, signals, evidence_ref);
    }
    if let Some(command) = map.get("verification_command").and_then(Value::as_str) {
        collect_command_token(command, signals, evidence_ref);
    }
    if let Some(summary) = map.get("sanitized_args_summary").and_then(Value::as_str) {
        if let Some(command) = summary.trim().strip_prefix("command=") {
            collect_command_token(command, signals, evidence_ref);
        }
    }
}

fn collect_command_machine_tokens(
    text: &str,
    signals: &mut CodingEvidenceSignals,
    evidence_ref: Option<&str>,
) {
    for line in text.lines().map(str::trim) {
        if let Some(command) = line.strip_prefix("command=") {
            collect_command_token(command, signals, evidence_ref);
        } else if (line.starts_with("exit=") || line.starts_with("detached="))
            && line.contains(" command=")
        {
            if let Some((_, command)) = line.split_once(" command=") {
                collect_command_token(command, signals, evidence_ref);
                if line.starts_with("exit=") && !line.starts_with("exit=0 ") {
                    record_verification_failure_kind(command, signals);
                }
            }
        }
    }
}

fn collect_command_token(
    command: &str,
    signals: &mut CodingEvidenceSignals,
    evidence_ref: Option<&str>,
) {
    let command = command.trim();
    if !is_bounded_single_line_token(command) || command.len() > 500 {
        return;
    }
    signals.commands.insert(command.to_string());
    if is_verification_command_token(command) {
        signals.verification_commands.insert(command.to_string());
    }
    if is_test_command_token(command) {
        signals.tests.insert(command.to_string());
    }
    record_evidence_ref(signals, evidence_ref);
}

fn collect_diff_summary_fields(
    map: &serde_json::Map<String, Value>,
    signals: &mut CodingEvidenceSignals,
    evidence_ref: Option<&str>,
) {
    if signals.diff_summaries.len() >= 16 {
        return;
    }
    for key in [
        "diff_summary",
        "final_diff_summary",
        "change_summary",
        "patch_summary",
        "git_diff_summary",
    ] {
        let Some(value) = map.get(key).and_then(bounded_diff_summary_value) else {
            continue;
        };
        let summary = json!({
            "field": key,
            "value": value,
        });
        if signals.diff_summaries.contains(&summary) {
            continue;
        }
        signals.diff_summaries.push(summary);
        record_evidence_ref(signals, evidence_ref);
        if signals.diff_summaries.len() >= 16 {
            return;
        }
    }
}

fn bounded_diff_summary_value(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.len() > 2_000 {
                None
            } else {
                Some(Value::String(trimmed.to_string()))
            }
        }
        Value::Object(_) | Value::Array(_) => serde_json::to_string(value)
            .ok()
            .filter(|serialized| serialized.len() <= 4_000)
            .map(|_| value.clone()),
        Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::Null => None,
    }
}

fn collect_failure_fields(
    map: &serde_json::Map<String, Value>,
    signals: &mut CodingEvidenceSignals,
    evidence_ref: Option<&str>,
) {
    let Some(status) = map.get("status").and_then(Value::as_str) else {
        return;
    };
    if !is_failure_status_token(status) || signals.failures.len() >= 32 {
        return;
    }
    let has_step_identity = map.get("step_id").is_some()
        || map.get("action_ref").is_some()
        || map.get("requested_action_ref").is_some();
    if !has_step_identity {
        return;
    }
    signals.failures.push(json!({
        "step_id": map.get("step_id").cloned().unwrap_or(Value::Null),
        "status": status,
        "skill": map.get("skill").cloned().unwrap_or(Value::Null),
        "action_ref": map
            .get("action_ref")
            .or_else(|| map.get("requested_action_ref"))
            .cloned()
            .unwrap_or(Value::Null),
        "error_code": map
            .get("error_code")
            .or_else(|| map.get("error_kind"))
            .cloned()
            .unwrap_or(Value::Null),
    }));
    if let Some(command) = map.get("command").and_then(Value::as_str) {
        record_verification_failure_kind(command, signals);
    }
    record_evidence_ref(signals, evidence_ref);
}

fn collect_retry_fields(map: &serde_json::Map<String, Value>, signals: &mut CodingEvidenceSignals) {
    for key in [
        "repair_count",
        "retry_count",
        "retry_attempt",
        "repair_attempt",
    ] {
        if let Some(count) = map.get(key).and_then(Value::as_u64) {
            signals.retry_count = signals.retry_count.max(count);
        }
    }
}

fn is_failure_status_token(status: &str) -> bool {
    matches!(
        status.trim(),
        "error" | "failed" | "failure" | "timeout" | "cancelled" | "canceled"
    )
}

fn is_test_command_token(command: &str) -> bool {
    let command = command.trim().to_ascii_lowercase();
    command.starts_with("cargo test")
        || command.starts_with("npm test")
        || command.starts_with("npm run test")
        || command.starts_with("pnpm test")
        || command.starts_with("yarn test")
        || command.starts_with("pytest")
        || is_python_test_command_token(&command)
        || command.starts_with("go test")
}

fn is_verification_command_token(command: &str) -> bool {
    let command = command.trim().to_ascii_lowercase();
    is_test_command_token(&command)
        || command.starts_with("cargo check")
        || command.starts_with("cargo clippy")
        || command.starts_with("cargo fmt")
        || command.starts_with("npm run lint")
        || command.starts_with("npm run build")
        || command.starts_with("pnpm lint")
        || command.starts_with("pnpm build")
        || command.starts_with("yarn lint")
        || command.starts_with("yarn build")
        || command.starts_with("pytest")
        || is_python_test_command_token(&command)
        || command.starts_with("ruff check")
        || command.starts_with("go vet")
        || command.starts_with("go test")
}

fn is_python_test_command_token(command: &str) -> bool {
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        return false;
    };
    if !matches!(program, "python" | "python3") {
        return false;
    }
    let args = parts.collect::<Vec<_>>();
    if args.starts_with(&["-m", "pytest"]) || args.starts_with(&["-m", "unittest"]) {
        return true;
    }
    args.iter().any(|arg| {
        let arg = arg.trim_matches('"').trim_matches('\'');
        arg.starts_with("test_") || arg.ends_with("_test.py") || arg.contains("/test_")
    })
}

fn record_verification_failure_kind(command: &str, signals: &mut CodingEvidenceSignals) {
    let Some(kind) = verification_failure_kind_for_command(command) else {
        return;
    };
    signals.verification_failure_kinds.insert(kind.to_string());
}

fn verification_failure_kind_for_command(command: &str) -> Option<&'static str> {
    let command = command.trim().to_ascii_lowercase();
    if is_test_command_token(&command) {
        Some("test")
    } else if command.starts_with("cargo check") {
        Some("compile")
    } else if command.starts_with("cargo fmt") {
        Some("formatter")
    } else if command.starts_with("cargo clippy")
        || command.starts_with("npm run lint")
        || command.starts_with("pnpm lint")
        || command.starts_with("yarn lint")
        || command.starts_with("ruff check")
        || command.starts_with("go vet")
    {
        Some("lint")
    } else if command.starts_with("npm run build")
        || command.starts_with("pnpm build")
        || command.starts_with("yarn build")
    {
        Some("build")
    } else if is_verification_command_token(&command) {
        Some("other_verification")
    } else {
        None
    }
}

fn is_bounded_single_line_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 300 && !value.chars().any(|ch| matches!(ch, '\n' | '\r'))
}

fn record_evidence_ref(signals: &mut CodingEvidenceSignals, evidence_ref: Option<&str>) {
    let Some(evidence_ref) = evidence_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    signals.evidence_refs.insert(evidence_ref.to_string());
}

fn append_provider_call_events(seq: &mut u64, events: &mut Vec<Value>, journal: &TaskJournal) {
    let Some(by_prompt) = journal.task_metrics.by_prompt.as_ref() else {
        return;
    };
    let mut entries: Vec<(&String, &crate::LlmPromptBucket)> = by_prompt.iter().collect();
    entries.sort_by(|a, b| {
        b.1.count
            .cmp(&a.1.count)
            .then_with(|| b.1.elapsed_ms.cmp(&a.1.elapsed_ms))
            .then_with(|| a.0.cmp(b.0))
    });
    for (prompt_label, bucket) in entries {
        if bucket.count == 0
            && bucket.provider_attempt_count == 0
            && bucket.prompt_truncation_count == 0
        {
            continue;
        }
        events.push(task_event_json(
            seq,
            "provider_call",
            json!({
                "prompt_label": prompt_label,
                "llm_call_count": bucket.count,
                "elapsed_ms": bucket.elapsed_ms,
                "provider_attempt_count": bucket.provider_attempt_count,
                "provider_retry_count": bucket.provider_retry_count,
                "provider_retryable_error_count": bucket.provider_retryable_error_count,
                "provider_final_error_count": bucket.provider_final_error_count,
                "provider_last_retry_error_kinds": bucket.provider_last_retry_error_kinds,
                "provider_final_error_kinds": bucket.provider_final_error_kinds,
                "prompt_truncation_count": bucket.prompt_truncation_count,
                "prompt_bytes_before_max": bucket.prompt_bytes_before_max,
                "prompt_bytes_budget_min": bucket.prompt_bytes_budget_min,
                "prompt_bytes_after_max": bucket.prompt_bytes_after_max,
                "prompt_truncated_bytes_total": bucket.prompt_truncated_bytes_total,
            }),
        ));
    }
}
