use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentArtifactEntry {
    name: String,
    kind: String,
    path: String,
    size_bytes: Option<u64>,
    modified_ts: Option<i64>,
    extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentArtifactInventory {
    root: String,
    sort_by: String,
    entries: Vec<RecentArtifactEntry>,
}

pub(super) fn try_recover_recent_artifacts_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route_allows_recent_artifacts_recovery(route) {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap()
        || !verifier.missing_evidence_fields.iter().any(|field| {
            matches!(
                field.as_str(),
                "output_format" | "candidates" | "field_value"
            )
        })
    {
        return false;
    }
    let Some(mut inventory) = observed_recent_artifact_inventory(reply) else {
        return false;
    };
    apply_recent_artifact_route_selector(route, &mut inventory);
    if inventory.entries.is_empty() {
        return false;
    }
    let answer = deterministic_recent_artifact_fields(&inventory);
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer.clone();
    reply.messages = vec![answer];
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_recent_artifact_fields");
    true
}

fn route_allows_recent_artifacts_recovery(route: &crate::RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::RecentArtifactsJudgment
}

fn observed_recent_artifact_inventory(reply: &AskReply) -> Option<RecentArtifactInventory> {
    let journal = reply.task_journal.as_ref()?;
    journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .filter_map(recent_artifact_inventory_from_output)
        .next()
}

pub(super) fn recent_artifacts_inventory_observation_can_finalize(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    if !route_allows_recent_artifacts_recovery(route) {
        return false;
    }
    let Some(mut inventory) = observed_recent_artifact_inventory_from_loop_state(loop_state) else {
        return false;
    };
    apply_recent_artifact_route_selector(route, &mut inventory);
    !inventory.entries.is_empty()
}

fn observed_recent_artifact_inventory_from_loop_state(
    loop_state: &LoopState,
) -> Option<RecentArtifactInventory> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter(|step| matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .filter_map(recent_artifact_inventory_from_output)
        .next()
}

fn apply_recent_artifact_route_selector(
    route: &crate::RouteResult,
    inventory: &mut RecentArtifactInventory,
) {
    match recent_artifact_selector_target_kind(route) {
        crate::OutputScalarCountTargetKind::Any => {}
        crate::OutputScalarCountTargetKind::File => {
            inventory.entries.retain(|entry| entry.kind == "file");
        }
        crate::OutputScalarCountTargetKind::Dir => {
            inventory
                .entries
                .retain(|entry| matches!(entry.kind.as_str(), "dir" | "directory"));
        }
    }
    if let Some(limit) = recent_artifact_selector_limit(route) {
        inventory.entries.truncate(limit);
    }
}

fn recent_artifact_selector_target_kind(
    route: &crate::RouteResult,
) -> crate::OutputScalarCountTargetKind {
    let selector = &route.output_contract.self_extension.list_selector;
    if selector.target_kind_specified {
        return selector.target_kind;
    }
    selector_target_kind_machine_token(route.resolved_intent.as_str())
        .or_else(|| selector_target_kind_machine_token(route.route_reason.as_str()))
        .unwrap_or_default()
}

fn selector_target_kind_machine_token(text: &str) -> Option<crate::OutputScalarCountTargetKind> {
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | ')' | '('))
        .filter_map(|part| part.trim().strip_prefix("selector_target_kind="))
        .find_map(|raw| match raw.trim() {
            "file" => Some(crate::OutputScalarCountTargetKind::File),
            "dir" => Some(crate::OutputScalarCountTargetKind::Dir),
            "any" => Some(crate::OutputScalarCountTargetKind::Any),
            _ => None,
        })
}

fn recent_artifact_selector_limit(route: &crate::RouteResult) -> Option<usize> {
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .and_then(|limit| usize::try_from(limit).ok())
        .filter(|limit| *limit > 0)
        .or_else(|| selector_limit_machine_token(route.resolved_intent.as_str()))
        .or_else(|| selector_limit_machine_token(route.route_reason.as_str()))
}

fn selector_limit_machine_token(text: &str) -> Option<usize> {
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | ')' | '('))
        .filter_map(|part| part.trim().strip_prefix("selector_limit="))
        .filter_map(|raw| raw.trim().parse::<usize>().ok())
        .find(|limit| *limit > 0)
}

fn recent_artifact_inventory_from_output(output: &str) -> Option<RecentArtifactInventory> {
    let value = serde_json::from_str::<Value>(output).ok()?;
    recent_artifact_inventory_from_value(&value)
        .or_else(|| {
            value
                .get("extra")
                .and_then(recent_artifact_inventory_from_value)
        })
        .or_else(|| {
            value
                .get("text")
                .and_then(Value::as_str)
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .and_then(|inner| recent_artifact_inventory_from_value(&inner))
        })
}

fn recent_artifact_inventory_from_value(payload: &Value) -> Option<RecentArtifactInventory> {
    if payload.get("action").and_then(Value::as_str) != Some("inventory_dir") {
        return None;
    }
    let entries = payload
        .get("entries")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(recent_artifact_entry_from_value)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    Some(RecentArtifactInventory {
        root: payload
            .get("path")
            .or_else(|| payload.get("resolved_path"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
        sort_by: payload
            .get("sort_by")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
        entries,
    })
}

fn recent_artifact_entry_from_value(value: &Value) -> Option<RecentArtifactEntry> {
    let name = value.get("name").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    Some(RecentArtifactEntry {
        name: name.to_string(),
        kind,
        path,
        size_bytes: value.get("size_bytes").and_then(Value::as_u64),
        modified_ts: value.get("modified_ts").and_then(Value::as_i64),
        extension: extension_token(name),
    })
}

fn extension_token(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .map(|extension| extension.to_ascii_lowercase())
}

fn deterministic_recent_artifact_fields(inventory: &RecentArtifactInventory) -> String {
    let mut lines = Vec::new();
    let archive_count = inventory
        .entries
        .iter()
        .filter(|entry| entry.extension.as_deref().is_some_and(is_archive_extension))
        .count();
    let config_count = inventory
        .entries
        .iter()
        .filter(|entry| entry.extension.as_deref().is_some_and(is_config_extension))
        .count();
    let dir_count = inventory
        .entries
        .iter()
        .filter(|entry| entry.kind == "dir")
        .count();
    let tmp_context = path_has_segment(&inventory.root, &["tmp", "temp"]);

    lines.push(format!("recent_entries.count={}", inventory.entries.len()));
    if !inventory.root.is_empty() {
        lines.push(format!("recent_entries.root={}", inventory.root));
    }
    if !inventory.sort_by.is_empty() {
        lines.push(format!("recent_entries.sort_by={}", inventory.sort_by));
    }
    for (index, entry) in inventory.entries.iter().enumerate() {
        lines.push(format!("recent_entries[{index}].name={}", entry.name));
        if !entry.kind.is_empty() {
            lines.push(format!("recent_entries[{index}].kind={}", entry.kind));
        }
        if !entry.path.is_empty() {
            lines.push(format!("recent_entries[{index}].path={}", entry.path));
        }
        if let Some(size_bytes) = entry.size_bytes {
            lines.push(format!("recent_entries[{index}].size_bytes={size_bytes}"));
        }
        if let Some(modified_ts) = entry.modified_ts {
            lines.push(format!("recent_entries[{index}].modified_ts={modified_ts}"));
        }
        if let Some(extension) = entry.extension.as_deref() {
            lines.push(format!("recent_entries[{index}].extension={extension}"));
        }
    }
    lines.push(format!("classification.archive_count={archive_count}"));
    lines.push(format!("classification.config_count={config_count}"));
    lines.push(format!("classification.dir_count={dir_count}"));
    lines.push(format!("classification.path_context_tmp={tmp_context}"));
    lines.push(format!(
        "classification.temporary_bundle_artifact={}",
        tmp_context && (archive_count > 0 || dir_count > 0)
    ));
    lines.push(format!(
        "classification.formal_config={}",
        config_count > 0 && archive_count == 0 && !tmp_context
    ));
    lines.join("\n")
}

fn is_archive_extension(extension: &str) -> bool {
    matches!(
        extension,
        "zip" | "tar" | "tgz" | "gz" | "bz2" | "xz" | "7z"
    )
}

fn is_config_extension(extension: &str) -> bool {
    matches!(
        extension,
        "toml" | "yaml" | "yml" | "json" | "env" | "ini" | "conf"
    )
}

fn path_has_segment(path: &str, allowed_segments: &[&str]) -> bool {
    path.split(['/', '\\'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .any(|segment| {
            let segment = segment.to_ascii_lowercase();
            allowed_segments.iter().any(|allowed| segment == *allowed)
        })
}
