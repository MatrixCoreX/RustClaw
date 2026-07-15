use serde_json::{json, Value};

use super::{register_file_path_output, AppState, LoopState};

pub(super) fn skill_extra_requests_user_input(extra: Option<&Value>) -> bool {
    let Some(obj) = extra.and_then(Value::as_object) else {
        return false;
    };
    obj.get("requires_user_input")
        .or_else(|| obj.get("needs_user_input"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn matrix_admitted_external_evidence_output(
    state: &AppState,
    normalized_skill: &str,
    action_args: &Value,
    out: &str,
    structured_extra: Option<&Value>,
) -> Option<String> {
    let extra = structured_extra?;
    let registry = state.get_skills_registry()?;
    let canonical = registry.resolve_canonical(normalized_skill)?;
    let entry = registry.get(canonical)?;
    let admission = entry.matrix_admission.as_ref()?;
    let requires_admission = entry.matrix_admission.is_some()
        || entry.kind == claw_core::skill_registry::SkillKind::External
        || entry
            .external_bundle_dir
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    if !requires_admission || !admission.eligible {
        return None;
    }
    let action = extra
        .get("action")
        .and_then(Value::as_str)
        .or_else(|| action_args.get("action").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if !registry.matrix_admission_eligible(canonical, action) {
        return None;
    }
    let extractor_kind = admission
        .extractor_kind
        .as_deref()
        .map(normalize_machine_token)
        .unwrap_or_else(|| "structured_json".to_string());
    if extractor_kind != "structured_json" {
        return None;
    }
    if !admission
        .required_extra_fields
        .iter()
        .all(|field| admitted_extra_field_exists(extra, field))
    {
        return None;
    }
    let mut payload = serde_json::Map::new();
    if let Some(action) = action {
        payload.insert("action".to_string(), json!(action));
    }
    payload.insert("text".to_string(), json!(out));
    payload.insert("extra".to_string(), extra.clone());
    payload.insert(
        "_matrix_admission".to_string(),
        json!({
            "schema_version": 1,
            "source": "skills_registry",
            "skill": canonical,
            "eligible": true,
            "extractor_kind": extractor_kind,
            "declared_actions": &admission.declared_actions,
            "evidence_sources": &admission.evidence_sources,
            "required_extra_fields": &admission.required_extra_fields,
            "admission_version": admission.admission_version.as_deref(),
        }),
    );
    Some(Value::Object(payload).to_string())
}

pub(super) fn structured_extra_evidence_output(
    out: &str,
    structured_extra: Option<&Value>,
) -> Option<String> {
    let extra = structured_extra?;
    if extra.is_null() {
        return None;
    }
    Some(
        json!({
            "text": out,
            "extra": extra,
        })
        .to_string(),
    )
}

pub(super) fn merge_isolation_artifact_refs(
    evidence_output: Option<String>,
    out: &str,
    artifact_refs: &[Value],
) -> Option<String> {
    if artifact_refs.is_empty() {
        return evidence_output;
    }
    let mut value = evidence_output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output.trim()).ok())
        .unwrap_or_else(|| json!({ "text": out }));
    let Some(obj) = value.as_object_mut() else {
        return Some(
            json!({
                "text": out,
                "artifacts": artifact_refs,
                "artifact_refs": artifact_refs,
            })
            .to_string(),
        );
    };
    append_json_array(obj, "artifacts", artifact_refs);
    append_json_array(obj, "artifact_refs", artifact_refs);
    Some(value.to_string())
}

fn append_json_array(map: &mut serde_json::Map<String, Value>, key: &str, items: &[Value]) {
    if items.is_empty() {
        return;
    }
    let entry = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(array) = entry.as_array_mut() {
        array.extend(items.iter().cloned());
    }
}

pub(super) fn register_structured_extra_file_path_outputs(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    normalized_skill: &str,
    structured_extra: Option<&Value>,
) {
    let Some(extra) = structured_extra.and_then(Value::as_object) else {
        return;
    };
    let mut paths = Vec::new();
    if let Some(path) = extra
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        paths.push(path.to_string());
    }
    if let Some(outputs) = extra.get("outputs").and_then(Value::as_array) {
        for item in outputs {
            let Some(path) = item
                .get("path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !paths.iter().any(|existing| existing == path) {
                paths.push(path.to_string());
            }
        }
    }
    for path in paths {
        let mut source = String::from("skill");
        source.push('.');
        source.push_str(normalized_skill);
        source.push('.');
        source.push_str("extra");
        register_file_path_output(loop_state, global_step, step_in_round, &source, &path);
    }
}

pub(super) fn admitted_extra_field_exists(extra: &Value, field: &str) -> bool {
    let mut field = field.trim();
    if field.is_empty() {
        return false;
    }
    field = field.strip_prefix("extra.").unwrap_or(field);
    field = field.strip_prefix("extra/").unwrap_or(field);
    field = field.trim_matches('.');
    if field.is_empty() || field == "extra" {
        return true;
    }
    let mut current = extra;
    for segment in field.split('.') {
        let segment = segment.trim();
        if segment.is_empty() {
            return false;
        }
        let Some(next) = current.get(segment) else {
            return false;
        };
        current = next;
    }
    !current.is_null()
}

fn normalize_machine_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}
