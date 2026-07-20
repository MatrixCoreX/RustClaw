use super::*;

pub(super) fn latest_path_batch_fact_answer_for_requested_summary(
    journal: &crate::task_journal::TaskJournal,
    requested_summary: &str,
) -> Option<String> {
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .find_map(|output| {
            let fact = path_batch_fact_answer_from_output(output)?;
            requested_summary_refs_path_fact(requested_summary, &fact.path)
                .then(|| fact.into_machine_answer())
        })
}

struct PathBatchFactAnswer {
    path: String,
    exists: bool,
    kind: Option<String>,
    size_bytes: Option<u64>,
}

impl PathBatchFactAnswer {
    fn into_machine_answer(self) -> String {
        let mut lines = vec![
            "message_key=clawd.msg.path_fact.observed".to_string(),
            "reason_code=path_fact_observed".to_string(),
            format!("exists={}", self.exists),
            format!("path={}", sanitize_machine_line_value(&self.path)),
        ];
        let kind = self
            .kind
            .filter(|kind| !kind.is_empty())
            .or_else(|| (!self.exists).then(|| "missing".to_string()));
        if let Some(kind) = kind {
            lines.push(format!("kind={kind}"));
        }
        if !self.exists {
            lines.push("error_code=path_not_found".to_string());
        }
        if let Some(size_bytes) = self.size_bytes {
            lines.push(format!("size_bytes={size_bytes}"));
        }
        lines.push("source_action=path_batch_facts".to_string());
        lines.join("\n")
    }
}

fn path_batch_fact_answer_from_output(output: &str) -> Option<PathBatchFactAnswer> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    let value = path_batch_facts_payload(&value)?;
    let facts = value.get("facts").and_then(serde_json::Value::as_array)?;
    if facts.len() != 1 {
        return None;
    }
    let entry = facts.first()?.as_object()?;
    let exists = entry
        .get("exists")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let fact = entry.get("fact").and_then(serde_json::Value::as_object);
    let path = fact
        .and_then(|fact| fact.get("resolved_path"))
        .or_else(|| fact.and_then(|fact| fact.get("path")))
        .or_else(|| entry.get("resolved_path"))
        .or_else(|| entry.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?
        .to_string();
    let kind = fact
        .and_then(|fact| fact.get("kind"))
        .or_else(|| entry.get("kind"))
        .and_then(serde_json::Value::as_str)
        .map(normalized_path_kind_token);
    let size_bytes = fact
        .and_then(|fact| fact.get("size_bytes"))
        .or_else(|| entry.get("size_bytes"))
        .and_then(serde_json::Value::as_u64);
    Some(PathBatchFactAnswer {
        path,
        exists,
        kind,
        size_bytes,
    })
}

fn path_batch_facts_payload(value: &serde_json::Value) -> Option<&serde_json::Value> {
    if value.get("action").and_then(serde_json::Value::as_str) == Some("path_batch_facts") {
        return Some(value);
    }
    let extra = value.get("extra")?;
    (extra.get("action").and_then(serde_json::Value::as_str) == Some("path_batch_facts"))
        .then_some(extra)
}

fn requested_summary_refs_path_fact(requested_summary: &str, path: &str) -> bool {
    let requested_summary = requested_summary.trim();
    if requested_summary.is_empty() {
        return false;
    }
    bare_machine_markers(requested_summary)
        .iter()
        .any(|marker| path_matches_requested_token(path, marker))
        || requested_machine_summary_pairs(requested_summary)
            .iter()
            .any(|(key, value)| {
                matches!(key.as_str(), "path" | "resolved_path")
                    && path_matches_requested_token(path, value)
            })
}

fn path_matches_requested_token(path: &str, token: &str) -> bool {
    let token = token.trim().trim_matches('`');
    let path = path.trim();
    !token.is_empty()
        && !path.is_empty()
        && (path == token
            || path.rsplit('/').next().is_some_and(|name| name == token)
            || path.ends_with(&format!("/{token}")))
}

fn normalized_path_kind_token(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "dir" | "directory" => "dir".to_string(),
        "file" => "file".to_string(),
        "symlink" | "link" => "symlink".to_string(),
        "missing" | "not_found" | "not found" => "missing".to_string(),
        "other" => "other".to_string(),
        "unknown" => "unknown".to_string(),
        value => value.to_string(),
    }
}

fn sanitize_machine_line_value(value: &str) -> String {
    crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(value).replace('\n', " "),
    )
}

pub(super) fn requested_machine_kv_summary_from_task_final_answer_with_surfaces(
    request_surfaces: &[String],
    _route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> Option<String> {
    let mut observed_texts =
        crate::machine_kv_projection::observed_machine_text_fragments_from_journal(journal);
    crate::machine_kv_projection::collect_machine_text_fragments_from_output(
        answer_text,
        &mut observed_texts,
    );
    for message in answer_messages {
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            message,
            &mut observed_texts,
        );
    }
    observed_texts.sort();
    observed_texts.dedup();
    crate::machine_kv_projection::requested_machine_kv_summary_from_observation_inputs(
        request_surfaces.iter().map(String::as_str),
        &observed_texts,
    )
}

pub(super) fn request_surfaces_explicitly_request_kv_summary(
    request_surfaces: &[String],
    requested_summary: &str,
) -> bool {
    let summary_units = machine_kv_units(requested_summary);
    !summary_units.is_empty()
        && request_surfaces.iter().any(|surface| {
            let surface_units = machine_kv_units(surface);
            !surface_units.is_empty()
                && summary_units.iter().any(|unit| {
                    surface_units
                        .iter()
                        .any(|surface_unit| surface_unit == unit)
                })
        })
}

pub(super) fn task_machine_kv_request_surfaces(
    prompt: &str,
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    let mut surfaces = Vec::new();
    for value in [
        Some(prompt),
        Some(journal.input_text.as_str()),
        route_result.selection.structured_field_selector.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, value);
    }
    if let Some(state_patch) = journal
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
    {
        crate::machine_kv_projection::collect_requested_machine_kv_surfaces_from_state_patch(
            state_patch,
            &mut surfaces,
        );
    }
    surfaces
}

pub(super) fn final_answer_preserves_compare_paths_existence_fields(
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    text_has_compare_paths_existence_fields(answer_text)
        || answer_messages
            .iter()
            .any(|message| text_has_compare_paths_existence_fields(message))
}

pub(super) fn text_has_compare_paths_existence_fields(text: &str) -> bool {
    let mut has_same_path = false;
    let mut has_left_exists = false;
    let mut has_right_exists = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("same_path=") {
            has_same_path = true;
        } else if line.starts_with("left_exists=") {
            has_left_exists = true;
        } else if line.starts_with("right_exists=") {
            has_right_exists = true;
        }
    }
    has_same_path && has_left_exists && has_right_exists
}
