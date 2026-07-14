use std::collections::BTreeSet;

use serde_json::{json, Map, Value};

use super::TaskJournal;

const MAX_CODING_ITEMS: usize = 24;

#[derive(Default)]
struct CodingWorkflowSignals {
    changed_files: BTreeSet<String>,
    verification_commands: BTreeSet<String>,
    checkpoint_refs: BTreeSet<String>,
    completed_side_effect_refs: BTreeSet<String>,
    failure_kinds: BTreeSet<String>,
    repair_step_refs: BTreeSet<String>,
    verified_observed: bool,
    failed_observed: bool,
}

pub(super) fn coding_workflow_summary_json(journal: &TaskJournal) -> Value {
    let signals = coding_workflow_signals(journal);
    let verification_status = verification_status(&signals);
    let changed_files = collapsed_path_values(&signals.changed_files);
    json!({
        "schema_version": 1,
        "source": "task_journal_observations",
        "current_phase_hint": current_phase_hint(&signals),
        "next_step": next_step(&signals, verification_status),
        "changed_file_count": changed_files.len(),
        "changed_files": changed_files,
        "verification_command_count": signals.verification_commands.len(),
        "verification_commands": bounded_set_values(&signals.verification_commands),
        "verification_status": verification_status,
        "failure_kind_count": signals.failure_kinds.len(),
        "failure_kinds": bounded_set_values(&signals.failure_kinds),
        "repair_attempt_count": signals.repair_step_refs.len(),
        "repair_attempt_refs": bounded_set_values(&signals.repair_step_refs),
        "checkpoint_ref_count": signals.checkpoint_refs.len(),
        "checkpoint_refs": bounded_set_values(&signals.checkpoint_refs),
        "completed_side_effect_count": signals.completed_side_effect_refs.len(),
        "completed_side_effect_refs": bounded_set_values(&signals.completed_side_effect_refs),
        "remaining_risks": remaining_risks(&signals, verification_status),
        "done_condition_coverage": done_condition_coverage(&signals, verification_status),
    })
}

fn coding_workflow_signals(journal: &TaskJournal) -> CodingWorkflowSignals {
    let mut signals = CodingWorkflowSignals::default();
    for observation in &journal.task_observations {
        let Some(map) = observation.as_object() else {
            continue;
        };
        match map.get("kind").and_then(Value::as_str) {
            Some("coding_state_transition") => collect_transition(map, &mut signals),
            Some("coding_checkpoint") => collect_checkpoint(map, &mut signals),
            _ => {}
        }
    }
    signals
}

fn collect_transition(map: &Map<String, Value>, signals: &mut CodingWorkflowSignals) {
    collect_string_array(map.get("changed_files"), &mut signals.changed_files);
    collect_string_field(
        map.get("verification_command"),
        &mut signals.verification_commands,
    );
    collect_string_array(
        map.get("completed_side_effect_refs"),
        &mut signals.completed_side_effect_refs,
    );
    if map.get("phase").and_then(Value::as_str) == Some("repair") {
        collect_step_ref(map, &mut signals.repair_step_refs);
    }
    if map.get("status").and_then(Value::as_str) == Some("error") {
        signals.failed_observed = true;
        collect_string_field(map.get("failure_kind"), &mut signals.failure_kinds);
    }
}

fn collect_checkpoint(map: &Map<String, Value>, signals: &mut CodingWorkflowSignals) {
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
    match map.get("verification_status").and_then(Value::as_str) {
        Some("verified") => signals.verified_observed = true,
        Some("failed") => signals.failed_observed = true,
        _ => {}
    }
    collect_string_field(map.get("failure_kind"), &mut signals.failure_kinds);
    if map.get("checkpoint_kind").and_then(Value::as_str) == Some("fix_applied") {
        collect_step_ref(map, &mut signals.repair_step_refs);
    }
}

fn verification_status(signals: &CodingWorkflowSignals) -> &'static str {
    if signals.failed_observed || !signals.failure_kinds.is_empty() {
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
    if signals.failed_observed || !signals.failure_kinds.is_empty() {
        "repair"
    } else if signals.verified_observed || !signals.verification_commands.is_empty() {
        "summarize"
    } else if !signals.changed_files.is_empty() {
        "verify"
    } else if !signals.checkpoint_refs.is_empty() {
        "background"
    } else {
        "idle"
    }
}

fn next_step(signals: &CodingWorkflowSignals, verification_status: &str) -> &'static str {
    match verification_status {
        "failed" => "repair_failed_verification",
        "unverified" => "run_verification",
        "verified" => "summarize",
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
