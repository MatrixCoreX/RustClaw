use serde_json::{json, Map, Value};

pub(super) fn task_outcome_report_json(raw_data: &Value, coding: &Value) -> Value {
    let outcome = task_outcome_root(raw_data);
    let evidence = evidence_coverage_root(raw_data);
    let mut done_conditions =
        collect_meta_list(outcome.and_then(|value| value.get("done_conditions")));
    done_conditions.extend(collect_meta_list(
        outcome.and_then(|value| value.get("acceptance")),
    ));
    let constraints = collect_meta_list(outcome.and_then(|value| value.get("constraints")));
    let mut verification = collect_meta_list(outcome.and_then(|value| value.get("verification")));
    append_coding_verification(coding, &mut verification);
    let mut current_progress = collect_meta_list(
        outcome
            .and_then(|value| value.get("current_progress"))
            .or_else(|| outcome.and_then(|value| value.get("progress"))),
    );
    append_coding_progress(coding, &mut current_progress);
    let mut remaining_work =
        collect_meta_list(outcome.and_then(|value| value.get("remaining_work")));
    append_remaining_work(coding, evidence, &mut remaining_work);

    json!({
        "schema_version": 1,
        "state": outcome.and_then(|value| string_field(value, "state")),
        "message_key": outcome.and_then(|value| string_field(value, "message_key")),
        "final_answer_shape": first_string_at(raw_data, &[
            "/result_json/task_journal/trace/contract_matrix/final_answer_shape",
            "/task_journal/trace/contract_matrix/final_answer_shape",
            "/result_json/task_journal/summary/finalizer_summary/final_answer_shape",
            "/task_journal/summary/finalizer_summary/final_answer_shape",
        ]),
        "done_condition_count": done_conditions.len(),
        "done_conditions": done_conditions,
        "constraint_count": constraints.len(),
        "constraints": constraints,
        "verification_count": verification.len(),
        "verification": verification,
        "current_progress_count": current_progress.len(),
        "current_progress": current_progress,
        "remaining_work_count": remaining_work.len(),
        "remaining_work": remaining_work,
    })
}

fn task_outcome_root(raw_data: &Value) -> Option<&Value> {
    first_object_at(
        raw_data,
        &[
            "/result_json/task_journal/summary/task_outcome",
            "/task_journal/summary/task_outcome",
            "/result_json/task_outcome",
            "/task_outcome",
        ],
    )
}

fn evidence_coverage_root(raw_data: &Value) -> Option<&Value> {
    first_object_at(
        raw_data,
        &[
            "/result_json/task_journal/trace/evidence_coverage",
            "/task_journal/trace/evidence_coverage",
        ],
    )
}

fn first_object_at<'a>(root: &'a Value, paths: &[&str]) -> Option<&'a Value> {
    paths
        .iter()
        .filter_map(|path| root.pointer(path))
        .find(|value| value.as_object().is_some())
}

fn first_string_at(root: &Value, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .filter_map(|path| root.pointer(path).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn string_field(root: &Value, key: &str) -> Option<String> {
    root.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn primitive_meta(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn collect_meta_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .flat_map(|item| {
                primitive_meta(item)
                    .into_iter()
                    .chain(object_meta(item.as_object()))
                    .collect::<Vec<_>>()
            })
            .collect(),
        Some(value) => primitive_meta(value)
            .into_iter()
            .chain(object_meta(value.as_object()))
            .collect(),
        None => Vec::new(),
    }
}

fn object_meta(map: Option<&Map<String, Value>>) -> Vec<String> {
    let Some(map) = map else {
        return Vec::new();
    };
    let mut keys = map.keys().collect::<Vec<_>>();
    keys.sort();
    keys.into_iter()
        .filter_map(|key| primitive_meta(&map[key]).map(|value| format!("{key}={value}")))
        .collect()
}

fn append_u64_field(root: &Value, pointer: &str, name: &str, out: &mut Vec<String>) {
    if let Some(value) = root.pointer(pointer).and_then(Value::as_u64) {
        out.push(format!("{name}={value}"));
    }
}

fn append_string_field(root: &Value, pointer: &str, name: &str, out: &mut Vec<String>) {
    if let Some(value) = root
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        out.push(format!("{name}={value}"));
    }
}

fn append_coding_verification(coding: &Value, out: &mut Vec<String>) {
    append_string_field(
        coding,
        "/state/verification_status",
        "verification_status",
        out,
    );
    append_u64_field(
        coding,
        "/verification_command_count",
        "verification_command_count",
        out,
    );
    append_u64_field(
        coding,
        "/verification_failure_kind_count",
        "verification_failure_kind_count",
        out,
    );
    append_string_field(coding, "/unverified_risk", "unverified_risk", out);
}

fn append_coding_progress(coding: &Value, out: &mut Vec<String>) {
    append_string_field(coding, "/state/current_phase_hint", "current_phase", out);
    append_u64_field(coding, "/changed_file_count", "changed_file_count", out);
    append_u64_field(coding, "/command_count", "command_count", out);
    append_u64_field(coding, "/test_count", "test_count", out);
    append_u64_field(
        coding,
        "/state/checkpoint_ref_count",
        "checkpoint_ref_count",
        out,
    );
    append_u64_field(
        coding,
        "/state/completed_side_effect_count",
        "completed_side_effect_count",
        out,
    );
}

fn append_remaining_work(coding: &Value, evidence: Option<&Value>, out: &mut Vec<String>) {
    append_string_field(coding, "/state/next_step", "next_step", out);
    if let Some(missing) = evidence.and_then(|value| value.get("missing_evidence")) {
        for item in collect_meta_list(Some(missing)) {
            out.push(format!("missing_evidence={item}"));
        }
    }
}
