use serde_json::{json, Map, Value};

use super::{TaskJournal, TaskJournalFinalStatus};

const MAX_GOAL_LIST_ITEMS: usize = 12;

pub(super) fn task_goal_spec_from_payload_json(payload_json: &str) -> Option<Value> {
    let payload = serde_json::from_str::<Value>(payload_json).ok()?;
    payload
        .get("goal")
        .or_else(|| payload.get("goal_spec"))
        .or_else(|| payload.get("task_goal"))
        .filter(|value| value.is_object())
        .cloned()
}

pub(super) fn task_goal_summary_json(journal: &TaskJournal) -> Value {
    let mut goal = Map::new();
    goal.insert("schema_version".to_string(), json!(1));
    goal.insert("render_owner".to_string(), json!("finalizer_or_ui_i18n"));
    if let Some(task_id) = journal.task_id.as_deref() {
        goal.insert("task_id".to_string(), json!(task_id));
        goal.insert("goal_id".to_string(), json!(format!("task:{task_id}")));
    }
    merge_goal_spec_fields(&mut goal, journal.task_goal_spec.as_ref());

    let missing_evidence = missing_evidence_for_journal(journal);
    let validation = super::task_journal_validation_result::validation_result_json(journal);
    let validation_status = validation
        .get("latest_status")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let explicit_goal_status = journal
        .task_goal_spec
        .as_ref()
        .and_then(explicit_goal_status_token);
    let (goal_status, goal_status_source) = journal_goal_status(
        journal,
        missing_evidence.len(),
        validation_status,
        explicit_goal_status,
    );
    goal.insert("goal_status".to_string(), json!(goal_status));
    goal.insert("goal_status_source".to_string(), json!(goal_status_source));

    insert_optional_string(&mut goal, "validation_status", validation_status);
    insert_optional_string(
        &mut goal,
        "last_checkpoint_id",
        latest_checkpoint_ref(journal).as_deref(),
    );
    insert_optional_string(
        &mut goal,
        "last_successful_evidence_ref",
        last_successful_evidence_ref(journal).as_deref(),
    );
    merge_string_array(
        &mut goal,
        "verification_commands",
        verification_commands(journal),
    );
    insert_string_array(
        &mut goal,
        "success_evidence_refs",
        success_evidence_refs(journal),
    );
    insert_string_array(
        &mut goal,
        "current_progress",
        current_progress(journal, missing_evidence.len(), validation_status),
    );
    insert_string_array(
        &mut goal,
        "remaining_work",
        remaining_work(journal, &missing_evidence, validation_status),
    );
    if !missing_evidence.is_empty() {
        goal.insert("missing_evidence".to_string(), json!(missing_evidence));
    }

    Value::Object(goal)
}

fn journal_goal_status(
    journal: &TaskJournal,
    missing_evidence_count: usize,
    validation_status: Option<&str>,
    explicit_goal_status: Option<&'static str>,
) -> (&'static str, &'static str) {
    if let Some(state) = journal
        .task_lifecycle
        .as_ref()
        .and_then(|lifecycle| {
            lifecycle
                .get("state")
                .or_else(|| lifecycle.get("execution_state"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
    {
        match state {
            "cancelled" | "canceled" => return ("cancelled", "task_lifecycle"),
            "background" | "waiting" => return ("background", "task_lifecycle"),
            "needs_user" | "needs_confirmation" => return ("waiting_user", "task_lifecycle"),
            _ => {}
        }
    }

    match journal.final_status {
        Some(TaskJournalFinalStatus::Success) if missing_evidence_count == 0 => {
            ("completed", "journal_final_status")
        }
        Some(TaskJournalFinalStatus::Success) => ("blocked", "evidence_coverage"),
        Some(TaskJournalFinalStatus::Clarify) => ("waiting_user", "journal_final_status"),
        Some(TaskJournalFinalStatus::Failure | TaskJournalFinalStatus::ResumeFailure) => {
            ("blocked", "journal_final_status")
        }
        None if validation_status == Some("passed") => ("verified", "validation_result"),
        None if let Some(status) = explicit_goal_status => (status, "goal"),
        None => ("in_progress", "journal_final_status"),
    }
}

fn explicit_goal_status_token(goal: &Value) -> Option<&'static str> {
    goal.get("goal_status")
        .or_else(|| goal.get("state"))
        .and_then(Value::as_str)
        .and_then(canonical_goal_status_token)
}

fn canonical_goal_status_token(value: &str) -> Option<&'static str> {
    match value.trim() {
        "created" => Some("created"),
        "in_progress" => Some("in_progress"),
        "waiting_user" => Some("waiting_user"),
        "background" => Some("background"),
        "blocked" => Some("blocked"),
        "verified" => Some("verified"),
        "completed" => Some("completed"),
        "cancelled" | "canceled" => Some("cancelled"),
        _ => None,
    }
}

fn merge_goal_spec_fields(goal: &mut Map<String, Value>, spec: Option<&Value>) {
    let Some(spec) = spec else {
        return;
    };
    for key in [
        "goal_id",
        "objective",
        "constraints",
        "done_conditions",
        "verification_commands",
        "allowed_files_or_scopes",
        "forbidden_actions",
    ] {
        copy_non_empty(goal, spec, key);
    }
}

fn current_progress(
    journal: &TaskJournal,
    missing_evidence_count: usize,
    validation_status: Option<&str>,
) -> Vec<String> {
    let mut progress = Vec::new();
    if let Some(status) = journal.final_status {
        progress.push(format!("final_status={}", status.as_str()));
    }
    let ok_count = journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .count();
    let error_count = journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Error)
        .count();
    if !journal.step_results.is_empty() {
        progress.push(format!("step_count={}", journal.step_results.len()));
        progress.push(format!("ok_step_count={ok_count}"));
        progress.push(format!("error_step_count={error_count}"));
    }
    if let Some(status) = validation_status {
        progress.push(format!("validation_status={status}"));
    }
    progress.push(format!("missing_evidence_count={missing_evidence_count}"));
    if let Some(checkpoint) = latest_checkpoint_ref(journal) {
        progress.push(format!("last_checkpoint_id={checkpoint}"));
    }
    progress.truncate(MAX_GOAL_LIST_ITEMS);
    progress
}

fn remaining_work(
    journal: &TaskJournal,
    missing_evidence: &[String],
    validation_status: Option<&str>,
) -> Vec<String> {
    let mut work = missing_evidence
        .iter()
        .take(MAX_GOAL_LIST_ITEMS)
        .map(|field| format!("missing_evidence={field}"))
        .collect::<Vec<_>>();
    if matches!(validation_status, Some("failed" | "validation_failed")) {
        work.push("validation_status=failed".to_string());
    }
    if journal.final_status.is_none() {
        work.push("agent_loop_status=in_progress".to_string());
    }
    work.truncate(MAX_GOAL_LIST_ITEMS);
    work
}

fn missing_evidence_for_journal(journal: &TaskJournal) -> Vec<String> {
    journal
        .output_contract
        .as_ref()
        .map(|contract| {
            super::task_journal_evidence_coverage::evidence_coverage_for_output_contract(
                contract, journal,
            )
            .missing_evidence
        })
        .unwrap_or_default()
}

fn success_evidence_refs(journal: &TaskJournal) -> Vec<String> {
    let mut refs = journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .map(|step| format!("step:{}", step.step_id))
        .collect::<Vec<_>>();
    for checkpoint in coding_checkpoints(journal) {
        if checkpoint
            .get("verification_status")
            .and_then(Value::as_str)
            .is_some_and(|status| status != "failed")
        {
            if let Some(reference) = checkpoint
                .get("evidence_ref")
                .or_else(|| checkpoint.get("checkpoint_ref"))
                .and_then(Value::as_str)
            {
                refs.push(reference.to_string());
            }
        }
    }
    refs.sort();
    refs.dedup();
    refs.truncate(MAX_GOAL_LIST_ITEMS);
    refs
}

fn last_successful_evidence_ref(journal: &TaskJournal) -> Option<String> {
    journal
        .step_results
        .iter()
        .rev()
        .find(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .map(|step| format!("step:{}", step.step_id))
}

fn latest_checkpoint_ref(journal: &TaskJournal) -> Option<String> {
    journal
        .task_checkpoint
        .as_ref()
        .and_then(|checkpoint| {
            checkpoint
                .get("checkpoint_id")
                .or_else(|| checkpoint.get("checkpoint_ref"))
                .or_else(|| checkpoint.get("evidence_ref"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| {
            coding_checkpoints(journal).last().and_then(|checkpoint| {
                checkpoint
                    .get("checkpoint_ref")
                    .or_else(|| checkpoint.get("evidence_ref"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
}

fn verification_commands(journal: &TaskJournal) -> Vec<String> {
    let mut commands = Vec::new();
    for checkpoint in coding_checkpoints(journal) {
        if let Some(command) = checkpoint
            .get("verification_command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            commands.push(command.to_string());
        }
    }
    commands.sort();
    commands.dedup();
    commands.truncate(MAX_GOAL_LIST_ITEMS);
    commands
}

fn coding_checkpoints(journal: &TaskJournal) -> Vec<&Value> {
    journal
        .task_observations
        .iter()
        .filter(|value| value.get("kind").and_then(Value::as_str) == Some("coding_checkpoint"))
        .collect()
}

fn insert_optional_string(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        map.insert(key.to_string(), json!(value));
    }
}

fn insert_string_array(map: &mut Map<String, Value>, key: &str, values: Vec<String>) {
    if !values.is_empty() {
        map.insert(key.to_string(), json!(values));
    }
}

fn merge_string_array(map: &mut Map<String, Value>, key: &str, values: Vec<String>) {
    let mut merged = map
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    merged.extend(values);
    merged.sort();
    merged.dedup();
    merged.truncate(MAX_GOAL_LIST_ITEMS);
    if !merged.is_empty() {
        map.insert(key.to_string(), json!(merged));
    }
}

fn copy_non_empty(map: &mut Map<String, Value>, source: &Value, key: &str) {
    let Some(value) = source.get(key) else {
        return;
    };
    if value_is_empty(value) {
        return;
    }
    map.insert(key.to_string(), value.clone());
}

fn value_is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(value) => value.trim().is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(values) => values.is_empty(),
        Value::Bool(_) | Value::Number(_) => false,
    }
}
