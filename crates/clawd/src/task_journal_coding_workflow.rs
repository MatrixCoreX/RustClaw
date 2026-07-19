use std::collections::BTreeSet;

use serde_json::{json, Map, Value};

use super::task_journal_coding_state::{
    coding_milestone_checkpoint_observation, coding_state_transition_observation_from_trace,
};
use super::TaskJournal;

const MAX_CODING_ITEMS: usize = 24;

#[derive(Default)]
struct CodingWorkflowSignals {
    planned_changes: BTreeSet<String>,
    diff_refs: BTreeSet<String>,
    changed_files: BTreeSet<String>,
    verification_commands: BTreeSet<String>,
    checkpoint_refs: BTreeSet<String>,
    completed_side_effect_refs: BTreeSet<String>,
    failed_commands: BTreeSet<String>,
    failed_command_refs: BTreeSet<String>,
    failed_command_stderr_refs: BTreeSet<String>,
    failure_kinds: BTreeSet<String>,
    repair_step_refs: BTreeSet<String>,
    verified_observed: bool,
    failed_observed: bool,
    latest_verification_outcome: Option<VerificationOutcome>,
    latest_verification_step_ref: Option<String>,
    projection_revision: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerificationOutcome {
    Failed,
    Verified,
}

impl VerificationOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Verified => "verified",
        }
    }
}

pub(super) fn coding_workflow_summary_json(journal: &TaskJournal) -> Value {
    let signals = coding_workflow_signals(journal);
    let verification_status = verification_status(&signals);
    let changed_files = collapsed_path_values(&signals.changed_files);
    let current_failure_kinds = if verification_status == "failed" {
        bounded_set_values(&signals.failure_kinds)
    } else {
        Vec::new()
    };
    json!({
        "schema_version": 2,
        "source": "task_journal_observations",
        "projection_revision": signals.projection_revision,
        "latest_verification_step_ref": signals.latest_verification_step_ref,
        "current_phase_hint": current_phase_hint(&signals),
        "next_step": next_step(&signals, verification_status),
        "planned_change_count": signals.planned_changes.len(),
        "planned_changes": bounded_set_values(&signals.planned_changes),
        "diff_ref_count": signals.diff_refs.len(),
        "diff_refs": bounded_set_values(&signals.diff_refs),
        "changed_file_count": changed_files.len(),
        "changed_files": changed_files,
        "verification_command_count": signals.verification_commands.len(),
        "verification_commands": bounded_set_values(&signals.verification_commands),
        "verification_status": verification_status,
        "failure_kind_count": current_failure_kinds.len(),
        "failure_kinds": current_failure_kinds,
        "historical_failure_kind_count": signals.failure_kinds.len(),
        "historical_failure_kinds": bounded_set_values(&signals.failure_kinds),
        "repair_attempt_count": signals.repair_step_refs.len(),
        "repair_attempt_refs": bounded_set_values(&signals.repair_step_refs),
        "checkpoint_ref_count": signals.checkpoint_refs.len(),
        "checkpoint_refs": bounded_set_values(&signals.checkpoint_refs),
        "completed_side_effect_count": signals.completed_side_effect_refs.len(),
        "completed_side_effect_refs": bounded_set_values(&signals.completed_side_effect_refs),
        "failed_command_count": signals.failed_commands.len(),
        "failed_commands": bounded_set_values(&signals.failed_commands),
        "failed_command_ref_count": signals.failed_command_refs.len(),
        "failed_command_refs": bounded_set_values(&signals.failed_command_refs),
        "failed_command_stderr_ref_count": signals.failed_command_stderr_refs.len(),
        "failed_command_stderr_refs": bounded_set_values(&signals.failed_command_stderr_refs),
        "remaining_risks": remaining_risks(&signals, verification_status),
        "done_condition_coverage": done_condition_coverage(&signals, verification_status),
        "validation_gate": validation_gate_json(&signals, verification_status),
    })
}

fn coding_workflow_signals(journal: &TaskJournal) -> CodingWorkflowSignals {
    let mut signals = CodingWorkflowSignals {
        projection_revision: journal.step_results.len() + journal.task_observations.len(),
        ..CodingWorkflowSignals::default()
    };
    for observation in &journal.task_observations {
        collect_observation(observation, &mut signals);
    }
    let mut authoritative_outcome = None;
    for step in &journal.step_results {
        let Some(transition) = coding_state_transition_observation_from_trace(step) else {
            continue;
        };
        collect_observation(&transition, &mut signals);
        if let Some(outcome) = verification_outcome_from_transition(&transition) {
            authoritative_outcome = Some((outcome, step.step_id.clone()));
        }
        if let Some(checkpoint) = coding_milestone_checkpoint_observation(&transition, &[]) {
            collect_observation(&checkpoint, &mut signals);
        }
    }
    if let Some((outcome, step_ref)) = authoritative_outcome {
        signals.latest_verification_outcome = Some(outcome);
        signals.latest_verification_step_ref = Some(step_ref);
    }
    signals
}

fn collect_observation(observation: &Value, signals: &mut CodingWorkflowSignals) {
    let Some(map) = observation.as_object() else {
        return;
    };
    match map.get("kind").and_then(Value::as_str) {
        Some("coding_state_transition") => collect_transition(map, signals),
        Some("coding_checkpoint") => collect_checkpoint(map, signals),
        _ => {}
    }
}

fn collect_transition(map: &Map<String, Value>, signals: &mut CodingWorkflowSignals) {
    collect_string_array(map.get("planned_changes"), &mut signals.planned_changes);
    collect_string_array(map.get("diff_refs"), &mut signals.diff_refs);
    collect_string_array(map.get("changed_files"), &mut signals.changed_files);
    collect_string_field(
        map.get("verification_command"),
        &mut signals.verification_commands,
    );
    collect_string_array(
        map.get("completed_side_effect_refs"),
        &mut signals.completed_side_effect_refs,
    );
    collect_string_array(map.get("failed_commands"), &mut signals.failed_commands);
    collect_string_array(
        map.get("failed_command_refs"),
        &mut signals.failed_command_refs,
    );
    collect_string_array(
        map.get("failed_command_stderr_refs"),
        &mut signals.failed_command_stderr_refs,
    );
    if map.get("phase").and_then(Value::as_str) == Some("repair") {
        collect_step_ref(map, &mut signals.repair_step_refs);
    }
    if map.get("status").and_then(Value::as_str) == Some("error") {
        signals.failed_observed = true;
        collect_string_field(map.get("failure_kind"), &mut signals.failure_kinds);
    }
    if map
        .get("verification_command")
        .and_then(Value::as_str)
        .is_some()
    {
        match map.get("status").and_then(Value::as_str) {
            Some("error") => {
                signals.latest_verification_outcome = Some(VerificationOutcome::Failed)
            }
            Some("ok") => signals.latest_verification_outcome = Some(VerificationOutcome::Verified),
            _ => {}
        }
        signals.latest_verification_step_ref = map
            .get("step_id")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
}

fn collect_checkpoint(map: &Map<String, Value>, signals: &mut CodingWorkflowSignals) {
    collect_string_array(map.get("planned_changes"), &mut signals.planned_changes);
    collect_string_array(map.get("diff_refs"), &mut signals.diff_refs);
    collect_string_array(map.get("changed_files"), &mut signals.changed_files);
    collect_string_field(
        map.get("verification_command"),
        &mut signals.verification_commands,
    );
    collect_string_field(map.get("checkpoint_ref"), &mut signals.checkpoint_refs);
    collect_string_field(map.get("evidence_ref"), &mut signals.checkpoint_refs);
    collect_string_array(
        map.get("completed_side_effect_refs"),
        &mut signals.completed_side_effect_refs,
    );
    collect_string_array(map.get("failed_commands"), &mut signals.failed_commands);
    collect_string_array(
        map.get("failed_command_refs"),
        &mut signals.failed_command_refs,
    );
    collect_string_array(
        map.get("failed_command_stderr_refs"),
        &mut signals.failed_command_stderr_refs,
    );
    match map.get("verification_status").and_then(Value::as_str) {
        Some("verified") => {
            signals.verified_observed = true;
            signals.latest_verification_outcome = Some(VerificationOutcome::Verified);
        }
        Some("failed") => {
            signals.failed_observed = true;
            signals.latest_verification_outcome = Some(VerificationOutcome::Failed);
        }
        _ => {}
    }
    if map
        .get("verification_status")
        .and_then(Value::as_str)
        .is_some()
    {
        signals.latest_verification_step_ref = map
            .get("source_step_id")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    collect_string_field(map.get("failure_kind"), &mut signals.failure_kinds);
    if map.get("checkpoint_kind").and_then(Value::as_str) == Some("fix_applied") {
        collect_step_ref(map, &mut signals.repair_step_refs);
    }
}

fn verification_status(signals: &CodingWorkflowSignals) -> &'static str {
    if let Some(outcome) = signals.latest_verification_outcome {
        outcome.as_str()
    } else if signals.failed_observed || !signals.failure_kinds.is_empty() {
        "failed"
    } else if signals.verified_observed || !signals.verification_commands.is_empty() {
        "verified"
    } else if !signals.changed_files.is_empty() {
        "unverified"
    } else {
        "not_applicable"
    }
}

fn current_phase_hint(signals: &CodingWorkflowSignals) -> &'static str {
    match verification_status(signals) {
        "failed" => "repair",
        "verified" => "summarize",
        "unverified" => "verify",
        _ if !signals.diff_refs.is_empty() => "review",
        _ if !signals.checkpoint_refs.is_empty() => "background",
        _ => "idle",
    }
}

fn verification_outcome_from_transition(value: &Value) -> Option<VerificationOutcome> {
    value
        .get("verification_command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())?;
    match value.get("status").and_then(Value::as_str) {
        Some("error") => Some(VerificationOutcome::Failed),
        Some("ok") => Some(VerificationOutcome::Verified),
        _ => None,
    }
}

fn next_step(signals: &CodingWorkflowSignals, verification_status: &str) -> &'static str {
    match verification_status {
        "failed" => "repair_failed_verification",
        "unverified" => "run_verification",
        "verified" => "summarize",
        _ if !signals.diff_refs.is_empty() => "summarize",
        _ if !signals.checkpoint_refs.is_empty() => "resume_from_checkpoint",
        _ => "inspect",
    }
}

fn remaining_risks(signals: &CodingWorkflowSignals, verification_status: &str) -> Value {
    let mut risks = Vec::new();
    if verification_status == "failed" {
        risks.push("failed_verification");
    }
    if verification_status == "unverified" && !signals.changed_files.is_empty() {
        risks.push("unverified_changes");
    }
    json!(risks)
}

fn done_condition_coverage(signals: &CodingWorkflowSignals, verification_status: &str) -> Value {
    json!([
        {
            "condition": "changes",
            "status": if signals.changed_files.is_empty() { "not_observed" } else { "observed" },
        },
        {
            "condition": "verification",
            "status": verification_status,
        },
        {
            "condition": "repair",
            "status": if signals.repair_step_refs.is_empty() { "not_observed" } else { "observed" },
        },
    ])
}

fn validation_gate_json(signals: &CodingWorkflowSignals, verification_status: &str) -> Value {
    let has_changes = !signals.changed_files.is_empty();
    let can_report_fully_verified =
        verification_status != "failed" && (!has_changes || verification_status == "verified");
    let gate_status = if verification_status == "failed" {
        "repair_required"
    } else if has_changes && verification_status != "verified" {
        "verification_required"
    } else {
        "satisfied"
    };
    let repair_signal = if verification_status == "failed" {
        json!({
            "signal_kind": "verification_failed",
            "next_step": "repair_failed_verification",
            "failure_kind_count": signals.failure_kinds.len(),
            "failure_kinds": bounded_set_values(&signals.failure_kinds),
            "failed_command_count": signals.failed_commands.len(),
            "failed_command_ref_count": signals.failed_command_refs.len(),
            "failed_command_stderr_ref_count": signals.failed_command_stderr_refs.len(),
        })
    } else {
        Value::Null
    };
    json!({
        "schema_version": 1,
        "gate_status": gate_status,
        "can_report_fully_verified": can_report_fully_verified,
        "requires_verification": has_changes && verification_status != "verified",
        "requires_repair": verification_status == "failed",
        "checkpoint_recommended": !can_report_fully_verified
            && (!signals.checkpoint_refs.is_empty() || !signals.completed_side_effect_refs.is_empty()),
        "repair_signal": repair_signal,
    })
}

fn collect_step_ref(map: &Map<String, Value>, refs: &mut BTreeSet<String>) {
    if let Some(step_id) = map
        .get("step_id")
        .or_else(|| map.get("source_step_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        refs.insert(format!("step:{step_id}"));
    }
}

fn collect_string_field(value: Option<&Value>, out: &mut BTreeSet<String>) {
    if let Some(value) = value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        out.insert(value.to_string());
    }
}

fn collect_string_array(value: Option<&Value>, out: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(value)) => {
            let value = value.trim();
            if !value.is_empty() {
                out.insert(value.to_string());
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect_string_array(Some(item), out);
            }
        }
        Some(Value::Object(_)) | Some(Value::Null | Value::Bool(_) | Value::Number(_)) | None => {}
    }
}

fn bounded_set_values(values: &BTreeSet<String>) -> Vec<String> {
    values.iter().take(MAX_CODING_ITEMS).cloned().collect()
}

fn collapsed_path_values(values: &BTreeSet<String>) -> Vec<String> {
    let mut collapsed = Vec::<String>::new();
    for value in values {
        let value = value.trim().replace('\\', "/");
        if value.is_empty() {
            continue;
        }
        if let Some(index) = collapsed
            .iter()
            .position(|existing| path_suffix_equivalent(existing, &value))
        {
            if value.len() < collapsed[index].len() {
                collapsed[index] = value;
            }
        } else {
            collapsed.push(value);
        }
    }
    collapsed.sort();
    collapsed.truncate(MAX_CODING_ITEMS);
    collapsed
}

fn path_suffix_equivalent(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let left_suffix = format!("/{left}");
    let right_suffix = format!("/{right}");
    left.ends_with(&right_suffix) || right.ends_with(&left_suffix)
}
