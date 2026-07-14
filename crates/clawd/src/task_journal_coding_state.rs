use serde_json::{json, Map, Value};

pub(super) fn coding_state_transition_observation(
    step_result: &crate::executor::StepExecutionResult,
) -> Option<Value> {
    let mut signals = CodingTransitionSignals::default();
    let parsed_output = step_result
        .output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output.trim()).ok());
    if let Some(value) = parsed_output.as_ref() {
        collect_value_signals(value, &mut signals);
    }
    if let Some(output) = step_result.output.as_deref() {
        collect_machine_line_signals(output, &mut signals);
    }
    if let Some(error) = step_result.error.as_deref() {
        collect_machine_line_signals(error, &mut signals);
    }
    if signals.action.is_none() {
        signals.action = parsed_output.as_ref().and_then(action_from_value);
    }
    if should_collect_skill_phase(step_result.skill.as_str()) {
        signals.skill_signal = true;
    }
    if !signals.has_coding_signal() {
        return None;
    }
    let status = step_result.status.as_str();
    let phase = coding_phase(step_result, &signals);
    let next_phase_hint = coding_next_phase_hint(status, phase, &signals);
    let mut payload = json!({
        "kind": "coding_state_transition",
        "schema_version": 1,
        "step_id": step_result.step_id,
        "skill": step_result.skill,
        "status": status,
        "phase": phase,
        "next_phase_hint": next_phase_hint,
        "started_at": step_result.started_at,
        "finished_at": step_result.finished_at,
    });
    let object = payload.as_object_mut()?;
    insert_optional_string(object, "action", signals.action.as_deref());
    insert_optional_string(object, "command", signals.command.as_deref());
    insert_optional_string(
        object,
        "verification_command",
        signals.verification_command.as_deref(),
    );
    insert_optional_string(
        object,
        "checkpoint_kind",
        signals.checkpoint_kind.as_deref(),
    );
    insert_optional_string(object, "checkpoint_ref", signals.checkpoint_ref.as_deref());
    insert_string_array(object, "planned_changes", &signals.planned_changes);
    insert_string_array(object, "diff_refs", &signals.diff_refs);
    insert_string_array(object, "changed_files", &signals.changed_files);
    insert_string_array(object, "files_read", &signals.files_read);
    insert_string_array(
        object,
        "completed_side_effect_refs",
        &signals.completed_side_effect_refs,
    );
    if status == "error" {
        object.insert(
            "failure_kind".to_string(),
            json!(coding_failure_kind(&signals)),
        );
    }
    Some(payload)
}

pub(super) fn coding_milestone_checkpoint_observation(
    transition: &Value,
    prior_observations: &[Value],
) -> Option<Value> {
    if transition.get("kind").and_then(Value::as_str) != Some("coding_state_transition") {
        return None;
    }
    let phase = transition.get("phase").and_then(Value::as_str)?;
    let checkpoint_kind = coding_checkpoint_kind(phase, transition, prior_observations)?;
    let step_id = transition.get("step_id").and_then(Value::as_str)?;
    let checkpoint_ref = format!("coding_checkpoint:{checkpoint_kind}:{step_id}");
    let mut payload = json!({
        "kind": "coding_checkpoint",
        "schema_version": 1,
        "checkpoint_kind": checkpoint_kind,
        "checkpoint_ref": checkpoint_ref,
        "evidence_ref": checkpoint_ref,
        "source_step_id": step_id,
        "phase": phase,
        "status": transition.get("status").cloned().unwrap_or(Value::Null),
        "next_phase_hint": transition.get("next_phase_hint").cloned().unwrap_or(Value::Null),
        "verification_status": coding_checkpoint_verification_status(phase, transition),
    });
    let object = payload.as_object_mut()?;
    copy_transition_field(object, transition, "action");
    copy_transition_field(object, transition, "command");
    copy_transition_field(object, transition, "verification_command");
    copy_transition_field(object, transition, "failure_kind");
    copy_transition_field(object, transition, "planned_changes");
    copy_transition_field(object, transition, "diff_refs");
    copy_transition_field(object, transition, "changed_files");
    copy_transition_field(object, transition, "files_read");
    copy_transition_field(object, transition, "completed_side_effect_refs");
    Some(payload)
}

#[derive(Default)]
struct CodingTransitionSignals {
    action: Option<String>,
    command: Option<String>,
    verification_command: Option<String>,
    planned_changes: Vec<String>,
    diff_refs: Vec<String>,
    changed_files: Vec<String>,
    files_read: Vec<String>,
    checkpoint_kind: Option<String>,
    checkpoint_ref: Option<String>,
    completed_side_effect_refs: Vec<String>,
    skill_signal: bool,
}

impl CodingTransitionSignals {
    fn has_coding_signal(&self) -> bool {
        self.skill_signal
            || self.command.is_some()
            || self.verification_command.is_some()
            || !self.planned_changes.is_empty()
            || !self.diff_refs.is_empty()
            || !self.changed_files.is_empty()
            || !self.files_read.is_empty()
            || self.checkpoint_kind.is_some()
            || self.checkpoint_ref.is_some()
            || !self.completed_side_effect_refs.is_empty()
    }
}

fn coding_checkpoint_kind(
    phase: &str,
    transition: &Value,
    prior_observations: &[Value],
) -> Option<&'static str> {
    match phase {
        "edit" => {
            if prior_observations
                .iter()
                .any(observation_is_repair_transition)
            {
                Some("fix_applied")
            } else {
                Some("file_edit_group")
            }
        }
        "verify" => Some("verification_command"),
        "repair" => Some("failed_step"),
        "checkpoint" => transition
            .get("checkpoint_kind")
            .and_then(Value::as_str)
            .and_then(|value| match value {
                "file_edit_group" => Some("file_edit_group"),
                "verification_command" => Some("verification_command"),
                "failed_step" => Some("failed_step"),
                "fix_applied" => Some("fix_applied"),
                _ => None,
            }),
        _ => None,
    }
}

fn observation_is_repair_transition(value: &Value) -> bool {
    value.get("kind").and_then(Value::as_str) == Some("coding_state_transition")
        && value.get("phase").and_then(Value::as_str) == Some("repair")
}

fn coding_checkpoint_verification_status(phase: &str, transition: &Value) -> &'static str {
    if transition.get("status").and_then(Value::as_str) == Some("error") {
        "failed"
    } else if phase == "verify" {
        "verified"
    } else if phase == "edit" {
        "unverified"
    } else {
        "not_applicable"
    }
}

fn copy_transition_field(map: &mut Map<String, Value>, transition: &Value, key: &str) {
    if let Some(value) = transition.get(key).filter(|value| !value.is_null()) {
        map.insert(key.to_string(), value.clone());
    }
}

fn collect_value_signals(value: &Value, signals: &mut CodingTransitionSignals) {
    match value {
        Value::Object(map) => {
            collect_map_signals(map, signals);
            if let Some(extra) = map.get("extra").and_then(Value::as_object) {
                collect_map_signals(extra, signals);
            }
            for child in map.values() {
                if child.is_object() || child.is_array() {
                    collect_value_signals(child, signals);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_value_signals(item, signals);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn collect_map_signals(map: &Map<String, Value>, signals: &mut CodingTransitionSignals) {
    if signals.action.is_none() {
        signals.action = string_field(map, "action");
    }
    if let Some(command) = string_field(map, "command") {
        collect_command(&command, signals);
    }
    if let Some(summary) = string_field(map, "sanitized_args_summary") {
        if let Some(command) = summary.strip_prefix("command=") {
            collect_command(command, signals);
        }
    }
    if signals.checkpoint_kind.is_none() {
        signals.checkpoint_kind = string_field(map, "checkpoint_kind");
    }
    if signals.checkpoint_ref.is_none() {
        signals.checkpoint_ref =
            string_field(map, "checkpoint_ref").or_else(|| string_field(map, "evidence_ref"));
    }
    collect_string_field(map, "planned_change", &mut signals.planned_changes);
    collect_string_list_field(map, "planned_changes", &mut signals.planned_changes);
    collect_string_field(map, "change_plan", &mut signals.planned_changes);
    collect_string_field(map, "diff_ref", &mut signals.diff_refs);
    collect_string_list_field(map, "diff_refs", &mut signals.diff_refs);
    collect_string_field(map, "patch_ref", &mut signals.diff_refs);
    collect_string_list_field(map, "patch_refs", &mut signals.diff_refs);
    collect_string_list_field(
        map,
        "completed_side_effect_refs",
        &mut signals.completed_side_effect_refs,
    );
    if map_action_is_mutating_file(signals.action.as_deref()) {
        collect_string_list_field(map, "changed_files", &mut signals.changed_files);
        collect_string_list_field(map, "files_changed", &mut signals.changed_files);
        collect_string_list_field(map, "paths", &mut signals.changed_files);
        collect_string_field(map, "path", &mut signals.changed_files);
        collect_string_field(map, "resolved_path", &mut signals.changed_files);
    } else if map_action_reads_file(signals.action.as_deref()) {
        collect_string_list_field(map, "paths", &mut signals.files_read);
        collect_string_field(map, "path", &mut signals.files_read);
        collect_string_field(map, "resolved_path", &mut signals.files_read);
    }
}

fn collect_machine_line_signals(text: &str, signals: &mut CodingTransitionSignals) {
    for line in text.lines().map(str::trim) {
        if let Some(command) = line.strip_prefix("command=") {
            collect_command(command, signals);
        } else if (line.starts_with("exit=") || line.starts_with("detached="))
            && line.contains(" command=")
        {
            if let Some((_, command)) = line.split_once(" command=") {
                collect_command(command, signals);
            }
        }
        if let Some(checkpoint_ref) = line.strip_prefix("checkpoint_ref=") {
            signals.checkpoint_ref = bounded_token(checkpoint_ref);
        }
        if let Some(diff_ref) = line.strip_prefix("diff_ref=") {
            if let Some(diff_ref) = bounded_token(diff_ref) {
                push_unique(&mut signals.diff_refs, diff_ref);
            }
        }
    }
}

fn collect_command(command: &str, signals: &mut CodingTransitionSignals) {
    let Some(command) = bounded_token(command) else {
        return;
    };
    if is_verification_command_token(&command) {
        signals.verification_command = Some(command.clone());
    }
    signals.command = Some(command);
}

fn action_from_value(value: &Value) -> Option<String> {
    let map = value.as_object()?;
    string_field(map, "action").or_else(|| {
        map.get("extra")
            .and_then(Value::as_object)
            .and_then(|extra| string_field(extra, "action"))
    })
}

fn string_field(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).and_then(bounded_token)
}

fn collect_string_field(map: &Map<String, Value>, key: &str, out: &mut Vec<String>) {
    if let Some(value) = string_field(map, key) {
        push_unique(out, value);
    }
}

fn collect_string_list_field(map: &Map<String, Value>, key: &str, out: &mut Vec<String>) {
    match map.get(key) {
        Some(Value::String(value)) => {
            if let Some(value) = bounded_token(value) {
                push_unique(out, value);
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(value) = item.as_str().and_then(bounded_token) {
                    push_unique(out, value);
                }
            }
        }
        _ => {}
    }
}

fn insert_optional_string(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        map.insert(key.to_string(), json!(value));
    }
}

fn insert_string_array(map: &mut Map<String, Value>, key: &str, values: &[String]) {
    if !values.is_empty() {
        map.insert(key.to_string(), json!(values));
    }
}

fn bounded_token(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.contains('\n') || value.contains('\r') || value.len() > 500 {
        return None;
    }
    Some(value.to_string())
}

fn push_unique(out: &mut Vec<String>, value: String) {
    if !out.iter().any(|existing| existing == &value) {
        out.push(value);
    }
}

fn should_collect_skill_phase(skill: &str) -> bool {
    matches!(skill, "run_cmd" | "git_basic")
}

fn coding_phase(
    step_result: &crate::executor::StepExecutionResult,
    signals: &CodingTransitionSignals,
) -> &'static str {
    if step_result.status == crate::executor::StepExecutionStatus::Error {
        return "repair";
    }
    if signals.checkpoint_kind.is_some() || signals.checkpoint_ref.is_some() {
        "checkpoint"
    } else if !signals.changed_files.is_empty() {
        "edit"
    } else if signals.verification_command.is_some() {
        "verify"
    } else if step_result.skill == "git_basic"
        || !signals.files_read.is_empty()
        || signals.command.is_some()
    {
        "inspect"
    } else {
        "plan"
    }
}

fn coding_next_phase_hint(
    status: &str,
    phase: &str,
    signals: &CodingTransitionSignals,
) -> &'static str {
    if status == "error" {
        "repair"
    } else if phase == "edit" && signals.verification_command.is_none() {
        "verify"
    } else if phase == "verify" {
        "summarize"
    } else if phase == "checkpoint" {
        "resume"
    } else {
        "continue"
    }
}

fn coding_failure_kind(signals: &CodingTransitionSignals) -> &'static str {
    if signals
        .verification_command
        .as_deref()
        .is_some_and(is_test_command_token)
    {
        "test"
    } else if signals.verification_command.is_some() {
        "verification"
    } else {
        "step"
    }
}

fn map_action_is_mutating_file(action: Option<&str>) -> bool {
    action.is_some_and(|action| {
        matches!(
            action,
            "write_text"
                | "append_text"
                | "replace_text"
                | "edit_file"
                | "create_file"
                | "delete_file"
                | "move_file"
                | "copy_file"
        )
    })
}

fn map_action_reads_file(action: Option<&str>) -> bool {
    action.is_some_and(|action| {
        matches!(
            action,
            "read" | "read_text" | "read_range" | "list" | "list_dir" | "find" | "search"
        )
    })
}

fn is_test_command_token(command: &str) -> bool {
    let command = command.trim().to_ascii_lowercase();
    command.starts_with("cargo test")
        || command.starts_with("npm test")
        || command.starts_with("npm run test")
        || command.starts_with("pnpm test")
        || command.starts_with("yarn test")
        || command.starts_with("pytest")
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
        || command.starts_with("ruff check")
        || command.starts_with("go vet")
}
