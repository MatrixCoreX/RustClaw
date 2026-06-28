use serde_json::{json, Value};

use super::{
    capability_resolution_source, next_requested_capability, requested_capability_sequence,
    step_action_kind, TaskJournal, TaskJournalFinalStatus,
};

fn task_event_json(seq: &mut u64, event_type: &'static str, payload: Value) -> Value {
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
    if let Some(lifecycle) = journal.task_lifecycle.as_ref() {
        events.push(task_event_json(
            &mut seq,
            "task_lifecycle",
            lifecycle.clone(),
        ));
    }
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
    }
    append_provider_call_events(&mut seq, &mut events, journal);
    let mut requested = requested_capability_sequence(journal);
    for (index, step) in journal.step_results.iter().enumerate() {
        let requested = next_requested_capability(&mut requested, step);
        let action_kind = step_action_kind(step, requested.as_ref());
        let step_trace = super::step_trace_json(step, requested.as_ref(), None);
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
                    .unwrap_or("step_trace_compat"),
                "status": step.status.as_str(),
                "error_kind": step_trace.get("error_kind"),
                "failure_attribution": step_trace.get("failure_attribution"),
                "output_evidence_count": step_trace.get("output_evidence_count"),
                "artifact_ref_count": step_trace.get("artifact_ref_count"),
                "artifact_refs": step_trace.get("artifact_refs").cloned().unwrap_or_else(|| json!([])),
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
            == Some("subagent_runtime")
        {
            let mut payload = observation.clone();
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("index".to_string(), json!(index));
            }
            events.push(task_event_json(&mut seq, "subagent", payload));
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
            .unwrap_or("step_trace_compat"),
        "status": step.status.as_str(),
        "error_kind": step_trace.get("error_kind"),
        "failure_attribution": step_trace.get("failure_attribution"),
        "output_evidence_count": step_trace.get("output_evidence_count"),
        "artifact_ref_count": step_trace.get("artifact_ref_count"),
        "started_at": step.started_at,
        "finished_at": step.finished_at,
    })
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
