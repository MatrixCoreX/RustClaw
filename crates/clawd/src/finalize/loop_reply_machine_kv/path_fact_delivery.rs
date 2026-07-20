use serde_json::Value;

use crate::agent_engine::{AgentRunContext, LoopState};

pub(super) fn latest_path_batch_fact_delivery_for_requested_summary(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    requested_summary: &str,
) -> Option<String> {
    if route_requests_scalar_path_only(agent_run_context) {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find_map(|output| {
            let fact = path_batch_fact_delivery_from_output(output)?;
            requested_summary_refs_path_fact(requested_summary, &fact.path, agent_run_context)
                .then(|| fact.into_machine_answer())
        })
}

fn route_requests_scalar_path_only(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(|route| {
            route.response_shape == crate::OutputResponseShape::Scalar
                && crate::finalize::route_matches_single_path_output_contract(route)
        })
}

struct PathBatchFactDelivery {
    path: String,
    exists: bool,
    kind: Option<String>,
    size_bytes: Option<u64>,
}

impl PathBatchFactDelivery {
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

fn path_batch_fact_delivery_from_output(output: &str) -> Option<PathBatchFactDelivery> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    let value = path_batch_facts_payload(&value)?;
    let facts = value.get("facts").and_then(Value::as_array)?;
    if facts.len() != 1 {
        return None;
    }
    let entry = facts.first()?.as_object()?;
    let exists = entry
        .get("exists")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fact = entry.get("fact").and_then(Value::as_object);
    let path = fact
        .and_then(|fact| fact.get("resolved_path"))
        .or_else(|| fact.and_then(|fact| fact.get("path")))
        .or_else(|| entry.get("resolved_path"))
        .or_else(|| entry.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?
        .to_string();
    let kind = fact
        .and_then(|fact| fact.get("kind"))
        .or_else(|| entry.get("kind"))
        .and_then(Value::as_str)
        .map(normalized_path_kind_token);
    let size_bytes = fact
        .and_then(|fact| fact.get("size_bytes"))
        .or_else(|| entry.get("size_bytes"))
        .and_then(Value::as_u64);
    Some(PathBatchFactDelivery {
        path,
        exists,
        kind,
        size_bytes,
    })
}

fn path_batch_facts_payload(value: &Value) -> Option<&Value> {
    if value.get("action").and_then(Value::as_str) == Some("path_batch_facts") {
        return Some(value);
    }
    let extra = value.get("extra")?;
    (extra.get("action").and_then(Value::as_str) == Some("path_batch_facts")).then_some(extra)
}

fn requested_summary_refs_path_fact(
    requested_summary: &str,
    path: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let requested_summary = requested_summary.trim();
    if requested_summary.is_empty() {
        return false;
    }
    if route_requests_existence_with_path(agent_run_context) {
        return true;
    }
    super::bare_machine_markers(requested_summary)
        .iter()
        .any(|marker| path_matches_requested_token(path, marker))
        || super::requested_machine_summary_pairs(requested_summary)
            .iter()
            .any(|(key, value)| {
                matches!(key.as_str(), "path" | "resolved_path")
                    && path_matches_requested_token(path, value)
            })
}

fn route_requests_existence_with_path(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(|route| route.semantic_kind_is(crate::OutputSemanticKind::ExistenceWithPath))
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
