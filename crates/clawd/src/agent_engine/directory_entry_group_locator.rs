use super::*;

#[cfg(test)]
pub(super) fn directory_purpose_extension_locator(route: &RouteResult) -> Option<String> {
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || !route.output_contract_marker_is(crate::OutputSemanticKind::DirectoryPurposeSummary)
        || route_requests_extension_assess_gap(route)
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace | crate::OutputLocatorKind::Path
        )
    {
        return None;
    }
    extension_from_globish_pattern(route.output_contract.locator_hint.trim())
        .or_else(|| structural_extension_filter_from_text(&route.resolved_intent))
}

#[cfg(test)]
fn route_requests_extension_assess_gap(route: &RouteResult) -> bool {
    route_has_machine_token(route, "extension.assess_gap")
        || (route_has_machine_token(route, "extension_manager")
            && route_has_machine_token(route, "assess_gap"))
}

#[cfg(test)]
fn route_has_machine_token(route: &RouteResult, token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    [route.resolved_intent.as_str(), route.route_reason.as_str()]
        .into_iter()
        .any(|text| machine_token_present(text, token))
}

#[cfg(test)]
fn machine_token_present(text: &str, token: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
        .any(|part| part == token || part.starts_with(&format!("{token}.")))
}

pub(super) fn step_output_action(value: &Value) -> Option<String> {
    let payload = value
        .get("extra")
        .filter(|extra| extra.is_object())
        .unwrap_or(value);
    payload
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
        .map(|action| action.to_ascii_lowercase())
}

pub(super) fn executed_step_is_successful_text_read(
    step: &crate::executor::StepExecutionResult,
) -> bool {
    if !step.is_ok() {
        return false;
    }
    if step.skill.eq_ignore_ascii_case("read_file") || step.skill.eq_ignore_ascii_case("doc_parse")
    {
        return step
            .output
            .as_deref()
            .map(str::trim)
            .is_some_and(|output| !output.is_empty());
    }
    if !(step.skill.eq_ignore_ascii_case("fs_basic")
        || step.skill.eq_ignore_ascii_case("system_basic"))
    {
        return false;
    }
    step.output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
        .and_then(|value| step_output_action(&value))
        .is_some_and(|action| action == "read_text_range" || action == "read_range")
}

#[cfg(test)]
pub(super) fn executed_find_entries_candidate_paths(
    step: &crate::executor::StepExecutionResult,
) -> Vec<String> {
    if !step.is_ok()
        || !(step.skill.eq_ignore_ascii_case("fs_basic")
            || step.skill.eq_ignore_ascii_case("fs_search"))
    {
        return Vec::new();
    }
    let Some(value) = step
        .output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
    else {
        return Vec::new();
    };
    let Some(action) = step_output_action(&value) else {
        return Vec::new();
    };
    if !matches!(action.as_str(), "find_entries" | "find_ext" | "find_name") {
        return Vec::new();
    }
    let payload = value
        .get("extra")
        .filter(|extra| extra.is_object())
        .unwrap_or(&value);
    payload
        .get("results")
        .or_else(|| payload.get("candidates"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
pub(super) fn safe_representative_find_result_paths(
    root: &str,
    candidates: Vec<String>,
) -> Vec<String> {
    let root_path = Path::new(root);
    let canonical_root = root_path
        .canonicalize()
        .unwrap_or_else(|_| root_path.to_path_buf());
    let mut selected = Vec::new();
    for candidate in candidates {
        if selected.len() >= 3 {
            break;
        }
        if candidate.contains('\0') {
            continue;
        }
        let raw = Path::new(&candidate);
        if raw.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        }) {
            continue;
        }
        let full_path = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            root_path.join(raw)
        };
        let canonical_candidate = full_path
            .canonicalize()
            .unwrap_or_else(|_| full_path.clone());
        if !canonical_candidate.starts_with(&canonical_root) || !canonical_candidate.is_file() {
            continue;
        }
        let read_path = canonical_candidate.display().to_string();
        if !selected.iter().any(|existing| existing == &read_path) {
            selected.push(read_path);
        }
    }
    selected
}

#[cfg(test)]
pub(super) fn directory_purpose_representative_reads_after_find_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if !loop_state.has_tool_or_skill_output
        || directory_purpose_extension_locator(route).is_none()
        || loop_state
            .executed_step_results
            .iter()
            .any(executed_step_is_successful_text_read)
    {
        return None;
    }
    let root = route_directory_locator_path(route, auto_locator_path)?;
    let candidates = loop_state
        .executed_step_results
        .iter()
        .rev()
        .flat_map(executed_find_entries_candidate_paths)
        .collect::<Vec<_>>();
    let selected = safe_representative_find_result_paths(&root, candidates);
    if selected.is_empty() {
        return None;
    }
    let mut actions = selected
        .into_iter()
        .map(|path| AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 60,
            }),
        })
        .collect::<Vec<_>>();
    let evidence_refs = (1..=actions.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    actions.push(AgentAction::SynthesizeAnswer { evidence_refs });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}
