use crate::agent_engine::{AgentRunContext, LoopState};

pub(super) fn inventory_ranked_size_list_answer(
    body: &str,
    route: &crate::IntentOutputContract,
) -> Option<String> {
    if route.response_shape != crate::OutputResponseShape::Strict {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(serde_json::Value::as_str) != Some("inventory_dir") {
        return None;
    }
    let sort_by = value.get("sort_by").and_then(serde_json::Value::as_str)?;
    if !matches!(sort_by, "size_desc" | "size_asc") {
        return None;
    }
    let mut entries = value
        .get("entries")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter(|entry| {
            entry
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|kind| kind == "file")
        })
        .filter_map(|entry| {
            let name = entry
                .get("name")
                .or_else(|| entry.get("path"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())?;
            let size_bytes = entry
                .get("size_bytes")
                .and_then(serde_json::Value::as_u64)?;
            Some((name.to_string(), size_bytes))
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    if sort_by == "size_desc" {
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    } else {
        entries.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    }
    if let Some(limit) = route
        .selection
        .list_selector
        .limit
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
    {
        entries.truncate(limit.min(entries.len()));
    }
    Some(
        entries
            .into_iter()
            .map(|(name, size_bytes)| format!("{name} {size_bytes}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

pub(super) fn direct_compare_paths_required_metadata_from_observed_output(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(AgentRunContext::output_contract)?;
    if !route_allows_compare_paths_required_metadata_projection(route) {
        return None;
    }
    let answer = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(|output| {
            let output =
                crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                    output,
                );
            compare_paths_metadata_answer(&output)
        })?;
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: loop_state.executed_step_results.len(),
            ..Default::default()
        },
    ))
}

fn route_allows_compare_paths_required_metadata_projection(
    route: &crate::IntentOutputContract,
) -> bool {
    !route.delivery_required
        && route.requires_content_evidence
        && crate::evidence_policy::required_evidence_fields_for_output_contract(route)
            .iter()
            .any(|field| matches!(field.as_str(), "exists" | "kind"))
}

fn compare_paths_metadata_answer(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(serde_json::Value::as_str) != Some("compare_paths") {
        return None;
    }
    let field_value = value.get("field_value").filter(|value| value.is_object());
    let same_path = field_value
        .and_then(|item| item.get("same_path"))
        .or_else(|| {
            value
                .get("comparison")
                .and_then(|item| item.get("same_path"))
        })
        .and_then(serde_json::Value::as_bool)?;
    let left = value.get("left")?;
    let right = value.get("right")?;
    let left_exists = field_value
        .and_then(|item| item.get("left_exists"))
        .or_else(|| left.get("exists"))
        .and_then(serde_json::Value::as_bool)?;
    let right_exists = field_value
        .and_then(|item| item.get("right_exists"))
        .or_else(|| right.get("exists"))
        .and_then(serde_json::Value::as_bool)?;
    let left_kind = path_kind_or_default(left);
    let right_kind = path_kind_or_default(right);
    Some(format!(
        "same_path={same_path}\nleft_exists={left_exists}\nleft_kind={left_kind}\nright_exists={right_exists}\nright_kind={right_kind}"
    ))
}

fn path_kind_or_default(value: &serde_json::Value) -> &str {
    value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("-")
}

#[cfg(test)]
#[path = "loop_reply_machine_projections_tests.rs"]
mod tests;
