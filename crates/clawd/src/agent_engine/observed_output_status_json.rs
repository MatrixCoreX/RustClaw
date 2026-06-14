use super::*;

#[derive(Clone)]
struct StatusJsonObservation {
    label: String,
    path: String,
    status: String,
    healthy: Option<bool>,
    last_error: Option<String>,
}

pub(super) fn multi_status_json_summary_candidate(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<String> {
    if route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::FileToken
        )
        || route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
    {
        return None;
    }
    let observations = status_json_observations(loop_state);
    if observations.len() < 2 {
        return None;
    }
    let mut lines = vec![format!("status_files.count={}", observations.len())];
    lines.extend(
        observations
            .iter()
            .enumerate()
            .map(|(index, item)| status_json_observation_line(index, item)),
    );
    let notable = observations
        .iter()
        .find(|item| status_json_observation_is_notable(item))
        .or_else(|| observations.first())?;
    lines.push(format!("status_files.notable.label={}", notable.label));
    lines.push(format!("status_files.notable.status={}", notable.status));
    Some(lines.join("\n"))
}

fn status_json_observations(loop_state: &LoopState) -> Vec<StatusJsonObservation> {
    let mut observations = Vec::new();
    let mut seen_paths = std::collections::BTreeSet::new();
    for step in &loop_state.executed_step_results {
        if !step.is_ok() || !matches!(step.skill.as_str(), "fs_basic" | "system_basic") {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        let body = normalized_success_body_for_direct_answer(output);
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        let Some(observation) = status_json_observation_from_read_range(&value) else {
            continue;
        };
        if seen_paths.insert(observation.path.clone()) {
            observations.push(observation);
        }
    }
    observations
}

fn status_json_observation_from_read_range(
    value: &serde_json::Value,
) -> Option<StatusJsonObservation> {
    if value.get("action").and_then(|value| value.as_str()) != Some("read_range") {
        return None;
    }
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let excerpt = value
        .get("excerpt")
        .or_else(|| value.get("content"))
        .and_then(|value| value.as_str())?;
    let normalized = normalize_read_range_excerpt(excerpt)?;
    let status = serde_json::from_str::<serde_json::Value>(normalized.trim()).ok()?;
    let status_text = status
        .get("status")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    Some(StatusJsonObservation {
        label: status_json_label(&path, &status),
        path,
        status: status_text,
        healthy: status.get("healthy").and_then(|value| value.as_bool()),
        last_error: status
            .get("last_error")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
    })
}

fn status_json_label(path: &str, status: &serde_json::Value) -> String {
    let relative_path = status_relative_path_label(path);
    let identity = status
        .get("scope")
        .or_else(|| status.get("kind"))
        .or_else(|| status.get("name"))
        .or_else(|| status.get("account_label"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match identity {
        Some(identity) => format!("{relative_path} ({identity})"),
        None => relative_path,
    }
}

fn status_relative_path_label(path: &str) -> String {
    let normalized = path.trim().replace('\\', "/");
    let parts = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() >= 2 {
        return format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]);
    }
    normalized
}

fn status_json_observation_line(index: usize, item: &StatusJsonObservation) -> String {
    let healthy = item
        .healthy
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let mut line = format!(
        "status_files.item.{index}=label={}; healthy={healthy}; status={}; path={}",
        item.label, item.status, item.path
    );
    if let Some(last_error) = item.last_error.as_deref() {
        line.push_str(&format!(", last_error={last_error}"));
    }
    line
}

fn status_json_observation_is_notable(item: &StatusJsonObservation) -> bool {
    if item.healthy == Some(false) || item.last_error.is_some() {
        return true;
    }
    let status = item.status.trim().to_ascii_lowercase();
    !matches!(
        status.as_str(),
        "ok" | "running" | "active" | "healthy" | "ready" | "connected"
    )
}

pub(super) fn latest_find_ext_results(
    loop_state: &LoopState,
) -> Option<(Vec<String>, usize, String)> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "fs_basic" | "fs_search"))
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| {
            let body = normalized_success_body_for_direct_answer(output);
            serde_json::from_str::<serde_json::Value>(&body).ok()
        })
        .find_map(|value| fs_search_find_ext_results(&value))
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FindExtRepresentativeKind {
    Root,
    Config,
    Channel,
    Locale,
    Other,
}

fn find_ext_representative_kind(path: &str) -> FindExtRepresentativeKind {
    let normalized = path.trim().replace('\\', "/");
    if !normalized.contains('/') {
        return FindExtRepresentativeKind::Root;
    }
    if normalized.starts_with("configs/channels/") {
        return FindExtRepresentativeKind::Channel;
    }
    if normalized.starts_with("configs/i18n/") {
        return FindExtRepresentativeKind::Locale;
    }
    if normalized.starts_with("configs/") {
        return FindExtRepresentativeKind::Config;
    }
    FindExtRepresentativeKind::Other
}

pub(super) fn find_ext_representative_lines(results: &[String]) -> Vec<String> {
    let mut selected = Vec::<(String, FindExtRepresentativeKind)>::new();
    let mut seen_kinds = std::collections::BTreeSet::new();
    for path in results {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        let kind = find_ext_representative_kind(path);
        if seen_kinds.insert(kind) {
            selected.push((path.to_string(), kind));
        }
        if selected.len() >= 5 {
            break;
        }
    }
    for path in results {
        if selected.len() >= 5 {
            break;
        }
        let path = path.trim();
        if path.is_empty()
            || selected
                .iter()
                .any(|(selected_path, _)| selected_path == path)
        {
            continue;
        }
        selected.push((path.to_string(), find_ext_representative_kind(path)));
    }
    selected
        .into_iter()
        .map(|(path, kind)| {
            format!(
                "find_ext.representative.path={path}; kind={}",
                find_ext_representative_kind_label(kind)
            )
        })
        .collect()
}

fn find_ext_representative_kind_label(kind: FindExtRepresentativeKind) -> &'static str {
    match kind {
        FindExtRepresentativeKind::Root => "root",
        FindExtRepresentativeKind::Config => "config",
        FindExtRepresentativeKind::Channel => "channel",
        FindExtRepresentativeKind::Locale => "locale",
        FindExtRepresentativeKind::Other => "other",
    }
}
