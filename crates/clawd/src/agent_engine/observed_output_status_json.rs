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
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<String> {
    if route.delivery_required
        || matches!(
            route.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::FileToken
        )
        || super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::RawCommandOutput,
        )
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
        let body = normalized_success_body_for_observed_output(output);
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
