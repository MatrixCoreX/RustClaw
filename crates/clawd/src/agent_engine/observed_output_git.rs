use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitStatusEntry {
    pub(crate) status: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitRepositoryStateObservation {
    pub(crate) branch: Option<String>,
    pub(crate) dirty: bool,
    pub(crate) changed_entries: Vec<GitStatusEntry>,
}

pub(crate) fn git_repository_state_observation_from_status_output(
    body: &str,
    branch_hint: Option<&str>,
) -> Option<GitRepositoryStateObservation> {
    let mut branch = branch_hint
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(ToOwned::to_owned);
    let mut saw_status_header = false;
    let mut changed_entries = Vec::new();
    for raw_line in body.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("exit=") {
            continue;
        }
        if trimmed.starts_with("## ") {
            saw_status_header = true;
            if branch.is_none() {
                branch = git_current_branch_from_status_header(trimmed);
            }
            continue;
        }
        if let Some(entry) = git_short_status_entry_from_line(line) {
            changed_entries.push(entry);
        }
    }
    if !saw_status_header && changed_entries.is_empty() {
        return None;
    }
    Some(GitRepositoryStateObservation {
        branch,
        dirty: !changed_entries.is_empty(),
        changed_entries,
    })
}

pub(super) fn git_repository_state_answer(
    observation: &GitRepositoryStateObservation,
    response_shape: Option<crate::OutputResponseShape>,
) -> String {
    let worktree = if observation.dirty { "dirty" } else { "clean" };
    let mut fields = Vec::new();
    if let Some(branch) = observation
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
    {
        fields.push(format!("git.branch={branch}"));
    }
    fields.push(format!("git.worktree={worktree}"));
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    ) {
        return fields.join(" ");
    }
    fields.push(format!(
        "git.changed.count={}",
        observation.changed_entries.len()
    ));
    for (idx, entry) in observation.changed_entries.iter().enumerate() {
        fields.push(format!(
            "git.changed[{idx}]={} {}",
            entry.status, entry.path
        ));
    }
    fields.join("\n")
}

pub(super) fn git_basic_direct_answer_candidate(
    _state: Option<&AppState>,
    route: Option<&crate::RouteResult>,
    body: &str,
    branch: Option<&str>,
    response_shape: Option<crate::OutputResponseShape>,
    _allow_localized_direct_template: bool,
    _prefer_english: bool,
) -> Option<String> {
    let route = route?;
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::GitRepositoryState,
    ) {
        return None;
    }
    let effective_response_shape = if route.output_contract.exact_sentence_count == Some(1) {
        Some(crate::OutputResponseShape::OneSentence)
    } else {
        response_shape
    };
    if route_git_repository_state_requires_language_synthesis(route) {
        return None;
    }
    for candidate in git_basic_observation_text_candidates(body) {
        let Some(observation) =
            git_repository_state_observation_from_status_output(&candidate, branch)
        else {
            continue;
        };
        if matches!(
            effective_response_shape,
            Some(crate::OutputResponseShape::Scalar)
        ) {
            return Some(if observation.dirty { "dirty" } else { "clean" }.to_string());
        }
        return Some(git_repository_state_answer(
            &observation,
            effective_response_shape,
        ));
    }
    None
}

pub(super) fn latest_git_repository_state_direct_answer(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    loop_state: &LoopState,
    response_shape: Option<crate::OutputResponseShape>,
    allow_localized_direct_template: bool,
    prefer_english: bool,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::GitRepositoryState,
    ) {
        return None;
    }
    let idx = latest_successful_step_index(loop_state, |step| step.skill == "git_basic")?;
    let body = loop_state.executed_step_results[idx]
        .output
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())?;
    let branch = latest_git_current_branch(loop_state);
    git_basic_direct_answer_candidate(
        state,
        Some(route),
        body,
        branch.as_deref(),
        response_shape,
        allow_localized_direct_template,
        prefer_english,
    )
}

pub(super) fn latest_git_current_branch(loop_state: &LoopState) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "git_basic")
        .filter_map(|step| step.output.as_deref())
        .find_map(git_current_branch_from_output)
}

pub(super) fn git_current_branch_from_output(body: &str) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim()) {
        if let Some(branch) = git_current_branch_from_json_value(&value) {
            return Some(branch);
        }
    }
    git_basic_observation_text_candidates(body)
        .iter()
        .find_map(|candidate| git_current_branch_from_text(candidate))
}

pub(super) fn git_basic_json_action(value: &serde_json::Value) -> Option<&str> {
    value
        .get("action")
        .or_else(|| value.get("raw_action"))
        .or_else(|| {
            value
                .get("field_value")
                .and_then(|field_value| field_value.get("action"))
        })
        .or_else(|| {
            value
                .get("extra")
                .and_then(|extra| extra.get("action").or_else(|| extra.get("raw_action")))
        })
        .or_else(|| {
            value
                .get("extra")
                .and_then(|extra| extra.get("field_value"))
                .and_then(|field_value| field_value.get("action"))
        })
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
}

pub(super) fn git_current_branch_from_json_value(value: &serde_json::Value) -> Option<String> {
    git_string_field_from_json_value(value, &["current_branch", "branch"])
        .or_else(|| {
            value.get("field_value").and_then(|field_value| {
                git_string_field_from_json_value(field_value, &["current_branch", "branch"])
            })
        })
        .or_else(|| {
            value
                .get("extra")
                .and_then(git_current_branch_from_json_value)
        })
        .or_else(|| {
            value
                .get("output")
                .or_else(|| value.get("text"))
                .and_then(serde_json::Value::as_str)
                .and_then(|text| {
                    normalized_scalar_candidate(text).or_else(|| git_current_branch_from_text(text))
                })
        })
}

pub(super) fn git_string_field_from_json_value(
    value: &serde_json::Value,
    fields: &[&str],
) -> Option<String> {
    fields.iter().find_map(|field| {
        value
            .get(*field)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

pub(super) fn git_current_branch_from_text(body: &str) -> Option<String> {
    for raw_line in body.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("exit=") {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("* ") {
            return rest
                .split_whitespace()
                .next()
                .map(str::trim)
                .filter(|branch| !branch.is_empty())
                .map(ToOwned::to_owned);
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            if let Some(branch) = git_current_branch_from_status_header(rest) {
                return Some(branch);
            }
        }
    }
    None
}

pub(super) fn git_basic_observation_text_candidates(body: &str) -> Vec<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        collect_git_basic_observation_text_candidates(&value, &mut candidates, 0);
    }
    candidates.push(trimmed.to_string());
    dedup_nonempty_strings(candidates)
}

pub(super) fn collect_git_basic_observation_text_candidates(
    value: &serde_json::Value,
    candidates: &mut Vec<String>,
    depth: usize,
) {
    if depth > 3 {
        return;
    }
    match value {
        serde_json::Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                return;
            }
            if let Ok(inner) = serde_json::from_str::<serde_json::Value>(text) {
                collect_git_basic_observation_text_candidates(&inner, candidates, depth + 1);
            }
            candidates.push(text.to_string());
        }
        serde_json::Value::Object(obj) => {
            for key in ["output", "command_output", "text"] {
                if let Some(value) = obj.get(key) {
                    collect_git_basic_observation_text_candidates(value, candidates, depth + 1);
                }
            }
            if let Some(extra) = obj.get("extra") {
                collect_git_basic_observation_text_candidates(extra, candidates, depth + 1);
            }
        }
        _ => {}
    }
}

pub(super) fn dedup_nonempty_strings(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || deduped.iter().any(|existing| existing == value) {
            continue;
        }
        deduped.push(value.to_string());
    }
    deduped
}

pub(super) fn git_current_branch_from_status_header(header: &str) -> Option<String> {
    let rest = header.strip_prefix("## ").unwrap_or(header);
    rest.split(['.', ' ', '\t'])
        .next()
        .map(str::trim)
        .filter(|branch| !branch.is_empty() && *branch != "HEAD")
        .map(ToOwned::to_owned)
}

pub(super) fn git_short_status_entry_from_line(line: &str) -> Option<GitStatusEntry> {
    if !line_looks_like_git_short_status_entry(line) {
        return None;
    }
    let status = line.get(..2)?.trim().to_string();
    let path = line.get(3..)?.trim();
    if status.is_empty() || path.is_empty() {
        return None;
    }
    Some(GitStatusEntry {
        status,
        path: path.to_string(),
    })
}

pub(super) fn line_looks_like_git_short_status_entry(line: &str) -> bool {
    let bytes = line.as_bytes();
    if bytes.len() < 3 {
        return false;
    }
    let status_a = bytes[0] as char;
    let status_b = bytes[1] as char;
    let sep = bytes[2] as char;
    (sep == ' ' || sep == '\t')
        && (is_git_short_status_code(status_a) || is_git_short_status_code(status_b))
}

pub(super) fn is_git_short_status_code(ch: char) -> bool {
    matches!(ch, 'M' | 'A' | 'D' | 'R' | 'C' | 'U' | '?' | '!')
}

pub(crate) fn answer_is_git_repository_state_machine_summary(answer: &str) -> bool {
    let mut saw_git_field = false;
    for token in answer.split_whitespace() {
        let token = token
            .trim_matches(|ch: char| matches!(ch, ',' | ';' | '.' | '，' | '；' | '。'))
            .trim();
        if token.is_empty() {
            continue;
        }
        if !token.starts_with("git.") || !token.contains('=') {
            return false;
        }
        saw_git_field = true;
    }
    saw_git_field
}
