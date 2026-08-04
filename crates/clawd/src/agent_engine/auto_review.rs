use serde_json::{json, Value};

use crate::{
    policy_decision::PolicyDecision,
    task_lifecycle::{CheckpointBudgetCounters, ResumeEntrypoint, TaskCheckpoint},
    AppState, AskReply, ClaimedTask,
};

const REVIEW_CAPABILITIES: &[&str] = &[
    "filesystem.list_directory",
    "filesystem.read_file",
    "filesystem.search",
];

pub(crate) async fn run_one_shot_auto_review(state: &AppState, task: &ClaimedTask) -> AskReply {
    let mut answer = AskReply::non_llm("auto_review_completed".to_string());
    attach_auto_review(state, task, &mut answer).await;
    answer
}

pub(crate) async fn attach_auto_review(
    state: &AppState,
    task: &ClaimedTask,
    answer: &mut AskReply,
) {
    let payload = serde_json::from_str::<Value>(&task.payload_json).unwrap_or(Value::Null);
    let coding = payload.get("execution_profile").and_then(Value::as_str) == Some("coding");
    let one_shot = payload.get("auto_review_once").and_then(Value::as_bool) == Some(true);
    if !coding || (!state.reload_ctx.auto_review.enabled && !one_shot) {
        return;
    }
    let target_task_id = payload
        .get("review_target_task_id")
        .and_then(Value::as_str)
        .unwrap_or(&task.task_id);
    let role = state.reload_ctx.auto_review.review_role.trim();
    let child_input = json!({
        "schema_version": 2,
        "role": role,
        "objective": "auto_review_coding_changes",
        "review_target_task_id": target_task_id,
        "runtime_policy": {
            "write_enabled": false,
            "external_publish_enabled": false,
            "tool_permission_profile": "read_only",
        },
        "context_refs": [
            format!("task:{target_task_id}:coding_evidence"),
            "workspace:current_changes"
        ],
        "allowed_capabilities": REVIEW_CAPABILITIES,
        "budget": {
            "max_rounds": 6,
            "max_tool_calls": 12,
            "max_tokens": 200000,
        },
        "timeout_policy": {
            "schema_version": 2,
            "policy": "no_operation_deadline",
            "runtime_deadline_ms": null,
            "join_wait_expires_child": false,
        },
        "result_contract": {
            "output_format": "machine_json",
            "required_keys": ["review_findings"],
            "require_evidence": true,
        },
    });
    let resumed_after_confirmation =
        payload.get("resume_trigger").and_then(Value::as_str) == Some("user_followup");
    let observation = match super::subagent_runtime::run_readonly_child_agent_loop(
        state,
        task,
        &child_input,
        None,
    )
    .await
    {
        Ok(result) => review_observation(
            target_task_id,
            role,
            &result,
            payload
                .get("auto_review_blocking")
                .and_then(Value::as_bool)
                .unwrap_or(state.reload_ctx.auto_review.blocking)
                && !resumed_after_confirmation,
        ),
        Err(_) => json!({
            "schema_version": 1,
            "owner_layer": "auto_review",
            "status": "error",
            "error_code": "auto_review_child_failed",
            "message_key": "agent.auto_review.failed",
            "retryable": true,
            "review_target_task_id": target_task_id,
            "review_role": role,
            "review_findings": [],
            "policy_decision": PolicyDecision::Allow.as_token(),
        }),
    };
    let confirmation_required = observation
        .get("confirmation_required")
        .and_then(Value::as_bool)
        == Some(true);
    answer
        .task_journal
        .get_or_insert_with(|| {
            crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "auto_review")
        })
        .push_task_observation(observation.clone());
    if confirmation_required {
        attach_review_confirmation_boundary(state, task, answer, &observation).await;
    }
}

async fn attach_review_confirmation_boundary(
    state: &AppState,
    task: &ClaimedTask,
    answer: &mut AskReply,
    observation: &Value,
) {
    let finding_count = observation
        .get("review_findings")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let (visible, mut resume_context) = super::build_confirmation_required_resume_context(
        state,
        task,
        &[],
        "auto_review_delivery_confirmation",
        "review_delivery_gate",
        &[],
        &[],
        &json!({"finding_count":finding_count}).to_string(),
        &[],
    )
    .await;
    let checkpoint_id = format!("auto-review-{}", uuid::Uuid::new_v4().simple());
    resume_context["owner_layer"] = json!("auto_review");
    resume_context["review_findings"] = observation
        .get("review_findings")
        .cloned()
        .unwrap_or_else(|| json!([]));
    resume_context["checkpoint_id"] = json!(checkpoint_id);

    let journal = answer.task_journal.get_or_insert_with(|| {
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "auto_review")
    });
    let completed_side_effect_refs = journal
        .to_summary_json()
        .pointer("/coding_workflow/completed_side_effect_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let evidence_refs = observation
        .get("review_findings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|finding| finding.get("suggestion_ref").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context: json!({
            "schema_version":1,
            "source":"auto_review",
            "policy_decision":PolicyDecision::RequireConfirmation.as_token(),
            "message_key":"agent.auto_review.confirmation_required",
            "finding_count":finding_count,
            "completed_side_effect_refs":completed_side_effect_refs,
        }),
        last_successful_round: None,
        last_successful_step: None,
        pending_action: Some(json!({
            "schema_version":1,
            "kind":"auto_review_delivery_confirmation",
            "resume_expected":"user_followup",
        })),
        observations: vec![observation.clone()],
        capability_results: Vec::new(),
        evidence_refs,
        artifact_refs: Vec::new(),
        completed_side_effect_refs,
        budget: CheckpointBudgetCounters {
            round: 0,
            step: 0,
            llm_calls: 0,
            tool_calls: 0,
            elapsed_ms: 0,
            llm_elapsed_ms: 0,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: ResumeEntrypoint::AwaitUserInput,
    };
    journal.record_task_lifecycle(json!({
        "schema_version":1,
        "state":"needs_user",
        "source":"auto_review",
        "resume_reason":"auto_review_confirmation_required",
        "checkpoint_id":checkpoint_id,
        "can_poll":true,
        "can_cancel":true,
        "message_key":"agent.auto_review.confirmation_required",
        "decision":PolicyDecision::RequireConfirmation.as_token(),
    }));
    journal.record_task_checkpoint(checkpoint.to_machine_json());
    answer.text = visible.clone();
    answer.messages = vec![visible];
    answer.resume_context = Some(resume_context);
}

fn review_observation(target_task_id: &str, role: &str, result: &Value, blocking: bool) -> Value {
    let raw_findings = result
        .pointer("/result/review_findings")
        .or_else(|| result.get("review_findings"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut rejected = 0_u64;
    let findings = raw_findings
        .iter()
        .filter_map(|finding| match normalize_finding(finding) {
            Some(finding) => Some(finding),
            None => {
                rejected += 1;
                None
            }
        })
        .take(128)
        .collect::<Vec<_>>();
    let blocking_error = blocking
        && findings
            .iter()
            .any(|finding| finding.get("severity").and_then(Value::as_str) == Some("error"));
    json!({
        "schema_version": 1,
        "owner_layer": "auto_review",
        "status": "completed",
        "review_target_task_id": target_task_id,
        "review_role": role,
        "blocking": blocking,
        "readonly_enforced": true,
        "review_findings": findings,
        "rejected_finding_count": rejected,
        "policy_decision": if blocking_error {
            PolicyDecision::RequireConfirmation.as_token()
        } else {
            PolicyDecision::Allow.as_token()
        },
        "confirmation_required": blocking_error,
        "message_key": if blocking_error {
            "agent.auto_review.confirmation_required"
        } else {
            "agent.auto_review.completed"
        },
    })
}

fn normalize_finding(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let severity = object.get("severity")?.as_str()?;
    if !matches!(severity, "info" | "warning" | "error") {
        return None;
    }
    let file = object.get("file")?.as_str()?.trim();
    if file.is_empty() || file.len() > 512 || file.starts_with('/') || file.contains("..") {
        return None;
    }
    let line_range = object.get("line_range")?.as_object()?;
    let start = line_range.get("start")?.as_u64()?;
    let end = line_range.get("end")?.as_u64()?;
    if start == 0 || end < start || end > 10_000_000 {
        return None;
    }
    let finding_code = machine_token(object.get("finding_code")?.as_str()?)?;
    let message_key = machine_key(object.get("message_key")?.as_str()?)?;
    let suggestion_ref = object
        .get("suggestion_ref")
        .and_then(Value::as_str)
        .and_then(machine_ref);
    Some(json!({
        "severity": severity,
        "file": file,
        "line_range": {"start":start,"end":end},
        "finding_code": finding_code,
        "message_key": message_key,
        "suggestion_ref": suggestion_ref,
    }))
}

fn machine_token(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 96
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'))
    .then_some(value)
}

fn machine_key(value: &str) -> Option<&str> {
    let value = value.trim();
    (value.contains('.')
        && value.len() <= 160
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '.' | '-')
        }))
    .then_some(value)
}

fn machine_ref(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 512
        && !value.chars().any(char::is_control)
        && !value.contains("../"))
    .then_some(value)
}

#[cfg(test)]
#[path = "auto_review_tests.rs"]
mod tests;
