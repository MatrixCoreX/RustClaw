use std::path::Path;

use serde::Deserialize;

use super::{AgentRunContext, LoopState};
use crate::{llm_gateway, AppState, ClaimedTask};

const OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/observed_answer_fallback_prompt.md");
const OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH: &str =
    "prompts/observed_answer_fallback_prompt.md";

fn render_observed_vars(mut text: String, vars: &[(&str, &str)]) -> String {
    for (name, value) in vars {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

fn observed_t(
    state: Option<&AppState>,
    key: &str,
    default_zh: &str,
    default_en: &str,
    prefer_english: bool,
) -> String {
    observed_t_with_vars(state, key, default_zh, default_en, prefer_english, &[])
}

fn observed_t_with_vars(
    state: Option<&AppState>,
    key: &str,
    default_zh: &str,
    default_en: &str,
    prefer_english: bool,
    vars: &[(&str, &str)],
) -> String {
    match state {
        Some(state) => crate::bilingual_t_with_default_vars(
            state,
            key,
            default_zh,
            default_en,
            prefer_english,
            vars,
        ),
        None => render_observed_vars(
            if prefer_english {
                default_en.to_string()
            } else {
                default_zh.to_string()
            },
            vars,
        ),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GenericObservedOutput {
    pub(crate) skill: String,
    #[cfg(test)]
    pub(crate) action_label: String,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
struct ObservedAnswerFallbackOut {
    #[serde(default)]
    answer: String,
    #[serde(default)]
    qualified: bool,
    #[serde(default)]
    needs_clarify: bool,
    #[serde(default)]
    is_meta_instruction: bool,
    #[serde(default)]
    publishable: bool,
    #[serde(default)]
    confidence: f64,
    #[serde(default, rename = "reason")]
    _reason: String,
}

fn latest_successful_step_index<F>(loop_state: &LoopState, predicate: F) -> Option<usize>
where
    F: Fn(&crate::executor::StepExecutionResult) -> bool,
{
    loop_state
        .executed_step_results
        .iter()
        .rposition(|step| step.is_ok() && predicate(step))
}

#[cfg(test)]
fn latest_successful_step_output<F>(loop_state: &LoopState, predicate: F) -> Option<String>
where
    F: Fn(&crate::executor::StepExecutionResult) -> bool,
{
    latest_successful_step_index(loop_state, predicate).and_then(|idx| {
        loop_state.executed_step_results[idx]
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string)
    })
}

#[cfg(test)]
pub(crate) fn extract_latest_successful_read_file_output(loop_state: &LoopState) -> Option<String> {
    latest_successful_step_output(loop_state, |step| step.skill == "read_file")
}

#[cfg(test)]
pub(crate) fn extract_latest_successful_list_dir_output(loop_state: &LoopState) -> Option<String> {
    latest_successful_step_output(loop_state, |step| step.skill == "list_dir")
}

pub(crate) fn extract_latest_generic_successful_output(
    loop_state: &LoopState,
) -> Option<GenericObservedOutput> {
    let idx = latest_successful_step_index(loop_state, |step| {
        if matches!(step.skill.as_str(), "read_file" | "list_dir") {
            return false;
        }
        let body = step.output.as_deref().map(str::trim).unwrap_or_default();
        !body.is_empty()
            && (crate::finalize::classify_observed_content_status(body)
                == crate::finalize::ObservedContentStatus::ContentAvailable
                || structured_scalar_candidate(None, &step.skill, body, None, false).is_some()
                || structured_observed_body(&step.skill, body).is_some())
            || system_basic_info_value(&step.skill, body).is_some()
            || system_basic_existence_with_path_value(&step.skill, body).is_some()
    })?;
    let step = &loop_state.executed_step_results[idx];
    let body = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    Some(GenericObservedOutput {
        skill: step.skill.clone(),
        #[cfg(test)]
        action_label: format!("{} skill({}): success", step.step_id, step.skill),
        body: body.to_string(),
    })
}

fn extract_latest_successful_output_for_skill(
    loop_state: &LoopState,
    skill_name: &str,
) -> Option<GenericObservedOutput> {
    let idx = latest_successful_step_index(loop_state, |step| step.skill == skill_name)?;
    let step = &loop_state.executed_step_results[idx];
    let body = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    Some(GenericObservedOutput {
        skill: step.skill.clone(),
        #[cfg(test)]
        action_label: format!("{} skill({}): success", step.step_id, step.skill),
        body: body.to_string(),
    })
}

fn latest_successful_list_dir_answer_candidate(
    loop_state: &LoopState,
    response_shape: Option<crate::OutputResponseShape>,
    max_entries: Option<usize>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
) -> Option<String> {
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    ) {
        return None;
    }
    let idx = latest_successful_step_index(loop_state, |_| true)?;
    let step = &loop_state.executed_step_results[idx];
    if !step.is_ok() || step.skill != "list_dir" {
        return None;
    }
    let listing =
        normalized_observed_listing(step.output.as_deref().unwrap_or_default(), max_entries)?;
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        let mut lines = listing
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        let first = lines.next()?;
        if lines.next().is_some() {
            return None;
        }
        if prefer_full_path {
            if let Some(resolved) = resolve_listing_entry_full_path(first, auto_locator_path) {
                return Some(resolved);
            }
        }
        return Some(first.to_string());
    }
    Some(listing)
}

fn canonical_existing_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn resolve_listing_entry_full_path(entry: &str, auto_locator_path: Option<&str>) -> Option<String> {
    let entry = entry.trim().trim_end_matches('/');
    if entry.is_empty() {
        return None;
    }
    let auto_locator_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(Path::new)?;

    if auto_locator_path.is_file() {
        let file_name_matches = auto_locator_path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .is_some_and(|name| name.eq_ignore_ascii_case(entry));
        if file_name_matches {
            return Some(canonical_existing_path(auto_locator_path));
        }
        if let Some(parent) = auto_locator_path.parent() {
            let candidate = parent.join(entry);
            if candidate.exists() {
                return Some(canonical_existing_path(&candidate));
            }
        }
    }

    if auto_locator_path.is_dir() {
        let candidate = auto_locator_path.join(entry);
        if candidate.exists() {
            return Some(canonical_existing_path(&candidate));
        }
    }

    None
}

fn normalized_listing_entry_count(listing: &str) -> usize {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count()
}

fn trim_listing_to_max_entries(listing: &str, max_entries: Option<usize>) -> Option<String> {
    let mut lines = listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if let Some(limit) = max_entries.filter(|limit| *limit > 0) {
        lines.truncate(limit);
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn looks_like_shell_long_listing_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed == "exit=0" || trimmed.starts_with("total ") {
        return false;
    }
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    matches!(first, '-' | 'd' | 'l' | 'b' | 'c' | 'p' | 's')
        && trimmed.split_whitespace().count() >= 9
}

fn parse_small_zh_number_prefix(text: &str) -> Option<(usize, usize)> {
    fn digit_value(ch: char) -> Option<usize> {
        match ch {
            '零' => Some(0),
            '一' => Some(1),
            '二' | '两' => Some(2),
            '三' => Some(3),
            '四' => Some(4),
            '五' => Some(5),
            '六' => Some(6),
            '七' => Some(7),
            '八' => Some(8),
            '九' => Some(9),
            _ => None,
        }
    }

    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return None;
    }
    if chars[0] == '十' {
        let ones = chars.get(1).and_then(|ch| digit_value(*ch)).unwrap_or(0);
        let consumed = if ones > 0 { 2 } else { 1 };
        return Some((10 + ones, consumed));
    }
    let tens = digit_value(chars[0])?;
    if chars.get(1) == Some(&'十') {
        let ones = chars.get(2).and_then(|ch| digit_value(*ch)).unwrap_or(0);
        let consumed = if ones > 0 { 3 } else { 2 };
        return Some((tens * 10 + ones, consumed));
    }
    Some((tens, 1))
}

fn parse_small_en_number_prefix(text: &str) -> Option<(usize, usize)> {
    const WORDS: [(&str, usize); 12] = [
        ("twelve", 12),
        ("eleven", 11),
        ("ten", 10),
        ("nine", 9),
        ("eight", 8),
        ("seven", 7),
        ("six", 6),
        ("five", 5),
        ("four", 4),
        ("three", 3),
        ("two", 2),
        ("one", 1),
    ];

    let trimmed = text.trim_start();
    for (word, value) in WORDS {
        let Some(rest) = trimmed.strip_prefix(word) else {
            continue;
        };
        if rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
        {
            continue;
        }
        return Some((value, word.chars().count()));
    }
    None
}

fn parse_positive_number_prefix(text: &str) -> Option<usize> {
    let trimmed = text.trim_start();
    let digit_len = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_len > 0 {
        return trimmed[..digit_len]
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0);
    }
    parse_small_zh_number_prefix(trimmed)
        .map(|(value, _)| value)
        .or_else(|| parse_small_en_number_prefix(trimmed).map(|(value, _)| value))
        .filter(|n| *n > 0)
}

fn trim_listing_limit_token(token: &str) -> &str {
    token.trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    })
}

fn parse_number_prefix_with_suffix(token: &str) -> Option<(usize, &str)> {
    let trimmed = trim_listing_limit_token(token);
    if trimmed.is_empty() {
        return None;
    }
    let digit_len = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_len > 0 {
        let value = trimmed[..digit_len]
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0)?;
        return Some((value, &trimmed[digit_len..]));
    }
    let (value, consumed_chars) = parse_small_zh_number_prefix(trimmed)?;
    let suffix_start = trimmed
        .char_indices()
        .nth(consumed_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(trimmed.len());
    Some((value, &trimmed[suffix_start..]))
}

fn parse_number_like_prefix_with_suffix(token: &str) -> Option<(usize, &str)> {
    parse_number_prefix_with_suffix(token).or_else(|| {
        let trimmed = trim_listing_limit_token(token);
        let (value, consumed_chars) = parse_small_en_number_prefix(trimmed)?;
        let suffix_start = trimmed
            .char_indices()
            .nth(consumed_chars)
            .map(|(idx, _)| idx)
            .unwrap_or(trimmed.len());
        Some((value, &trimmed[suffix_start..]))
    })
}

fn token_starts_with_listing_limit_unit(token: &str) -> bool {
    let trimmed = trim_listing_limit_token(token);
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    [
        "个", "条", "项", "行", "份", "files", "file", "entries", "entry", "items", "item",
        "lines", "line", "rows", "row",
    ]
    .iter()
    .any(|needle| lower.starts_with(needle))
}

fn token_is_listing_limit_modifier(token: &str) -> bool {
    let lower = trim_listing_limit_token(token).to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "the"
            | "most"
            | "more"
            | "recent"
            | "recently"
            | "modified"
            | "latest"
            | "newest"
            | "updated"
            | "changed"
            | "runtime"
            | "run"
            | "log"
            | "logs"
    )
}

fn suffix_has_listing_limit_unit(suffix: &str) -> bool {
    let trimmed = suffix.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    if token_starts_with_listing_limit_unit(trimmed) {
        return true;
    }
    let tokens = trimmed.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return false;
    }
    let mut modifiers_skipped = 0usize;
    for token in tokens {
        if token_starts_with_listing_limit_unit(token) {
            return true;
        }
        if !token_is_listing_limit_modifier(token) {
            return false;
        }
        modifiers_skipped += 1;
        if modifiers_skipped >= 4 {
            return false;
        }
    }
    false
}

fn requested_listing_limit_from_intent(intent: &str) -> Option<usize> {
    let trimmed = intent.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    for marker in ["top", "first"] {
        if let Some(idx) = lower.find(marker) {
            let suffix = &trimmed[idx + marker.len()..];
            if let Some(limit) = parse_positive_number_prefix(suffix) {
                return Some(limit);
            }
        }
    }
    for marker in ['前', '头'] {
        if let Some(idx) = trimmed.find(marker) {
            let suffix = &trimmed[idx + marker.len_utf8()..];
            if let Some(limit) = parse_positive_number_prefix(suffix) {
                return Some(limit);
            }
        }
    }
    let starts = std::iter::once(0usize).chain(trimmed.char_indices().skip(1).map(|(idx, _)| idx));
    for idx in starts {
        let prev = if idx == 0 {
            None
        } else {
            trimmed[..idx].chars().next_back()
        };
        if prev.is_some_and(|ch| ch.is_ascii_alphanumeric()) {
            continue;
        }
        let Some((limit, suffix)) = parse_number_like_prefix_with_suffix(&trimmed[idx..]) else {
            continue;
        };
        if suffix_has_listing_limit_unit(suffix) {
            return Some(limit);
        }
    }
    None
}

fn requested_listing_limit(route: Option<&crate::RouteResult>) -> Option<usize> {
    requested_listing_limit_from_intent(route?.resolved_intent.as_str())
}

fn route_requests_scalar_count(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount
}

fn route_requests_hidden_entries_check(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::HiddenEntriesCheck
}

pub(crate) fn route_prefers_direct_observed_answer_for_scalar(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::HiddenEntriesCheck
        )
}

pub(crate) fn scalar_route_prefers_structured_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && (route_prefers_direct_observed_answer_for_scalar(route)
            || extract_latest_generic_successful_output(loop_state)
                .is_some_and(|observed| observed.skill == "health_check"))
}

fn route_requests_directory_purpose_summary(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::DirectoryPurposeSummary
}

fn route_requests_recent_artifacts_judgment(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::RecentArtifactsJudgment
}

fn route_requests_workspace_project_summary(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::WorkspaceProjectSummary
}

fn route_requests_quantity_comparison(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
}

fn route_requests_scalar_path_only(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarPathOnly
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn health_check_request_prefers_summary(
    route: Option<&crate::RouteResult>,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let request_text = agent_run_context
        .and_then(|ctx| ctx.user_request.as_deref())
        .filter(|text| !text.trim().is_empty())
        .or_else(|| route.map(|route| route.resolved_intent.as_str()))
        .unwrap_or_default()
        .to_ascii_lowercase();
    contains_any(
        &request_text,
        &[
            "summarize",
            "summary",
            "key fields",
            "important findings",
            "host operating system",
            "只总结",
            "关键字段",
            "最重要",
            "操作系统",
            "不展开总结",
        ],
    )
}

fn route_allows_raw_listing_direct_answer(route: Option<&crate::RouteResult>) -> bool {
    route.is_none_or(|route| !route.output_contract.requires_content_evidence)
}

fn latest_list_dir_listing(loop_state: &LoopState) -> Option<String> {
    let idx = latest_successful_step_index(loop_state, |_| true)?;
    let step = &loop_state.executed_step_results[idx];
    if !step.is_ok() || step.skill != "list_dir" {
        return None;
    }
    normalized_observed_listing(step.output.as_deref().unwrap_or_default(), None)
}

fn hidden_entries_from_listing(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| line.starts_with('.'))
        .map(ToString::to_string)
        .collect()
}

fn hidden_entries_from_entries(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .filter(|entry| entry.starts_with('.'))
        .map(ToString::to_string)
        .collect()
}

fn hidden_entries_direct_answer(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    if !route_requests_hidden_entries_check(route) {
        return None;
    }
    let hidden_entries = latest_directory_listing_entries(loop_state, None, None)
        .map(|entries| hidden_entries_from_entries(&entries))
        .or_else(|| {
            latest_list_dir_listing(loop_state).map(|listing| hidden_entries_from_listing(&listing))
        })?;
    let examples = hidden_entries
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if route.ask_mode.is_plain_act()
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
    {
        if hidden_entries.is_empty() {
            return Some(observed_t(
                state,
                "clawd.msg.hidden_entries_none_scalar",
                "没有",
                "No",
                prefer_english,
            ));
        }
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.hidden_entries_found_scalar_examples",
            "有。示例：{examples}",
            "Yes: {examples}",
            prefer_english,
            &[("examples", examples.as_str())],
        ));
    }
    if hidden_entries.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.hidden_entries_none_with_reason",
            "没有发现隐藏文件；这类以点开头的条目通常用于保存配置、元数据或本地状态。",
            "No hidden entries were found; dot-prefixed entries usually store config, metadata, or local state.",
            prefer_english,
        ));
    }
    Some(observed_t_with_vars(
        state,
        "clawd.msg.hidden_entries_found_with_reason",
        "有隐藏文件：{examples}；这类以点开头的条目通常用于保存配置、元数据或本地状态。",
        "Yes: {examples}. Dot-prefixed entries usually store config, metadata, or local state.",
        prefer_english,
        &[("examples", examples.as_str())],
    ))
}

fn listing_entries(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn latest_directory_listing_entries(
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    max_entries: Option<usize>,
) -> Option<Vec<String>> {
    let idx = latest_successful_step_index(loop_state, |_| true)?;
    let step = &loop_state.executed_step_results[idx];
    if !step.is_ok() {
        return None;
    }
    let body = step.output.as_deref().unwrap_or_default();
    match step.skill.as_str() {
        "list_dir" => {
            normalized_observed_listing(body, max_entries).map(|listing| listing_entries(&listing))
        }
        "run_cmd" => run_cmd_listing_text_candidate(body, auto_locator_path, max_entries)
            .map(|listing| listing_entries(&listing)),
        "system_basic" => {
            let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
            let mut entries = inventory_dir_names(&value)?;
            if let Some(limit) = max_entries.filter(|limit| *limit > 0) {
                entries.truncate(limit);
            }
            Some(entries)
        }
        _ => None,
    }
    .filter(|entries| !entries.is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryPurposeKind {
    Documentation,
    ScriptsAutomation,
    Configs,
    Logs,
    DataArtifacts,
    ServiceOps,
    Generic,
}

#[derive(Default)]
struct DirectoryPurposeScores {
    docs: i32,
    scripts: i32,
    configs: i32,
    logs: i32,
    data: i32,
    service_ops: i32,
}

fn directory_scope_hint(
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    locator_hint
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
        .map(|hint| {
            hint.trim_end_matches('/')
                .trim_end_matches('\\')
                .to_string()
        })
        .filter(|hint| !hint.is_empty())
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .and_then(|path| Path::new(path).file_name().and_then(|value| value.to_str()))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToString::to_string)
        })
}

fn add_directory_scope_bias(scores: &mut DirectoryPurposeScores, scope_hint: Option<&str>) {
    let Some(scope_hint) = scope_hint.map(str::trim).filter(|hint| !hint.is_empty()) else {
        return;
    };
    let lower = scope_hint.to_ascii_lowercase();
    if lower.contains("doc") || lower.contains("readme") {
        scores.docs += 4;
    }
    if lower.contains("script") || lower == "bin" || lower == "tools" {
        scores.scripts += 4;
    }
    if lower.contains("config") || lower == "conf" {
        scores.configs += 4;
    }
    if lower.contains("log") {
        scores.logs += 4;
    }
    if lower.contains("data") || lower.contains("db") || lower.contains("cache") {
        scores.data += 4;
    }
    if lower.contains("service")
        || lower.contains("deploy")
        || lower.contains("systemd")
        || lower.contains("docker")
    {
        scores.service_ops += 4;
    }
}

fn add_directory_entry_scores(scores: &mut DirectoryPurposeScores, entry: &str) {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return;
    }
    let lower = trimmed.trim_end_matches('/').to_ascii_lowercase();
    let ext = lower
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default();

    match ext {
        "md" | "rst" | "txt" | "pdf" | "doc" | "docx" => scores.docs += 3,
        "sh" | "py" | "rb" | "ps1" | "fish" => scores.scripts += 4,
        "toml" | "yaml" | "yml" | "json" | "ini" | "conf" | "cfg" | "env" => scores.configs += 3,
        "log" | "trace" | "out" | "err" => scores.logs += 4,
        "sqlite" | "db" | "csv" | "tsv" | "parquet" | "jsonl" => scores.data += 4,
        "service" => scores.service_ops += 4,
        _ => {}
    }

    for marker in [
        "readme",
        "guide",
        "manual",
        "checklist",
        "summary",
        "report",
        "spec",
        "design",
        "plan",
        "contract",
        "notes",
        "release",
        "faq",
        "tutorial",
    ] {
        if lower.contains(marker) {
            scores.docs += 2;
        }
    }
    for marker in [
        "run",
        "start",
        "build",
        "deploy",
        "test",
        "sync",
        "setup",
        "migrate",
        "seed",
        "bootstrap",
        "lint",
        "check",
    ] {
        if lower.contains(marker) {
            scores.scripts += 2;
        }
    }
    for marker in [
        "config", "settings", "profile", "env", "template", "example",
    ] {
        if lower.contains(marker) {
            scores.configs += 2;
        }
    }
    for marker in [
        "log", "trace", "stderr", "stdout", "journal", "panic", "error",
    ] {
        if lower.contains(marker) {
            scores.logs += 2;
        }
    }
    for marker in [
        "data", "cache", "snapshot", "export", "backup", "dump", "fixture", "seed",
    ] {
        if lower.contains(marker) {
            scores.data += 2;
        }
    }
    for marker in [
        "service",
        "systemd",
        "docker",
        "compose",
        "k8s",
        "helm",
        "supervisor",
        "nginx",
        "caddy",
    ] {
        if lower.contains(marker) {
            scores.service_ops += 2;
        }
    }
}

fn classify_directory_purpose(
    entries: &[String],
    scope_hint: Option<&str>,
) -> DirectoryPurposeKind {
    let mut scores = DirectoryPurposeScores::default();
    add_directory_scope_bias(&mut scores, scope_hint);
    for entry in entries {
        add_directory_entry_scores(&mut scores, entry);
    }
    let mut ranked = vec![
        (DirectoryPurposeKind::Documentation, scores.docs),
        (DirectoryPurposeKind::ScriptsAutomation, scores.scripts),
        (DirectoryPurposeKind::Configs, scores.configs),
        (DirectoryPurposeKind::Logs, scores.logs),
        (DirectoryPurposeKind::DataArtifacts, scores.data),
        (DirectoryPurposeKind::ServiceOps, scores.service_ops),
    ];
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    let (winner, winner_score) = ranked[0];
    let second_score = ranked.get(1).map(|(_, score)| *score).unwrap_or_default();
    if winner_score < 4 || winner_score < second_score + 2 {
        DirectoryPurposeKind::Generic
    } else {
        winner
    }
}

fn directory_purpose_summary_text(
    state: Option<&AppState>,
    kind: DirectoryPurposeKind,
    prefer_english: bool,
) -> String {
    match kind {
        DirectoryPurposeKind::Documentation => observed_t(
            state,
            "clawd.msg.directory_purpose_docs",
            "这个目录主要放说明文档、操作指引和检查清单。",
            "This directory mainly holds docs, guides, and checklists.",
            prefer_english,
        ),
        DirectoryPurposeKind::ScriptsAutomation => observed_t(
            state,
            "clawd.msg.directory_purpose_scripts",
            "这个目录主要放运行、测试和维护用的脚本。",
            "This directory mainly holds scripts for running, testing, and maintenance.",
            prefer_english,
        ),
        DirectoryPurposeKind::Configs => observed_t(
            state,
            "clawd.msg.directory_purpose_configs",
            "这个目录主要放配置文件和环境参数。",
            "This directory mainly holds config files and environment settings.",
            prefer_english,
        ),
        DirectoryPurposeKind::Logs => observed_t(
            state,
            "clawd.msg.directory_purpose_logs",
            "这个目录主要放运行日志和排查记录。",
            "This directory mainly holds runtime logs and troubleshooting records.",
            prefer_english,
        ),
        DirectoryPurposeKind::DataArtifacts => observed_t(
            state,
            "clawd.msg.directory_purpose_data",
            "这个目录主要放运行期数据、数据库或导出结果。",
            "This directory mainly holds runtime data, databases, or exported artifacts.",
            prefer_english,
        ),
        DirectoryPurposeKind::ServiceOps => observed_t(
            state,
            "clawd.msg.directory_purpose_service_ops",
            "这个目录主要放服务启动、部署和运维相关文件。",
            "This directory mainly holds service startup, deployment, and ops-related files.",
            prefer_english,
        ),
        DirectoryPurposeKind::Generic => observed_t(
            state,
            "clawd.msg.directory_purpose_generic",
            "这个目录里的条目更像围绕同一任务的一组工作文件和辅助材料。",
            "These entries look like a grouped set of working files and support materials for one area.",
            prefer_english,
        ),
    }
}

fn directory_purpose_summary_direct_answer(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    requested_listing_limit: Option<usize>,
    prefer_english: bool,
) -> Option<String> {
    if !route_requests_directory_purpose_summary(route) {
        return None;
    }
    let entries =
        latest_directory_listing_entries(loop_state, auto_locator_path, requested_listing_limit)?;
    let scope_hint = directory_scope_hint(
        Some(route.output_contract.locator_hint.as_str()),
        auto_locator_path,
    );
    let summary = directory_purpose_summary_text(
        state,
        classify_directory_purpose(&entries, scope_hint.as_deref()),
        prefer_english,
    );
    Some(format!("{}\n\n{summary}", entries.join("\n")))
}

fn workspace_project_summary_from_entries(
    state: Option<&AppState>,
    entries: &[String],
    prefer_english: bool,
) -> Option<String> {
    fn normalize_workspace_entry(value: &str) -> &str {
        value.trim().trim_end_matches('/')
    }

    if entries.is_empty() {
        return None;
    }
    let contains_exact = |target: &str| {
        let target = normalize_workspace_entry(target);
        entries
            .iter()
            .any(|entry| normalize_workspace_entry(entry).eq_ignore_ascii_case(target))
    };
    let contains_substring = |needle: &str| {
        entries.iter().any(|entry| {
            normalize_workspace_entry(entry)
                .to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
        })
    };
    let count_matching = |needle: &str| {
        entries
            .iter()
            .filter(|entry| {
                normalize_workspace_entry(entry)
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
            })
            .count()
    };

    let has_rust_workspace = contains_exact("Cargo.toml") && contains_exact("crates");
    let has_ui = contains_exact("UI") || contains_exact("ui");
    let has_configs = contains_exact("configs");
    let has_docs = contains_exact("README.md")
        || contains_exact("README.zh-CN.md")
        || contains_exact("docs")
        || contains_exact("document");
    let has_service_ops = contains_exact("docker-compose.yml")
        || contains_exact("services")
        || contains_exact("systemd")
        || contains_exact("rustclaw.service")
        || entries.iter().any(|entry| {
            let entry = normalize_workspace_entry(entry);
            entry.starts_with("start-") && entry.ends_with(".sh")
        });
    let channel_markers = ["telegram", "wechat", "whatsapp", "feishu", "lark"];
    let channel_hits = channel_markers
        .iter()
        .map(|marker| count_matching(marker))
        .sum::<usize>();
    let has_skills_or_runtime = contains_exact("prompts")
        || contains_exact("external_skills")
        || contains_exact("skill_develop")
        || contains_exact("skills_output")
        || contains_exact("rustclaw")
        || contains_substring("clawd");

    if has_rust_workspace && has_ui && has_service_ops && has_skills_or_runtime && channel_hits >= 2
    {
        return Some(observed_t(
            state,
            "clawd.msg.workspace_project_summary_assistant_platform",
            "这是一个可本地部署的智能助手平台仓库，带网页界面、多聊天渠道接入和后台服务，用来通过聊天或浏览器处理任务、记忆、调度和自动化。",
            "This looks like a locally deployable assistant platform with a web UI, multiple chat-channel adapters, and backend services for tasks, memory, scheduling, and automation.",
            prefer_english,
        ));
    }

    if has_rust_workspace && has_ui && has_configs && has_docs {
        return Some(observed_t(
            state,
            "clawd.msg.workspace_project_summary_fullstack_control_console",
            "这是一个带网页界面和配置脚本的软件项目仓库，前后端都在这里，主要用来构建、部署并管理一套本地服务。",
            "This looks like a software project repository with both backend code and a web console, mainly for building, deploying, and managing a local service stack.",
            prefer_english,
        ));
    }

    if has_rust_workspace && has_docs {
        return Some(observed_t(
            state,
            "clawd.msg.workspace_project_summary_rust_service",
            "这是一个以 Rust 为主的软件项目仓库，里面有代码、文档和运行脚本，主要用来构建并运行一套服务。",
            "This looks like a Rust-based software repository with code, docs, and run scripts for building and operating a service.",
            prefer_english,
        ));
    }

    Some(observed_t(
        state,
        "clawd.msg.workspace_project_summary_generic",
        "这是一个软件项目仓库，里面放着代码、文档、配置和启动脚本，主要用来构建并运行一套程序。",
        "This looks like a software project repository with code, docs, configs, and startup scripts for building and running an application.",
        prefer_english,
    ))
}

fn workspace_project_summary_direct_answer(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if !route_requests_workspace_project_summary(route) {
        return None;
    }
    let entries = latest_directory_listing_entries(loop_state, auto_locator_path, None)?;
    workspace_project_summary_from_entries(state, &entries, prefer_english)
}

fn comparison_winner_from_route(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<String> {
    if !route_requests_quantity_comparison(route) {
        return None;
    }
    let winner = route.output_contract.locator_hint.trim();
    if winner.is_empty() {
        return None;
    }
    let successful_steps = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok())
        .count();
    (successful_steps >= 2).then(|| winner.to_string())
}

fn count_answer_from_latest_listing(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<String> {
    if !route_requests_scalar_count(route) {
        return None;
    }
    let listing = latest_list_dir_listing(loop_state)?;
    Some(normalized_listing_entry_count(&listing).to_string())
}

fn trim_for_observed_prompt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push_str("\n...[truncated]");
    out
}

fn looks_like_structured_machine_output_line(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
}

fn normalized_scalar_candidate(body: &str) -> Option<String> {
    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .filter(|line| !looks_like_structured_machine_output_line(line))
        .collect::<Vec<_>>();
    (lines.len() == 1).then(|| lines[0].to_string())
}

fn value_scalar_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => Some("null".to_string()),
        serde_json::Value::Bool(v) => Some(v.to_string()),
        serde_json::Value::Number(v) => Some(v.to_string()),
        serde_json::Value::String(v) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
        _ => None,
    }
}

fn text_contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        )
    })
}

fn text_contains_ascii_alpha(text: &str) -> bool {
    text.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn observed_request_language_hint(user_text: &str) -> &'static str {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return "config_default";
    }
    match (
        text_contains_cjk(trimmed),
        text_contains_ascii_alpha(trimmed),
    ) {
        (true, false) => "zh-CN",
        (false, true) => "en",
        (true, true) => "mixed",
        (false, false) => "config_default",
    }
}

fn observed_response_style_hint(agent_run_context: Option<&AgentRunContext>) -> &'static str {
    match agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.response_shape)
    {
        Some(crate::OutputResponseShape::Scalar) => {
            "Return only the final scalar value with no label, prefix, suffix, or explanation."
        }
        Some(crate::OutputResponseShape::FileToken) => {
            "Return only the delivery token or delivery-marker output itself. Do not add explanation."
        }
        Some(crate::OutputResponseShape::OneSentence) => {
            "Return exactly one sentence unless the current user request explicitly asks for another exact sentence count."
        }
        Some(crate::OutputResponseShape::Free) => {
            "Return a short direct answer, usually one short paragraph or compact listing plus one concise conclusion."
        }
        None => "Return the shortest grounded answer that directly satisfies the user request.",
    }
}

fn db_basic_scalar_candidate(value: &serde_json::Value) -> Option<String> {
    let columns = value.get("columns")?.as_array()?;
    if columns.len() != 1 {
        return None;
    }
    let column = columns[0].as_str()?.trim();
    if column.is_empty() {
        return None;
    }
    let row = value.get("rows")?.as_array()?.first()?.as_object()?;
    value_scalar_text(row.get(column)?)
}

fn service_control_summary_candidate(value: &serde_json::Value) -> Option<String> {
    value
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceControlRuntimeState {
    Running,
    Stopped,
    Unknown,
}

fn service_control_runtime_state(value: &serde_json::Value) -> Option<ServiceControlRuntimeState> {
    let state = value
        .get("post_state")
        .or_else(|| value.get("pre_state"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_ascii_lowercase();
    if state == "running" || state.contains("=running") {
        return Some(ServiceControlRuntimeState::Running);
    }
    if state == "stopped" || state.contains("=stopped") {
        return Some(ServiceControlRuntimeState::Stopped);
    }
    if state == "unknown" || state.contains("=unknown") {
        return Some(ServiceControlRuntimeState::Unknown);
    }
    None
}

fn service_control_process_count(value: &serde_json::Value) -> Option<u64> {
    let evidences = value.get("key_evidence")?.as_array()?;
    for evidence in evidences.iter().filter_map(|v| v.as_str()) {
        let Some((_, rest)) = evidence.split_once("process_count=") else {
            continue;
        };
        let digits = rest
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if digits.is_empty() {
            continue;
        }
        if let Ok(count) = digits.parse::<u64>() {
            return Some(count);
        }
    }
    None
}

fn system_basic_info_summary_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    if !matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    ) {
        return None;
    }
    let hostname = value
        .get("hostname")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let os = value
        .get("os")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let arch = value
        .get("arch")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    Some(match arch {
        Some(arch) => observed_t_with_vars(
            state,
            "clawd.msg.system_basic_info_brief_with_arch",
            "主机名 {hostname}，操作系统 {os}（{arch}）。",
            "Hostname: {hostname}; system: {os} ({arch}).",
            prefer_english,
            &[("hostname", hostname), ("os", os), ("arch", arch)],
        ),
        None => observed_t_with_vars(
            state,
            "clawd.msg.system_basic_info_brief",
            "主机名 {hostname}，操作系统 {os}。",
            "Hostname: {hostname}; system: {os}.",
            prefer_english,
            &[("hostname", hostname), ("os", os)],
        ),
    })
}

fn system_basic_value_looks_like_info(value: &serde_json::Value) -> bool {
    value
        .get("hostname")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some()
        && value
            .get("os")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .is_some()
}

fn system_basic_info_value(skill: &str, body: &str) -> Option<serde_json::Value> {
    if skill != "system_basic" {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    system_basic_value_looks_like_info(&value).then_some(value)
}

fn system_basic_existence_with_path_value(skill: &str, body: &str) -> Option<serde_json::Value> {
    if skill != "system_basic" {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    matches!(
        value.get("action").and_then(|v| v.as_str()),
        Some("find_path" | "path_batch_facts" | "find_name")
    )
    .then_some(value)
}

fn service_control_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    let requested_action = value
        .get("requested_action")
        .or_else(|| value.get("action"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_ascii_lowercase();
    if !matches!(requested_action.as_str(), "status" | "verify") {
        return service_control_summary_candidate(value);
    }

    let service_name = value
        .get("service_name")
        .or_else(|| value.get("target"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;

    let failure_reason = value
        .get("failure_reason")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if let Some(reason) = failure_reason {
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.service_status_check_failed",
            "检查 {service_name} 状态失败：{reason}",
            "{service_name} status check failed: {reason}.",
            prefer_english,
            &[("service_name", service_name), ("reason", reason)],
        ));
    }

    let runtime_state = service_control_runtime_state(value)?;
    let process_count = service_control_process_count(value);
    Some(match (runtime_state, process_count) {
        (ServiceControlRuntimeState::Running, Some(count)) => {
            let count_text = count.to_string();
            observed_t_with_vars(
                state,
                "clawd.msg.service_status_running_with_count",
                "{service_name} 当前正在运行，检查到进程数为 {count}。",
                "{service_name} is running; process count is {count}.",
                prefer_english,
                &[("service_name", service_name), ("count", &count_text)],
            )
        }
        (ServiceControlRuntimeState::Running, None) => observed_t_with_vars(
            state,
            "clawd.msg.service_status_running",
            "{service_name} 当前正在运行。",
            "{service_name} is running.",
            prefer_english,
            &[("service_name", service_name)],
        ),
        (ServiceControlRuntimeState::Stopped, Some(count)) => {
            let count_text = count.to_string();
            observed_t_with_vars(
                state,
                "clawd.msg.service_status_stopped_with_count",
                "{service_name} 当前没有运行，检查到进程数为 {count}。",
                "{service_name} is not running; process count is {count}.",
                prefer_english,
                &[("service_name", service_name), ("count", &count_text)],
            )
        }
        (ServiceControlRuntimeState::Stopped, None) => observed_t_with_vars(
            state,
            "clawd.msg.service_status_stopped",
            "{service_name} 当前没有运行。",
            "{service_name} is not running.",
            prefer_english,
            &[("service_name", service_name)],
        ),
        (ServiceControlRuntimeState::Unknown, _) => observed_t_with_vars(
            state,
            "clawd.msg.service_status_unknown",
            "{service_name} 当前状态未知。",
            "{service_name} status is unknown.",
            prefer_english,
            &[("service_name", service_name)],
        ),
    })
}

fn inventory_dir_names(value: &serde_json::Value) -> Option<Vec<String>> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if action != "inventory_dir" {
        return None;
    }
    value
        .get("names")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
}

fn inventory_dir_direct_answer_candidate(
    value: &serde_json::Value,
    max_entries: Option<usize>,
) -> Option<String> {
    let names = inventory_dir_names(value)?;
    trim_listing_to_max_entries(&names.join("\n"), max_entries)
}

fn inventory_dir_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let names = inventory_dir_names(value)?;
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    Some(format!(
        "inventory_dir path={path}\n- {}",
        names.join("\n- ")
    ))
}

fn is_ignorable_shell_warning(line: &str) -> bool {
    let normalized = line.trim();
    normalized.starts_with("bash: warning: setlocale:")
        || normalized.starts_with("zsh: warning: setlocale:")
}

fn run_cmd_directory_entry_list_candidate(
    body: &str,
    auto_locator_path: Option<&str>,
    max_entries: Option<usize>,
) -> Option<String> {
    let auto_locator_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(Path::new)?;
    if !auto_locator_path.is_dir() {
        return None;
    }
    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .collect::<Vec<_>>();
    if lines.is_empty()
        || lines.len() > 200
        || lines
            .iter()
            .any(|line| looks_like_shell_long_listing_line(line))
    {
        return None;
    }
    let all_direct_entries = lines.iter().all(|line| {
        let candidate = line.trim_end_matches('/');
        !candidate.is_empty()
            && !candidate.starts_with('/')
            && !candidate.starts_with('~')
            && !candidate.contains('/')
            && !candidate.contains('\\')
            && serde_json::from_str::<serde_json::Value>(candidate).is_err()
    });
    all_direct_entries
        .then(|| trim_listing_to_max_entries(&lines.join("\n"), max_entries))
        .flatten()
}

fn run_cmd_listing_text_candidate(
    body: &str,
    auto_locator_path: Option<&str>,
    max_entries: Option<usize>,
) -> Option<String> {
    run_cmd_shell_listing_entry_names(body, max_entries)
        .map(|names| names.join("\n"))
        .or_else(|| run_cmd_directory_entry_list_candidate(body, auto_locator_path, max_entries))
}

fn run_cmd_shell_listing_entry_names(
    body: &str,
    max_entries: Option<usize>,
) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for line in body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
    {
        if line.starts_with("total ") {
            continue;
        }
        let first = line.chars().next()?;
        if !matches!(first, '-' | 'd' | 'l' | 'b' | 'c' | 'p' | 's') {
            return None;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 9 {
            return None;
        }
        let raw_name = fields[8..].join(" ");
        let name = raw_name
            .split_once(" -> ")
            .map(|(name, _)| name)
            .unwrap_or(raw_name.as_str())
            .trim();
        if name.is_empty() {
            return None;
        }
        names.push(name.to_string());
    }
    if names.is_empty() {
        return None;
    }
    if let Some(limit) = max_entries.filter(|limit| *limit > 0) {
        names.truncate(limit);
    }
    Some(names)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecentArtifactStyle {
    LogLike,
    FormalDeliverable,
    WorkingArtifact,
}

fn recent_artifact_style_for_name(name: &str) -> (i32, i32) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return (0, 0);
    }
    let lower = trimmed.to_ascii_lowercase();
    let ext = lower
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default();

    let mut log_like = 0;
    let mut formal = 0;

    match ext {
        "log" | "trace" | "out" | "err" => log_like += 4,
        "md" | "pdf" | "doc" | "docx" | "ppt" | "pptx" | "html" | "htm" => formal += 4,
        "txt" => formal += 1,
        _ => {}
    }

    for marker in ["log", "trace", "stderr", "stdout", "runtime", "journal"] {
        if lower.contains(marker) {
            log_like += 2;
        }
    }
    for marker in [
        "readme",
        "guide",
        "manual",
        "checklist",
        "proposal",
        "summary",
        "report",
        "spec",
        "design",
        "plan",
    ] {
        if lower.contains(marker) {
            formal += 2;
        }
    }

    (log_like, formal)
}

fn classify_recent_artifact_style(names: &[String]) -> RecentArtifactStyle {
    let (log_like, formal) = names.iter().fold((0, 0), |(log_acc, formal_acc), name| {
        let (log_score, formal_score) = recent_artifact_style_for_name(name);
        (log_acc + log_score, formal_acc + formal_score)
    });
    if log_like >= formal + 2 {
        RecentArtifactStyle::LogLike
    } else if formal >= log_like + 2 {
        RecentArtifactStyle::FormalDeliverable
    } else {
        RecentArtifactStyle::WorkingArtifact
    }
}

fn recent_artifacts_judgment_direct_answer(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    if !route_requests_recent_artifacts_judgment(route) {
        return None;
    }
    let names = extract_latest_generic_successful_output(loop_state).and_then(|observed| {
        (observed.skill == "run_cmd")
            .then(|| {
                run_cmd_shell_listing_entry_names(
                    &observed.body,
                    requested_listing_limit(Some(route)),
                )
            })
            .flatten()
    })?;
    if names.is_empty() {
        return None;
    }
    let names_text = if prefer_english {
        names.join(", ")
    } else {
        names.join("、")
    };
    let answer = match classify_recent_artifact_style(&names) {
        RecentArtifactStyle::LogLike => observed_t_with_vars(
            state,
            "clawd.msg.recent_artifacts_judgment_log_like",
            "最近修改的文件是：{names}；这些更像运行或测试过程中产生的日志，不像正式交付产物。",
            "The most recently modified files are {names}; these look more like runtime or test logs than formal deliverables.",
            prefer_english,
            &[("names", names_text.as_str())],
        ),
        RecentArtifactStyle::FormalDeliverable => observed_t_with_vars(
            state,
            "clawd.msg.recent_artifacts_judgment_formal",
            "最近修改的文件是：{names}；这些更像整理好的正式文档或交付产物，不太像日志。",
            "The most recently modified files are {names}; these look more like prepared documents or formal deliverables than logs.",
            prefer_english,
            &[("names", names_text.as_str())],
        ),
        RecentArtifactStyle::WorkingArtifact => observed_t_with_vars(
            state,
            "clawd.msg.recent_artifacts_judgment_working",
            "最近修改的文件是：{names}；这些更像工作过程中的中间产物或运行痕迹，暂时不像正式交付产物。",
            "The most recently modified files are {names}; these look more like working/runtime artifacts than formal deliverables.",
            prefer_english,
            &[("names", names_text.as_str())],
        ),
    };
    Some(answer)
}

fn route_prefers_archive_success_answer(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Free
        )
}

fn archive_destination_from_locator_hint(locator_hint: &str) -> Option<&str> {
    let trimmed = locator_hint.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .rsplit_once("->")
        .map(|(_, dst)| dst.trim())
        .filter(|dst| !dst.is_empty())
        .or_else(|| (!trimmed.is_empty()).then_some(trimmed))
}

fn normalized_output_path_hint(state: Option<&AppState>, raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let Some(state) = state else {
        return trimmed.to_string();
    };
    let effective = crate::ensure_default_file_path(&state.skill_rt.workspace_root, trimmed);
    let path = Path::new(&effective);
    if path.is_absolute() {
        return canonical_existing_path(path);
    }
    let joined = state.skill_rt.workspace_root.join(path);
    canonical_existing_path(&joined)
}

fn archive_basic_output_destination(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let action = value.get("action").and_then(|v| v.as_str())?;
    if action != "pack" {
        return None;
    }
    value
        .get("archive")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

fn archive_basic_output_succeeded(body: &str) -> bool {
    if body
        .lines()
        .map(str::trim)
        .any(|line| line == "exit=0" || line.eq_ignore_ascii_case("ok"))
    {
        return true;
    }
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("output")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .is_some_and(|output| {
            output
                .lines()
                .map(str::trim)
                .any(|line| line == "exit=0" || line.eq_ignore_ascii_case("ok"))
        })
}

fn archive_basic_success_direct_answer_candidate(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    if !route_prefers_archive_success_answer(route) {
        return None;
    }
    if !archive_basic_output_succeeded(body) {
        return None;
    }
    let destination = archive_basic_output_destination(body).or_else(|| {
        archive_destination_from_locator_hint(&route.output_contract.locator_hint)
            .map(str::to_string)
    })?;
    let destination = normalized_output_path_hint(state, &destination);
    Some(observed_t_with_vars(
        state,
        "clawd.msg.archive_created_successfully",
        "已成功打包到 {path}。",
        "Archive created successfully at {path}.",
        prefer_english,
        &[("path", destination.as_str())],
    ))
}

fn run_cmd_presence_with_path_candidate(
    state: Option<&AppState>,
    body: &str,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    english_answer: bool,
) -> Option<String> {
    let scalar = normalized_scalar_candidate(body)?;
    let normalized = scalar.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "exists" | "yes" | "true" => Some(candidate_exists_with_path_text(
            state,
            existence_with_path_target_hint(locator_hint, auto_locator_path).as_deref(),
            english_answer,
        )),
        "not_found" | "not found" | "no" | "false" => {
            Some(candidate_not_found_text(state, english_answer))
        }
        _ => None,
    }
}

fn existence_with_path_target_hint(
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let locator_hint = locator_hint
        .map(str::trim)
        .filter(|hint| !hint.is_empty())?;
    let locator_path = Path::new(locator_hint);
    if locator_path.is_absolute() && locator_path.exists() {
        return Some(canonical_existing_path(locator_path));
    }
    resolve_listing_entry_full_path(locator_hint, auto_locator_path).or_else(|| {
        auto_locator_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .and_then(|root| {
                let root = Path::new(root);
                if !root.is_dir() {
                    return None;
                }
                let candidate = root.join(locator_hint);
                candidate
                    .exists()
                    .then(|| canonical_existing_path(&candidate))
            })
    })
}

fn candidate_exists_with_path_text(
    state: Option<&AppState>,
    path: Option<&str>,
    prefer_english: bool,
) -> String {
    match path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => observed_t_with_vars(
            state,
            "clawd.msg.exists_with_path",
            "有，路径：{path}",
            "yes, path: {path}",
            prefer_english,
            &[("path", path)],
        ),
        None => observed_t(state, "clawd.msg.exists_yes", "有", "yes", prefer_english),
    }
}

fn candidate_not_found_text(state: Option<&AppState>, prefer_english: bool) -> String {
    observed_t(state, "clawd.msg.exists_no", "没有", "no", prefer_english)
}

fn normalize_system_basic_match_path(
    resolved_root: Option<&str>,
    candidate_path: Option<&str>,
) -> Option<String> {
    let candidate_path = candidate_path
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let candidate = Path::new(candidate_path);
    if candidate.is_absolute() {
        return Some(candidate_path.to_string());
    }
    let root = resolved_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(Path::new)?;
    Some(root.join(candidate).to_string_lossy().to_string())
}

fn system_basic_existence_with_path_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
        "find_name" => {
            let (results, count, pattern) = fs_search_find_name_results(value)?;
            if count == 0 || results.is_empty() {
                return Some(candidate_not_found_text(state, prefer_english));
            }
            let preferred = if results.len() == 1 {
                Some(results[0].clone())
            } else {
                let pattern = normalized_find_name_pattern(pattern.as_deref())
                    .or_else(|| normalized_find_name_pattern(locator_hint))?;
                preferred_fs_search_exact_match(&results, &pattern)
            }?;
            let root = value
                .get("root")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|root| !root.is_empty());
            let resolved_path = Path::new(&preferred)
                .is_absolute()
                .then(|| canonical_existing_path(Path::new(&preferred)))
                .or_else(|| {
                    root.and_then(|root| {
                        let candidate = Path::new(root).join(&preferred);
                        candidate
                            .exists()
                            .then(|| canonical_existing_path(&candidate))
                    })
                })
                .or_else(|| resolve_listing_entry_full_path(&preferred, auto_locator_path))
                .unwrap_or(preferred);
            Some(candidate_exists_with_path_text(
                state,
                Some(resolved_path.as_str()),
                prefer_english,
            ))
        }
        "find_path" => {
            let count = value
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or_default() as usize;
            let matches = value.get("matches").and_then(|v| v.as_array())?;
            if count == 0 || matches.is_empty() {
                return Some(candidate_not_found_text(state, prefer_english));
            }
            if matches.len() != 1 {
                return None;
            }
            let matched = matches.first()?.as_object()?;
            let resolved_root = value.get("resolved_root").and_then(|v| v.as_str());
            let path = normalize_system_basic_match_path(
                resolved_root,
                matched
                    .get("resolved_path")
                    .and_then(|v| v.as_str())
                    .or_else(|| matched.get("path").and_then(|v| v.as_str())),
            );
            Some(candidate_exists_with_path_text(
                state,
                path.as_deref(),
                prefer_english,
            ))
        }
        "path_batch_facts" => {
            let facts = value.get("facts").and_then(|v| v.as_array())?;
            if facts.is_empty() {
                return Some(candidate_not_found_text(state, prefer_english));
            }
            if facts.len() != 1 {
                return None;
            }
            let entry = facts.first()?.as_object()?;
            let exists = entry
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !exists {
                return Some(candidate_not_found_text(state, prefer_english));
            }
            let path = entry
                .get("fact")
                .and_then(|v| v.as_object())
                .and_then(|fact| {
                    fact.get("resolved_path")
                        .and_then(|v| v.as_str())
                        .or_else(|| fact.get("path").and_then(|v| v.as_str()))
                })
                .or_else(|| entry.get("path").and_then(|v| v.as_str()));
            Some(candidate_exists_with_path_text(state, path, prefer_english))
        }
        _ => None,
    }
}

fn fs_search_find_name_results(
    value: &serde_json::Value,
) -> Option<(Vec<String>, usize, Option<String>)> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if !action.eq_ignore_ascii_case("find_name") {
        return None;
    }
    let results = value
        .get("results")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = value
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(results.len() as u64) as usize;
    let pattern = value
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);
    Some((results, count, pattern))
}

fn path_matches_find_name_pattern(path: &str, pattern: &str) -> bool {
    let path = Path::new(path);
    let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    if file_name.eq_ignore_ascii_case(pattern) {
        return true;
    }
    if pattern.contains('.') {
        return false;
    }
    path.file_stem()
        .and_then(|v| v.to_str())
        .map(|stem| stem.eq_ignore_ascii_case(pattern))
        .unwrap_or(false)
}

fn is_direct_child_relative_match(path: &str) -> bool {
    let path = Path::new(path);
    match path.parent().and_then(|parent| parent.to_str()) {
        None => true,
        Some("") | Some(".") => true,
        Some(_) => false,
    }
}

fn preferred_fs_search_exact_match(results: &[String], pattern: &str) -> Option<String> {
    let mut exact_matches = results
        .iter()
        .filter(|path| path_matches_find_name_pattern(path, pattern))
        .cloned()
        .collect::<Vec<_>>();
    exact_matches.sort();
    exact_matches.dedup();
    let mut direct_child_matches = exact_matches
        .iter()
        .filter(|path| is_direct_child_relative_match(path))
        .cloned()
        .collect::<Vec<_>>();
    direct_child_matches.sort();
    direct_child_matches.dedup();
    if direct_child_matches.len() == 1 {
        return direct_child_matches.into_iter().next();
    }
    (exact_matches.len() == 1).then(|| exact_matches.into_iter().next().unwrap_or_default())
}

fn rank_fs_search_candidates(results: &[String], pattern: &str) -> Vec<String> {
    let pattern_norm = pattern.trim().to_lowercase();
    let mut ranked = results
        .iter()
        .cloned()
        .map(|path| {
            let path_buf = Path::new(&path);
            let file_name = path_buf
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_string();
            let file_name_norm = file_name.to_lowercase();
            let stem_norm = path_buf
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_lowercase();
            let score = if stem_norm == pattern_norm {
                500
            } else if stem_norm.starts_with(&pattern_norm) {
                400
            } else if stem_norm.contains(&pattern_norm) {
                300
            } else if file_name_norm.starts_with(&pattern_norm) {
                200
            } else if file_name_norm.contains(&pattern_norm) {
                100
            } else {
                0
            };
            (score, file_name.len(), path)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    ranked.dedup_by(|a, b| a.2 == b.2);
    ranked
        .into_iter()
        .take(3)
        .map(|(_, _, path)| path)
        .collect()
}

fn normalized_find_name_pattern(pattern: Option<&str>) -> Option<String> {
    let pattern = pattern?.trim();
    if pattern.is_empty() {
        return None;
    }
    let path = Path::new(pattern);
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some(pattern.to_string()))
}

fn fs_search_scalar_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    let (results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.fs_search_no_match",
            "没有找到匹配项",
            "No matches found.",
            prefer_english,
        ));
    }
    if results.len() == 1 {
        return Some(results[0].clone());
    }
    let pattern = normalized_find_name_pattern(pattern.as_deref())
        .or_else(|| normalized_find_name_pattern(locator_hint))?;
    preferred_fs_search_exact_match(&results, &pattern)
}

fn fs_search_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    let (results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.fs_search_no_match",
            "没有找到匹配项",
            "No matches found.",
            prefer_english,
        ));
    }
    if results.len() == 1 {
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.exists_with_path",
            "有，路径：{path}",
            "yes, path: {path}",
            prefer_english,
            &[("path", &results[0])],
        ));
    }
    if let Some(pattern) = normalized_find_name_pattern(pattern.as_deref())
        .or_else(|| normalized_find_name_pattern(locator_hint))
    {
        if let Some(preferred) = preferred_fs_search_exact_match(&results, &pattern) {
            return Some(observed_t_with_vars(
                state,
                "clawd.msg.exists_with_path",
                "有，路径：{path}",
                "yes, path: {path}",
                prefer_english,
                &[("path", &preferred)],
            ));
        }
        let ranked = rank_fs_search_candidates(&results, &pattern);
        if !ranked.is_empty() {
            let count_text = ranked.len().to_string();
            let separator = if prefer_english { "; " } else { "；" };
            let candidates = ranked.join(separator);
            return Some(observed_t_with_vars(
                state,
                "clawd.msg.fs_search_candidates",
                "我找到 {count} 个最接近的候选，请确认要哪一个：{candidates}",
                "I found {count} closest candidates. Please confirm which one you want: {candidates}",
                prefer_english,
                &[("count", &count_text), ("candidates", &candidates)],
            ));
        }
    }
    let count_text = results.len().min(count.max(results.len())).to_string();
    let separator = if prefer_english { "; " } else { "；" };
    let matches = results
        .into_iter()
        .take(3)
        .collect::<Vec<_>>()
        .join(separator);
    Some(observed_t_with_vars(
        state,
        "clawd.msg.fs_search_multiple",
        "找到 {count} 个匹配项：{matches}",
        "Found {count} matches: {matches}",
        prefer_english,
        &[("count", &count_text), ("matches", &matches)],
    ))
}

fn log_analyze_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    let display_path = value
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(if prefer_english { "log" } else { "日志" });
    let mut counts = value
        .get("keyword_counts")
        .and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_u64().map(|count| (k.as_str().to_string(), count)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let recent = value
        .get("recent_matches")
        .and_then(|v| v.as_array())
        .and_then(|items| items.first())
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);
    if counts.iter().all(|(_, count)| *count == 0) {
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.log_no_anomaly",
            "{display_path} 未发现明显异常。",
            "No obvious anomalies were found in {display_path}.",
            prefer_english,
            &[("display_path", display_path)],
        ));
    }
    let (keyword, count) = counts.into_iter().next()?;
    let count_text = count.to_string();
    let mut out = observed_t_with_vars(
        state,
        "clawd.msg.log_top_finding",
        "{display_path} 里最值得注意的是 {keyword}={count}。",
        "The most notable signal in {display_path} is {keyword}={count}.",
        prefer_english,
        &[
            ("display_path", display_path),
            ("keyword", &keyword),
            ("count", &count_text),
        ],
    );
    if let Some(line) = recent {
        out.push_str(&observed_t_with_vars(
            state,
            "clawd.msg.log_recent_line_suffix",
            " 最近一条相关记录：{line}",
            " Most recent related line: {line}",
            prefer_english,
            &[("line", &line)],
        ));
    }
    Some(out)
}

#[derive(Debug, Clone)]
struct ListeningPortEntry {
    process: String,
    port: u16,
    public: bool,
}

fn parse_listening_port_entry(line: &str) -> Option<ListeningPortEntry> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed == "exit=0"
        || trimmed.starts_with("COMMAND ")
        || !trimmed.contains(" TCP ")
        || !trimmed.contains("(LISTEN)")
    {
        return None;
    }
    let process = trimmed.split_whitespace().next()?.trim();
    if process.is_empty() {
        return None;
    }
    let tcp_part = trimmed.split(" TCP ").nth(1)?.trim();
    let endpoint = tcp_part.split_whitespace().next()?.trim();
    let port = endpoint.rsplit(':').next()?.trim().parse::<u16>().ok()?;
    let endpoint_lc = endpoint.to_ascii_lowercase();
    let public = endpoint.starts_with("*:")
        || (!endpoint_lc.starts_with("127.0.0.1:")
            && !endpoint_lc.starts_with("[::1]:")
            && !endpoint_lc.starts_with("localhost:"));
    Some(ListeningPortEntry {
        process: process.to_string(),
        port,
        public,
    })
}

fn process_basic_port_list_summary_candidate(
    state: Option<&AppState>,
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let entries = body
        .lines()
        .filter_map(parse_listening_port_entry)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }

    let mut grouped: std::collections::BTreeMap<String, (Vec<u16>, bool)> =
        std::collections::BTreeMap::new();
    for entry in entries {
        let slot = grouped
            .entry(entry.process)
            .or_insert_with(|| (Vec::new(), false));
        if !slot.0.contains(&entry.port) {
            slot.0.push(entry.port);
        }
        slot.1 |= entry.public;
    }

    let mut groups = grouped
        .into_iter()
        .map(|(process, (mut ports, public))| {
            ports.sort_unstable();
            let ports_text = ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join("/");
            let score = (if public { 100 } else { 0 })
                + if process.eq_ignore_ascii_case("clawd") {
                    30
                } else {
                    0
                }
                + if process.eq_ignore_ascii_case("nginx") {
                    20
                } else {
                    0
                }
                + if process.eq_ignore_ascii_case("ss-local") {
                    10
                } else {
                    0
                };
            (process, ports_text, public, score)
        })
        .collect::<Vec<_>>();
    groups.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));

    let highlights = groups
        .iter()
        .take(3)
        .map(|(process, ports, _, _)| format!("{process}({ports})"))
        .collect::<Vec<_>>();
    if highlights.is_empty() {
        return None;
    }
    let has_public = groups.iter().any(|(_, _, public, _)| *public);
    let highlights_text = highlights.join(if prefer_english { ", " } else { "、" });
    Some(if has_public {
        observed_t_with_vars(
            state,
            "clawd.msg.process_ports_public_summary",
            "最值得注意的是 {highlights}；其余大多是本地回环或辅助监听端口。",
            "The most notable listening ports are {highlights}; the rest are mostly loopback or auxiliary listeners.",
            prefer_english,
            &[("highlights", &highlights_text)],
        )
    } else {
        observed_t_with_vars(
            state,
            "clawd.msg.process_ports_local_summary",
            "当前主要看到这些监听端口：{highlights}。",
            "The main listening ports right now are {highlights}.",
            prefer_english,
            &[("highlights", &highlights_text)],
        )
    })
}

fn health_check_log_error_count(value: &serde_json::Value, key: &str) -> Option<u64> {
    value
        .get(key)?
        .get("keyword_error_count")
        .and_then(|v| v.as_u64())
}

fn health_check_os_display(value: &serde_json::Value) -> Option<&'static str> {
    match value
        .get("system_health")
        .and_then(|system| system.get("os_family"))
        .and_then(|v| v.as_str())?
    {
        "macos" => Some("macOS"),
        "linux" => Some("Linux"),
        _ => Some("host OS"),
    }
}

fn health_check_system_warning_text(
    state: Option<&AppState>,
    warning_code: &str,
    prefer_english: bool,
) -> Option<String> {
    match warning_code {
        "disk_root_low" => Some(observed_t(
            state,
            "clawd.msg.health_check_system_warning_disk_root_low",
            "根分区可用空间偏低。",
            "Root filesystem available space is low.",
            prefer_english,
        )),
        "memory_available_low" => Some(observed_t(
            state,
            "clawd.msg.health_check_system_warning_memory_available_low",
            "可用内存偏低。",
            "Available memory is low.",
            prefer_english,
        )),
        "load_high" => Some(observed_t(
            state,
            "clawd.msg.health_check_system_warning_load_high",
            "系统负载偏高。",
            "System load is high.",
            prefer_english,
        )),
        _ => None,
    }
}

fn health_check_system_summary_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    let os_display = health_check_os_display(value)?;
    let warnings = value
        .get("system_health")
        .and_then(|system| system.get("warnings"))
        .and_then(|warnings| warnings.as_array())?;
    if let Some(warning_text) = warnings
        .first()
        .and_then(|warning| warning.as_str())
        .and_then(|code| health_check_system_warning_text(state, code, prefer_english))
    {
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.health_check_system_warning",
            "{os_family} 宿主机有一项更值得先看的系统层告警：{warning}",
            "The {os_family} host has a system-level warning worth checking first: {warning}",
            prefer_english,
            &[("os_family", os_display), ("warning", &warning_text)],
        ));
    }
    Some(observed_t_with_vars(
        state,
        "clawd.msg.health_check_system_ok",
        "{os_family} 宿主机当前没有明显的系统层告警。",
        "The {os_family} host has no obvious system-level warning right now.",
        prefer_english,
        &[("os_family", os_display)],
    ))
}

fn health_check_field_extract_candidate(value: &serde_json::Value) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(count) = value.get("clawd_process_count").and_then(|v| v.as_u64()) {
        parts.push(format!("clawd_process_count={count}"));
    }
    if let Some(count) = value
        .get("telegramd_process_count")
        .and_then(|v| v.as_u64())
    {
        parts.push(format!("telegramd_process_count={count}"));
    }
    if let Some(open) = value
        .get("clawd_health_port_open")
        .and_then(|v| v.as_bool())
    {
        parts.push(format!("clawd_health_port_open={open}"));
    }
    if let Some(count) = health_check_log_error_count(value, "clawd_log") {
        parts.push(format!("clawd_log_errors={count}"));
    }
    if let Some(count) = health_check_log_error_count(value, "telegramd_log") {
        parts.push(format!("telegramd_log_errors={count}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn health_check_summary_candidate(
    state: Option<&AppState>,
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let rustclaw_fields = health_check_field_extract_candidate(&value)?;
    match health_check_system_summary_candidate(state, &value, prefer_english) {
        Some(system_summary) => Some(format!("{system_summary} {rustclaw_fields}")),
        None => Some(rustclaw_fields),
    }
}

fn git_basic_status_summary_candidate(
    state: Option<&AppState>,
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    let has_status_header = lines.iter().any(|line| line.starts_with("## "));
    if !has_status_header {
        return None;
    }
    let has_changes = lines.iter().any(|line| !line.starts_with("## "));
    if has_changes {
        Some(observed_t(
            state,
            "clawd.msg.git_has_changes",
            "当前仓库有未提交改动。",
            "The current repository has uncommitted changes.",
            prefer_english,
        ))
    } else {
        Some(observed_t(
            state,
            "clawd.msg.git_clean",
            "当前仓库没有未提交改动。",
            "The current repository has no uncommitted changes.",
            prefer_english,
        ))
    }
}

fn structured_scalar_candidate(
    state: Option<&AppState>,
    skill: &str,
    body: &str,
    locator_hint: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if skill == "db_basic" {
        return db_basic_scalar_candidate(&value);
    }
    if skill == "service_control" {
        return service_control_summary_candidate(&value);
    }
    if skill == "fs_search" {
        return fs_search_scalar_candidate(state, &value, locator_hint, prefer_english);
    }
    if skill == "process_basic" {
        return process_basic_port_list_summary_candidate(state, body, prefer_english);
    }
    if skill != "system_basic" {
        return None;
    }
    let action = value.get("action").and_then(|v| v.as_str())?;
    match action {
        "extract_field" => {
            let field_path = value
                .get("field_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("requested field");
            if value
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let text = value
                    .get("value_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
                return value.get("value").and_then(|v| match v {
                    serde_json::Value::Null => Some("null".to_string()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::String(s) => Some(s.clone()),
                    _ => None,
                });
            }
            Some(observed_t_with_vars(
                state,
                "clawd.msg.field_not_found",
                "{field_path} 字段不存在",
                "Field `{field_path}` does not exist.",
                prefer_english,
                &[("field_path", field_path)],
            ))
        }
        "count_inventory" => value
            .get("counts")
            .and_then(|v| v.get("total"))
            .and_then(value_scalar_text),
        _ => None,
    }
}

fn normalize_read_range_excerpt(excerpt: &str) -> Option<String> {
    let lines = excerpt
        .lines()
        .map(str::trim_end)
        .map(|line| {
            line.split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start().to_string())
                .unwrap_or_else(|| line.trim().to_string())
        })
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.iter().all(|line| line.is_empty()) {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn read_range_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let excerpt = value
        .get("excerpt")
        .and_then(|v| v.as_str())
        .and_then(normalize_read_range_excerpt)?;
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    Some(match path {
        Some(path) => format!("read_range path={path}\n{excerpt}"),
        None => excerpt,
    })
}

fn compact_log_analyze_excerpt(value: &serde_json::Value) -> Option<String> {
    let path = value
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let keyword_counts = value
        .get("keyword_counts")
        .and_then(|v| v.as_object())
        .map(|map| {
            let mut pairs = map
                .iter()
                .filter_map(|(key, count)| count.as_u64().map(|count| (key.as_str(), count)))
                .collect::<Vec<_>>();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            pairs
                .into_iter()
                .map(|(key, count)| format!("{key}={count}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let recent_matches = value
        .get("recent_matches")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(8)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let total_lines = value
        .get("total_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();

    let mut sections = vec![format!("log_analyze path={path} total_lines={total_lines}")];
    if !keyword_counts.is_empty() {
        sections.push(format!("keyword_counts: {}", keyword_counts.join(", ")));
    }
    if !recent_matches.is_empty() {
        sections.push(format!(
            "recent_matches:\n- {}",
            recent_matches.join("\n- ")
        ));
    }
    Some(sections.join("\n"))
}

fn archive_basic_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let action = value.get("action").and_then(|v| v.as_str())?.trim();
    if action.is_empty() {
        return None;
    }
    let archive = value
        .get("archive")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("-");
    let output = value
        .get("output")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("ok");
    Some(format!(
        "archive_basic action={action} archive={archive}\n{output}"
    ))
}

fn structured_observed_body(skill: &str, body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    match skill {
        "system_basic" => {
            let action = value.get("action").and_then(|v| v.as_str())?;
            match action {
                "read_range" => read_range_observed_candidate(&value),
                "inventory_dir" => inventory_dir_observed_candidate(&value),
                _ => None,
            }
        }
        "db_basic" => db_basic_scalar_candidate(&value).map(|text| format!("db_scalar={text}")),
        "service_control" => service_control_summary_candidate(&value),
        "fs_search" => fs_search_direct_answer_candidate(None, &value, None, false),
        "archive_basic" => archive_basic_observed_candidate(&value),
        "log_analyze" => compact_log_analyze_excerpt(&value),
        _ => None,
    }
}

fn extract_direct_scalar_from_generic_output_with_locator_hint_impl(
    state: Option<&AppState>,
    loop_state: &LoopState,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
    prefer_english: bool,
) -> Option<String> {
    if let Some(answer) = latest_successful_list_dir_answer_candidate(
        loop_state,
        Some(crate::OutputResponseShape::Scalar),
        None,
        auto_locator_path,
        prefer_full_path,
    ) {
        if !crate::finalize::looks_like_planner_artifact(&answer)
            && !crate::finalize::looks_like_internal_trace_artifact(&answer)
        {
            return Some(answer);
        }
    }
    let observed_output = extract_latest_generic_successful_output(loop_state)?;
    let answer = structured_scalar_candidate(
        state,
        &observed_output.skill,
        &observed_output.body,
        locator_hint.filter(|hint| !hint.trim().is_empty()),
        prefer_english,
    )
    .or_else(|| normalized_scalar_candidate(&observed_output.body))?;
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    Some(answer)
}

#[cfg(test)]
pub(crate) fn extract_direct_scalar_from_generic_output_with_locator_hint(
    loop_state: &LoopState,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
) -> Option<String> {
    extract_direct_scalar_from_generic_output_with_locator_hint_impl(
        None,
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
        false,
    )
}

pub(crate) fn extract_direct_scalar_from_generic_output(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let prefer_full_path = route.is_some_and(route_requests_scalar_path_only);
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if let Some(answer) = comparison_winner_from_route(route, loop_state) {
            return Some(answer);
        }
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
            return Some(answer);
        }
    }
    let locator_hint = route.map(|route| route.output_contract.locator_hint.as_str());
    extract_direct_scalar_from_generic_output_with_locator_hint_impl(
        None,
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
        false,
    )
}

pub(crate) fn extract_direct_scalar_from_generic_output_i18n(
    loop_state: &LoopState,
    state: &AppState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let prefer_full_path = route.is_some_and(route_requests_scalar_path_only);
    let prefer_english = route
        .map(|route| route.resolved_intent.trim())
        .filter(|intent| !intent.is_empty())
        .is_some_and(|intent| !text_contains_cjk(intent));
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if let Some(answer) = comparison_winner_from_route(route, loop_state) {
            return Some(answer);
        }
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
            return Some(answer);
        }
    }
    let locator_hint = route.map(|route| route.output_contract.locator_hint.as_str());
    extract_direct_scalar_from_generic_output_with_locator_hint_impl(
        Some(state),
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
        prefer_english,
    )
}

fn http_basic_status_code(body: &str) -> Option<u16> {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !is_ignorable_shell_warning(line))
        .and_then(|line| line.strip_prefix("status="))
        .map(str::trim)
        .and_then(|digits| digits.parse::<u16>().ok())
}

fn http_basic_summary_candidate(
    state: Option<&AppState>,
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    if !matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    ) {
        return None;
    }
    let status_code = http_basic_status_code(body)?;
    let status_code_text = status_code.to_string();
    Some(match status_code {
        200..=299 => observed_t_with_vars(
            state,
            "clawd.msg.http_request_succeeded",
            "请求成功，返回 HTTP {status_code}。",
            "Request succeeded with HTTP {status_code}.",
            prefer_english,
            &[("status_code", &status_code_text)],
        ),
        300..=399 => observed_t_with_vars(
            state,
            "clawd.msg.http_request_redirected",
            "请求已完成，但返回了 HTTP {status_code} 重定向。",
            "Request completed with HTTP {status_code} redirection.",
            prefer_english,
            &[("status_code", &status_code_text)],
        ),
        _ => observed_t_with_vars(
            state,
            "clawd.msg.http_request_returned",
            "请求返回 HTTP {status_code}。",
            "Request returned HTTP {status_code}.",
            prefer_english,
            &[("status_code", &status_code_text)],
        ),
    })
}

fn extract_direct_answer_from_generic_output_impl(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let response_shape = route.map(|route| route.output_contract.response_shape);
    let is_plain_act = route.is_some_and(|route| route.ask_mode.is_plain_act());
    let allow_raw_listing_direct_answer = route_allows_raw_listing_direct_answer(route);
    let requested_listing_limit = requested_listing_limit(route);
    let locator_hint = route
        .map(|route| route.output_contract.locator_hint.as_str())
        .filter(|hint| !hint.trim().is_empty());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let request_language_hint = agent_run_context
        .and_then(|ctx| ctx.user_request.as_deref())
        .map(observed_request_language_hint)
        .or_else(|| {
            route.map(|route| observed_request_language_hint(route.resolved_intent.as_str()))
        })
        .unwrap_or("config_default");
    let prefers_english_free_text = request_language_hint == "en";
    let prefers_english_presence_answer = route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
            && prefers_english_free_text
    });
    let health_check_prefers_raw_payload = is_plain_act
        && !matches!(
            response_shape,
            Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
        )
        && !health_check_request_prefers_summary(route, agent_run_context);
    if health_check_request_prefers_summary(route, agent_run_context) {
        if let Some(observed_output) =
            extract_latest_successful_output_for_skill(loop_state, "health_check")
        {
            if let Some(answer) = health_check_summary_candidate(
                state,
                &observed_output.body,
                prefers_english_free_text,
            ) {
                return Some(answer);
            }
        }
    }
    let prefer_full_path = route.is_some_and(route_requests_scalar_path_only);

    if let Some(route) = route {
        if let Some(answer) =
            hidden_entries_direct_answer(state, route, loop_state, prefers_english_free_text)
        {
            return Some(answer);
        }
        if let Some(answer) = directory_purpose_summary_direct_answer(
            state,
            route,
            loop_state,
            auto_locator_path,
            requested_listing_limit,
            prefers_english_free_text,
        ) {
            return Some(answer);
        }
        if let Some(answer) = workspace_project_summary_direct_answer(
            state,
            route,
            loop_state,
            auto_locator_path,
            prefers_english_free_text,
        ) {
            return Some(answer);
        }
        if let Some(answer) = recent_artifacts_judgment_direct_answer(
            state,
            route,
            loop_state,
            prefers_english_free_text,
        ) {
            return Some(answer);
        }
    }

    let answer = allow_raw_listing_direct_answer
        .then(|| {
            latest_successful_list_dir_answer_candidate(
                loop_state,
                response_shape,
                requested_listing_limit,
                auto_locator_path,
                prefer_full_path,
            )
        })
        .flatten()
        .or_else(|| {
            let observed_output = extract_latest_generic_successful_output(loop_state)?;
            if observed_output.skill == "run_cmd" {
                run_cmd_presence_with_path_candidate(
                    state,
                    &observed_output.body,
                    locator_hint,
                    auto_locator_path,
                    prefers_english_presence_answer,
                )
                .or_else(|| {
                    allow_raw_listing_direct_answer
                        .then(|| {
                            run_cmd_listing_text_candidate(
                                &observed_output.body,
                                auto_locator_path,
                                requested_listing_limit,
                            )
                        })
                        .flatten()
                })
            } else {
                None
            }
            .or_else(|| match observed_output.skill.as_str() {
                "health_check" => health_check_prefers_raw_payload
                    .then_some(observed_output.body.clone())
                    .or_else(|| {
                        health_check_summary_candidate(
                            state,
                            &observed_output.body,
                            prefers_english_free_text,
                        )
                    }),
                "http_basic" => http_basic_summary_candidate(
                    state,
                    &observed_output.body,
                    response_shape,
                    prefers_english_free_text,
                ),
                "process_basic" => process_basic_port_list_summary_candidate(
                    state,
                    &observed_output.body,
                    prefers_english_free_text,
                ),
                "service_control" => {
                    serde_json::from_str::<serde_json::Value>(&observed_output.body)
                        .ok()
                        .and_then(|value| {
                            service_control_direct_answer_candidate(
                                state,
                                &value,
                                prefers_english_free_text,
                            )
                        })
                }
                "fs_search" => serde_json::from_str::<serde_json::Value>(&observed_output.body)
                    .ok()
                    .and_then(|value| {
                        fs_search_direct_answer_candidate(
                            state,
                            &value,
                            locator_hint,
                            prefers_english_free_text,
                        )
                    }),
                "git_basic" => git_basic_status_summary_candidate(
                    state,
                    &observed_output.body,
                    prefers_english_free_text,
                ),
                "archive_basic" => route.and_then(|route| {
                    archive_basic_success_direct_answer_candidate(
                        state,
                        route,
                        &observed_output.body,
                        prefers_english_free_text,
                    )
                }),
                "log_analyze" => serde_json::from_str::<serde_json::Value>(&observed_output.body)
                    .ok()
                    .and_then(|value| {
                        log_analyze_direct_answer_candidate(
                            state,
                            &value,
                            prefers_english_free_text,
                        )
                    }),
                "system_basic" => {
                    let value = serde_json::from_str::<serde_json::Value>(&observed_output.body)
                        .ok()
                        .or_else(|| {
                            system_basic_info_value("system_basic", &observed_output.body)
                        })?;
                    let action = value.get("action").and_then(|v| v.as_str());
                    if action == Some("read_range")
                        && is_plain_act
                        && !matches!(
                            response_shape,
                            Some(crate::OutputResponseShape::OneSentence)
                        )
                    {
                        value
                            .get("excerpt")
                            .and_then(|v| v.as_str())
                            .and_then(normalize_read_range_excerpt)
                    } else if action == Some("inventory_dir")
                        && is_plain_act
                        && allow_raw_listing_direct_answer
                    {
                        inventory_dir_direct_answer_candidate(&value, requested_listing_limit)
                    } else if action == Some("info")
                        || (action.is_none() && system_basic_value_looks_like_info(&value))
                    {
                        system_basic_info_summary_candidate(
                            state,
                            &value,
                            response_shape,
                            prefers_english_free_text,
                        )
                    } else if route.is_some_and(|route| {
                        route.output_contract.semantic_kind
                            == crate::OutputSemanticKind::ExistenceWithPath
                    }) {
                        system_basic_existence_with_path_candidate(
                            state,
                            &value,
                            locator_hint,
                            auto_locator_path,
                            prefers_english_presence_answer,
                        )
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .or_else(|| {
                structured_scalar_candidate(
                    state,
                    &observed_output.skill,
                    &observed_output.body,
                    locator_hint,
                    prefers_english_free_text,
                )
            })
            .or_else(|| normalized_scalar_candidate(&observed_output.body))
        })?;
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    Some(answer)
}

pub(crate) fn extract_direct_answer_from_generic_output(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    extract_direct_answer_from_generic_output_impl(None, loop_state, agent_run_context)
}

pub(crate) fn extract_direct_answer_from_generic_output_i18n(
    loop_state: &LoopState,
    state: &AppState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    extract_direct_answer_from_generic_output_impl(Some(state), loop_state, agent_run_context)
}

fn observed_step_body(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let body = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    if let Some(normalized) = structured_observed_body(&step.skill, body) {
        return Some(normalized);
    }
    (crate::finalize::classify_observed_content_status(body)
        == crate::finalize::ObservedContentStatus::ContentAvailable)
        .then(|| body.to_string())
}

fn observed_step_entry(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let output = observed_step_body(step)?;
    if crate::finalize::looks_like_planner_artifact(&output)
        || crate::finalize::looks_like_internal_trace_artifact(&output)
    {
        return None;
    }
    Some(format!(
        "### {} skill({})\n{}",
        step.step_id,
        step.skill,
        trim_for_observed_prompt(&output, 1800)
    ))
}

fn observed_output_entries(loop_state: &LoopState) -> Vec<String> {
    let latest_listing_idx = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rfind(|(_, step)| {
            step.is_ok() && step.skill == "list_dir" && observed_step_entry(step).is_some()
        })
        .map(|(idx, _)| idx);
    let mut selected_indices = latest_listing_idx.into_iter().collect::<Vec<_>>();
    let mut recent_non_listing = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            step.is_ok() && step.skill != "list_dir" && observed_step_entry(step).is_some()
        })
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    if recent_non_listing.len() > 4 {
        recent_non_listing = recent_non_listing.split_off(recent_non_listing.len() - 4);
    }
    selected_indices.extend(recent_non_listing);
    selected_indices.sort_unstable();
    selected_indices.dedup();
    selected_indices
        .into_iter()
        .filter_map(|idx| observed_step_entry(&loop_state.executed_step_results[idx]))
        .collect()
}

pub(crate) fn has_observed_answer_candidates(loop_state: &LoopState) -> bool {
    !observed_output_entries(loop_state).is_empty()
}

fn observed_contract_json(agent_run_context: Option<&AgentRunContext>) -> String {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return "{}".to_string();
    };
    serde_json::json!({
        "routed_mode": route.routed_mode.as_str(),
        "response_shape": route.output_contract.response_shape.as_str(),
        "requires_content_evidence": route.output_contract.requires_content_evidence,
        "delivery_required": route.output_contract.delivery_required,
        "locator_kind": route.output_contract.locator_kind.as_str(),
        "delivery_intent": route.output_contract.delivery_intent.as_str(),
        "semantic_kind": route.output_contract.semantic_kind.as_str(),
        "locator_hint": route.output_contract.locator_hint,
        "needs_clarify": route.needs_clarify,
    })
    .to_string()
}

fn resolved_user_intent(agent_run_context: Option<&AgentRunContext>, user_text: &str) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.resolved_intent.trim())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| user_text.trim())
        .to_string()
}

pub(crate) async fn synthesize_answer_from_observed_output(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let observed_entries = observed_output_entries(loop_state);
    if observed_entries.is_empty() {
        return None;
    }
    let observed_block = observed_entries.join("\n\n");
    let resolved_intent = resolved_user_intent(agent_run_context, user_text);
    let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
        state,
        OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH,
        OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_REQUEST__", user_text.trim()),
            ("__RESOLVED_USER_INTENT__", &resolved_intent),
            (
                "__OUTPUT_CONTRACT__",
                &observed_contract_json(agent_run_context),
            ),
            ("__OBSERVED_OUTPUTS__", &observed_block),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            (
                "__REQUEST_LANGUAGE_HINT__",
                observed_request_language_hint(user_text),
            ),
            (
                "__RESPONSE_STYLE_HINT__",
                observed_response_style_hint(agent_run_context),
            ),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "observed_answer_fallback_prompt",
        &prompt_source,
        None,
    );
    let llm_out =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await
            .ok()?;
    let parsed_raw = serde_json::from_str::<ObservedAnswerFallbackOut>(llm_out.trim()).ok();
    let parsed = parsed_raw.or_else(|| {
        crate::extract_first_json_object_any(&llm_out)
            .and_then(|json| serde_json::from_str::<ObservedAnswerFallbackOut>(&json).ok())
    })?;
    let answer = parsed.answer.trim().to_string();
    // §3.4 finalize-tier: 这里属于 observed_answer_fallback 兜底路径（finalize 层
    // 的 fallback 分支），是 semantic_judge LLM 入口的允许调用方之一。
    // Phase 0.2: 复用同一次 LLM 调用已经返回的 `publishable` + `is_meta_instruction`，
    // 高置信度时直接信任，避免再发一次 `semantic_judge::is_meta_respond_instruction`
    // 二次判定调用。低置信度（<0.55）时才回退到 semantic_judge 做安全兜底，
    // 保留"LLM 过保守错判为不可发"的救回链路。
    const OBSERVED_SELF_CLASSIFY_CONF_THRESHOLD: f64 = 0.55;
    let semantically_publishable = if !answer.is_empty() && !parsed.needs_clarify {
        if parsed.confidence >= OBSERVED_SELF_CLASSIFY_CONF_THRESHOLD {
            parsed.publishable && !parsed.is_meta_instruction
        } else if parsed.publishable {
            !parsed.is_meta_instruction
        } else {
            !crate::semantic_judge::is_meta_respond_instruction(state, task, &answer).await
        }
    } else {
        false
    };
    let qualified = !answer.is_empty()
        && !parsed.needs_clarify
        && (parsed.qualified || semantically_publishable);
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(if qualified {
                crate::finalize::FinalizerDisposition::QualifiedCompletion
            } else {
                crate::finalize::FinalizerDisposition::AllowFallback
            }),
            parsed: true,
            contract_ok: qualified,
            completion_ok: Some(qualified),
            grounded_ok: Some(qualified),
            format_ok: Some(qualified),
            needs_clarify: Some(parsed.needs_clarify),
            confidence: Some(parsed.confidence.clamp(0.0, 1.0)),
            used_evidence_ids_count: observed_entries.len(),
            evidence_quotes_count: 0,
            ..Default::default()
        },
    ))
}

pub(crate) fn normalized_observed_listing(
    observed: &str,
    max_entries: Option<usize>,
) -> Option<String> {
    trim_listing_to_max_entries(observed, max_entries)
}

#[cfg(test)]
mod tests {
    use super::super::LoopState;
    use super::{
        extract_direct_answer_from_generic_output, extract_direct_scalar_from_generic_output,
        extract_direct_scalar_from_generic_output_with_locator_hint, normalized_observed_listing,
        observed_contract_json, observed_output_entries, observed_request_language_hint,
        observed_response_style_hint, requested_listing_limit_from_intent, AgentRunContext,
        OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
    };
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    use crate::{
        IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
        OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
    };

    fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
        StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        }
    }

    #[test]
    fn direct_scalar_ignores_exit_zero_prefix() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "git_basic", "exit=0\nmain\n"));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("main")
        );
    }

    #[test]
    fn direct_scalar_ignores_shell_locale_warning_noise() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "/tmp/rustclaw-workspace\n\nbash: warning: setlocale: LC_ALL: cannot change locale (C.UTF-8): No such file or directory\n",
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("/tmp/rustclaw-workspace")
        );
    }

    #[test]
    fn direct_scalar_reads_extract_field_value_from_structured_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("rustclaw")
        );
    }

    #[test]
    fn direct_scalar_reports_missing_extract_field_as_field_absent() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("name 字段不存在")
        );
    }

    #[test]
    fn direct_scalar_reads_count_inventory_total_from_structured_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":12,"files":9,"dirs":3}}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("12")
        );
    }

    #[test]
    fn direct_scalar_prefers_unique_exact_fs_search_match_path() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"README.md","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("README.md")
        );
    }

    #[test]
    fn direct_scalar_uses_locator_hint_when_fs_search_output_omits_pattern() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output_with_locator_hint(
                &loop_state,
                Some("README.md"),
                None,
                false,
            )
            .as_deref(),
            Some("README.md")
        );
    }

    #[test]
    fn direct_scalar_does_not_collapse_ambiguous_fs_search_to_count() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"README","count":2,"results":["README.md","README.txt"],"root":""}"#,
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None),
            None
        );
    }

    #[test]
    fn fs_search_direct_answer_uses_top3_confirmation_for_ambiguous_matches() {
        let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"abcd","count":4,"results":["abcd_report.md","my_abcd.txt","x_abcd_log.txt","zz_abcd_backup.log"],"root":""}"#,
        )
        .expect("json");
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false).as_deref(),
            Some(
                "我找到 3 个最接近的候选，请确认要哪一个：abcd_report.md；my_abcd.txt；x_abcd_log.txt"
            )
        );
    }

    #[test]
    fn fs_search_direct_answer_prefers_exact_match_before_confirmation() {
        let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"README.md","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        )
        .expect("json");
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, None, false).as_deref(),
            Some("有，路径：README.md")
        );
    }

    #[test]
    fn fs_search_direct_answer_uses_locator_hint_when_output_omits_pattern() {
        let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","count":4,"results":["scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"],"root":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"}"#,
        )
        .expect("json");
        assert_eq!(
            super::fs_search_direct_answer_candidate(None, &value, Some("abcd"), false).as_deref(),
            Some(
                "我找到 3 个最接近的候选，请确认要哪一个：scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md；scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt；scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"
            )
        );
    }

    #[test]
    fn observed_entries_keep_latest_listing_plus_recent_non_listing_steps() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a.md\nb.md\nc.md\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_2", "read_file", "# A\nalpha\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_3", "read_file", "# B\nbeta\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_4", "read_file", "# C\ngamma\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_5", "read_file", "# D\ndelta\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_6", "read_file", "# E\nepsilon\n"));

        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 5);
        assert!(entries
            .iter()
            .any(|entry| entry.contains("step_1 skill(list_dir)")));
        assert!(entries
            .iter()
            .any(|entry| entry.contains("step_6 skill(read_file)")));
        assert!(!entries
            .iter()
            .any(|entry| entry.contains("step_2 skill(read_file)")));
    }

    #[test]
    fn normalized_listing_trims_blank_lines() {
        assert_eq!(
            normalized_observed_listing("\nfoo\n\nbar\n", None).as_deref(),
            Some("foo\nbar")
        );
    }

    #[test]
    fn normalized_listing_honors_requested_entry_limit() {
        assert_eq!(
            normalized_observed_listing("a\nb\nc\n", Some(2)).as_deref(),
            Some("a\nb")
        );
    }

    #[test]
    fn observed_entries_use_read_range_excerpt_body_instead_of_raw_json() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|Hello"}"#,
        ));
        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("read_range path=/tmp/README.md"));
        assert!(entries[0].contains("# RustClaw"));
        assert!(entries[0].contains("# RustClaw\n\nHello"));
        assert!(entries[0].contains("Hello"));
        assert!(!entries[0].contains(r#""action":"read_range""#));
    }

    #[test]
    fn observed_contract_json_includes_semantic_kind_and_locator_hint() {
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读一下 README.md 开头，然后用一句话总结".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        let contract = observed_contract_json(Some(&agent_run_context));
        assert!(contract.contains(r#""semantic_kind":"content_excerpt_summary""#));
        assert!(contract.contains(r#""locator_hint":"README.md""#));
    }

    #[test]
    fn observed_request_language_hint_follows_current_user_text() {
        assert_eq!(
            observed_request_language_hint("读一下 README 开头，三句话总结"),
            "mixed"
        );
        assert_eq!(
            observed_request_language_hint("Summarize the README in one sentence."),
            "en"
        );
        assert_eq!(observed_request_language_hint("只输出路径"), "zh-CN");
        assert_eq!(observed_request_language_hint("12345"), "config_default");
    }

    #[test]
    fn observed_response_style_hint_reflects_output_contract_shape() {
        let mut route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读一下 README.md 开头，然后用一句话总结".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let mut agent_run_context = AgentRunContext {
            route_result: Some(route_result.clone()),
            ..AgentRunContext::default()
        };
        assert!(
            observed_response_style_hint(Some(&agent_run_context)).contains("exactly one sentence")
        );

        route_result.output_contract.response_shape = OutputResponseShape::Scalar;
        agent_run_context.route_result = Some(route_result.clone());
        assert!(observed_response_style_hint(Some(&agent_run_context))
            .contains("only the final scalar value"));

        route_result.output_contract.response_shape = OutputResponseShape::Free;
        agent_run_context.route_result = Some(route_result.clone());
        assert!(
            observed_response_style_hint(Some(&agent_run_context)).contains("short direct answer")
        );

        route_result.output_contract.response_shape = OutputResponseShape::FileToken;
        agent_run_context.route_result = Some(route_result);
        assert!(observed_response_style_hint(Some(&agent_run_context)).contains("delivery token"));
    }

    #[test]
    fn observed_fallback_prompt_renders_language_and_response_style_hints() {
        let prompt = crate::render_prompt_template(
            OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
            &[
                ("__USER_REQUEST__", "读一下 README 开头，然后用一句话总结"),
                ("__RESOLVED_USER_INTENT__", "读一下 README 开头，然后用一句话总结"),
                (
                    "__OUTPUT_CONTRACT__",
                    r#"{"response_shape":"one_sentence","semantic_kind":"content_excerpt_summary"}"#,
                ),
                ("__OBSERVED_OUTPUTS__", "### step_1 skill(read_file)\n# RustClaw"),
                ("__CONFIG_RESPONSE_LANGUAGE__", "zh-CN"),
                ("__REQUEST_LANGUAGE_HINT__", "mixed"),
                (
                    "__RESPONSE_STYLE_HINT__",
                    "Return exactly one sentence unless the current user request explicitly asks for another exact sentence count.",
                ),
            ],
        );
        assert!(prompt.contains("Request language hint:\nmixed"));
        assert!(prompt.contains("Response style hint:"));
        assert!(prompt.contains("Return exactly one sentence"));
    }

    #[test]
    fn direct_answer_uses_inventory_dir_names_for_system_basic() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["act_plan.log","clawd.log","feishud.log"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("act_plan.log\nclawd.log\nfeishud.log")
        );
    }

    #[test]
    fn direct_answer_truncates_inventory_dir_names_to_requested_limit() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["a","b","c","d"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("a\nb")
        );
    }

    #[test]
    fn direct_answer_uses_latest_list_dir_entries_for_act_free_shape() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "README.txt\nnotes.md\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "列出 archive 目录下有什么".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "archive".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("README.txt\nnotes.md")
        );
    }

    #[test]
    fn direct_answer_truncates_list_dir_entries_to_requested_limit() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a\nb\nc\nd\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("a\nb")
        );
    }

    #[test]
    fn direct_answer_formats_hidden_entries_check_from_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".git\nREADME.md\n.env\nsrc\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "检查当前目录是否存在隐藏文件，然后用一句话解释隐藏文件的常见用途"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("有隐藏文件：.git, .env；这类以点开头的条目通常用于保存配置、元数据或本地状态。")
        );
    }

    #[test]
    fn direct_answer_formats_hidden_entries_check_scalar_from_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".git\nREADME.md\n.env\nsrc\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("有。示例：.git, .env")
        );
    }

    #[test]
    fn direct_answer_formats_hidden_entries_check_act_free_from_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".cargo/\nREADME.md\n.dockerignore\n.env.example\nsrc\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("有。示例：.cargo/, .dockerignore, .env.example")
        );
    }

    #[test]
    fn direct_answer_formats_hidden_entries_check_from_system_basic_inventory_dir() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/workspace","resolved_path":"/tmp/workspace","names_only":true,"include_hidden":true,"names":[".cargo",".dockerignore",".env.example","README.md","src"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
                locator_hint: ".".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("有。示例：.cargo, .dockerignore, .env.example")
        );
    }

    #[test]
    fn direct_answer_formats_existence_with_path_from_system_basic_path_batch_facts_even_when_free()
    {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/tmp/rustclaw-workspace/rustclaw.service","size_bytes":1190},"path":"/tmp/rustclaw-workspace/rustclaw.service"}],"include_missing":true}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("有，路径：/tmp/rustclaw-workspace/rustclaw.service")
        );
    }

    #[test]
    fn direct_answer_formats_existence_with_path_from_run_cmd_yes_output() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_exists_yes_{}_{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let target = temp_dir.join("rustclaw.service");
        std::fs::write(&target, "ok").expect("write target");
        let resolved = target
            .canonicalize()
            .unwrap_or(target.clone())
            .to_string_lossy()
            .to_string();

        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "yes\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        let expected = format!("有，路径：{resolved}");
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(expected.as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_formats_existence_with_path_from_run_cmd_exists_output() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_exists_lower_{}_{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let target = temp_dir.join("rustclaw.service");
        std::fs::write(&target, "ok").expect("write target");
        let resolved = target
            .canonicalize()
            .unwrap_or(target.clone())
            .to_string_lossy()
            .to_string();

        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "exists\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        let expected = format!("有，路径：{resolved}");
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(expected.as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_formats_existence_with_path_from_system_basic_find_name_output() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_exists_find_name_{}_{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let target = temp_dir.join("rustclaw.service");
        std::fs::write(&target, "ok").expect("write target");
        let resolved = target
            .canonicalize()
            .unwrap_or(target.clone())
            .to_string_lossy()
            .to_string();

        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        let expected = format!("有，路径：{resolved}");
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(expected.as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_does_not_passthrough_listing_when_content_evidence_is_required() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "base_skill_response_contract.md\nskill_integration_guide.md\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_does_not_passthrough_inventory_dir_when_content_evidence_is_required() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/docs","resolved_path":"/tmp/docs","names_only":true,"names":["base_skill_response_contract.md","skill_integration_guide.md"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_does_not_passthrough_run_cmd_listing_when_content_evidence_is_required() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd-observed-output-listing-only-{}-{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "a.md\nb.md\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
            None
        );
    }

    #[test]
    fn direct_answer_combines_inventory_dir_listing_with_docs_purpose_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/docs","resolved_path":"/tmp/docs","names_only":true,"names":["release_checklist.md","operator-guide.md","rollout-summary.pdf"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "release_checklist.md\noperator-guide.md\nrollout-summary.pdf\n\n这个目录主要放说明文档、操作指引和检查清单。"
            )
        );
    }

    #[test]
    fn direct_answer_keeps_listing_for_directory_purpose_summary_even_when_shape_one_sentence() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/docs","resolved_path":"/tmp/docs","names_only":true,"names":["release_checklist.md","operator-guide.md","rollout-summary.pdf"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "release_checklist.md\noperator-guide.md\nrollout-summary.pdf\n\n这个目录主要放说明文档、操作指引和检查清单。"
            )
        );
    }

    #[test]
    fn direct_answer_combines_run_cmd_listing_with_scripts_purpose_summary() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd-observed-output-directory-purpose-{}-{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "run_suite.sh\nrun_manual_test.sh\nsync_skill_docs.py\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "执行 ls scripts，然后用一句话告诉我这个目录大概放的是什么"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
                locator_hint: "scripts".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "run_suite.sh\nrun_manual_test.sh\nsync_skill_docs.py\n\n这个目录主要放运行、测试和维护用的脚本。"
            )
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_combines_run_cmd_shell_listing_with_scripts_purpose_summary() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd-observed-output-directory-purpose-shell-{}-{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "total 96\n-rwxr-xr-x   1 testuser  staff  10314 Apr  2 19:32 auth-key.sh\n-rwxr-xr-x   1 testuser  staff   3953 Apr  2 19:32 check-secrets.sh\n-rwxr-xr-x   1 testuser  staff  10941 Apr  2 19:32 codex_fix.sh\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "执行 ls scripts，然后用一句话告诉我这个目录大概放的是什么"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
                locator_hint: "scripts".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "auth-key.sh\ncheck-secrets.sh\ncodex_fix.sh\n\n这个目录主要放运行、测试和维护用的脚本。"
            )
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_classifies_recent_log_artifacts_from_shell_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "total 151792\n-rw-r--r--@ 1 testuser staff 76509771 Apr 12 16:30 model_io.log\n-rw-r--r--@ 1 testuser staff 906739 Apr 12 16:30 act_plan.log\n-rw-r--r--@ 1 testuser staff 191187 Apr 12 15:48 service_ops.log\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "列出 logs 目录最近修改的 3 个文件，再告诉我这更像是测试日志还是正式产物"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("最近修改的文件是：model_io.log、act_plan.log、service_ops.log；这些更像运行或测试过程中产生的日志，不像正式交付产物。")
        );
    }

    #[test]
    fn requested_listing_limit_parses_recent_numbered_file_count() {
        assert_eq!(
            requested_listing_limit_from_intent(
                "列出 logs 目录最近修改的 3 个文件，再告诉我这更像是测试日志还是正式产物"
            ),
            Some(3)
        );
        assert_eq!(
            requested_listing_limit_from_intent(
                "list the recent 3 files in logs and tell me whether they look like logs"
            ),
            Some(3)
        );
        assert_eq!(
            requested_listing_limit_from_intent(
                "List the 3 most recently modified files in the logs directory, then tell me whether this looks more like runtime logs or formal deliverables."
            ),
            Some(3)
        );
        assert_eq!(
            requested_listing_limit_from_intent(
                "List the three most recently modified files in docs and say whether they look more like logs or formal deliverables."
            ),
            Some(3)
        );
        assert_eq!(
            requested_listing_limit_from_intent(
                "列出logs目录最近修改的3个文件，再告诉我这更像是测试日志还是正式产物"
            ),
            Some(3)
        );
    }

    #[test]
    fn direct_answer_classifies_recent_formal_artifacts_from_shell_listing_in_english() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "total 48\n-rw-r--r--  1 testuser  staff  2048 Apr 12 16:30 release-checklist.md\n-rw-r--r--  1 testuser  staff  1024 Apr 12 16:20 operator-guide.md\n-rw-r--r--  1 testuser  staff   512 Apr 12 16:10 rollout-summary.pdf\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "List the three most recently modified files in docs and say whether they look more like logs or formal deliverables."
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
                locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("The most recently modified files are release-checklist.md, operator-guide.md, rollout-summary.pdf; these look more like prepared documents or formal deliverables than logs.")
        );
    }

    #[test]
    fn direct_answer_limits_recent_artifact_listing_for_english_recent_files_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "total 96\n-rw-r--r--  1 testuser  staff  2048 Apr 13 14:40 model_io.log\n-rw-r--r--  1 testuser  staff  1536 Apr 13 14:39 act_plan.log\n-rw-r--r--  1 testuser  staff  1024 Apr 13 14:38 service_ops.log\n-rw-r--r--  1 testuser  staff   900 Apr 13 14:37 wechatd.log\n-rw-r--r--  1 testuser  staff   880 Apr 13 14:36 webd.log\n-rw-r--r--  1 testuser  staff   860 Apr 13 14:35 feishud.log\n-rw-r--r--  1 testuser  staff   840 Apr 13 14:34 telegramd.log\n-rw-r--r--  1 testuser  staff   820 Apr 13 14:33 clawd.log\n-rw-r--r--  1 testuser  staff   800 Apr 13 14:32 logs_directory_listing.txt\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "List the 3 most recently modified files in the logs directory, then tell me whether this looks more like runtime logs or formal deliverables."
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
                locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("The most recently modified files are model_io.log, act_plan.log, service_ops.log; these look more like runtime or test logs than formal deliverables.")
        );
    }

    #[test]
    fn direct_answer_summarizes_system_basic_info_in_english_for_brief_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"rustclaw-test-host.local","os":"macos","arch":"x86_64","cwd":"/tmp/rustclaw-workspace"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent:
                "show me the basic machine info here like hostname and system, keep it brief"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RawCommandOutput,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("Hostname: rustclaw-test-host.local; system: macos (x86_64).")
        );
    }

    #[test]
    fn direct_answer_summarizes_archive_creation_success_with_destination() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            "exit=0\nupdating: tmp/rustclaw-workspace/scripts/skill_calls/\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "scripts/skill_calls -> tmp/nl_archive_case.zip".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("已成功打包到 tmp/nl_archive_case.zip。")
        );
    }

    #[test]
    fn direct_answer_prefers_archive_basic_output_destination_over_route_hint() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            r#"{"action":"pack","format":"zip","source":"/tmp/rustclaw-workspace/scripts/skill_calls","archive":"/tmp/rustclaw-workspace/tmp/nl_archive_case.zip","output":"exit=0\nupdating: skill_calls/\n"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::ExistenceWithPath,
                locator_hint: "scripts/skill_calls".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("已成功打包到 /tmp/rustclaw-workspace/tmp/nl_archive_case.zip。")
        );
    }

    #[test]
    fn direct_answer_summarizes_system_basic_info_without_action_field() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"hostname":"rustclaw-test-host.local","os":"macos","arch":"x86_64","cwd":"/tmp/rustclaw-workspace"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent:
                "show me the basic machine info here like hostname and system, keep it brief"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::RawCommandOutput,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("Hostname: rustclaw-test-host.local; system: macos (x86_64).")
        );
    }

    #[test]
    fn direct_answer_summarizes_workspace_project_from_top_level_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "Cargo.toml\ncrates/\nUI/\nconfigs/\nREADME.md\nREADME.zh-CN.md\nprompts/\nrustclaw.service\nstart-telegramd.sh\nstart-wechatd.sh\nstart-whatsappd.sh\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "用非技术用户能听懂的话，简短解释这个仓库主要是干什么的".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("这是一个可本地部署的智能助手平台仓库，带网页界面、多聊天渠道接入和后台服务，用来通过聊天或浏览器处理任务、记忆、调度和自动化。")
        );
    }

    #[test]
    fn direct_answer_summarizes_workspace_project_from_inventory_dir_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/workspace","resolved_path":"/tmp/workspace","names_only":true,"names":["Cargo.toml","crates","UI","configs","README.md","prompts","rustclaw.service","start-telegramd.sh","start-wechatd.sh","start-whatsappd.sh"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "用非技术用户能听懂的话，简短解释这个仓库主要是干什么的".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("这是一个可本地部署的智能助手平台仓库，带网页界面、多聊天渠道接入和后台服务，用来通过聊天或浏览器处理任务、记忆、调度和自动化。")
        );
    }

    #[test]
    fn direct_answer_summarizes_workspace_project_from_shell_listing_in_english() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd-observed-output-workspace-summary-{}-{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "Cargo.toml\ncrates\nUI\nconfigs\nREADME.md\nprompts\nrustclaw.service\nstart-telegramd.sh\nstart-wechatd.sh\nstart-whatsappd.sh\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "Inspect the current workspace and briefly explain in one sentence what this project is for."
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
                locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("This looks like a locally deployable assistant platform with a web UI, multiple chat-channel adapters, and backend services for tasks, memory, scheduling, and automation.")
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_summarizes_workspace_project_from_top_level_listing_in_english() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "Cargo.toml\ncrates/\nUI/\nconfigs/\nREADME.md\nREADME.zh-CN.md\nprompts/\nrustclaw.service\nstart-telegramd.sh\nstart-wechatd.sh\nstart-whatsappd.sh\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "Inspect the current workspace to identify the project type and briefly summarize what it is in one sentence."
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
                locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("This looks like a locally deployable assistant platform with a web UI, multiple chat-channel adapters, and backend services for tasks, memory, scheduling, and automation.")
        );
    }

    #[test]
    fn direct_scalar_uses_latest_list_dir_entries_when_listing_is_latest_step() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "README.txt\n"));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
            Some("README.txt")
        );
    }

    #[test]
    fn direct_scalar_path_only_uses_auto_locator_full_path_for_unique_list_dir_match() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd-observed-output-{}-{}",
            std::process::id(),
            crate::now_ts_u64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("Report.MD");
        std::fs::write(&file_path, "hello").unwrap();

        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "Report.MD\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "去 case_only 找 report.md，只输出路径".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_hint: "report.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(file_path.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        let resolved = file_path
            .canonicalize()
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(resolved.as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_scalar_does_not_passthrough_multiline_list_dir_listing() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "README.txt\nnotes.md\n",
        ));
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, None),
            None
        );
    }

    #[test]
    fn direct_scalar_counts_multiline_list_dir_when_route_requests_count() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a\nb\nc\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "数一下 scripts 目录直接有多少个子项".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarCount,
                locator_hint: "scripts".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("3")
        );
    }

    #[test]
    fn direct_scalar_uses_route_locator_hint_for_quantity_comparison() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a\nb\n"));
        loop_state
            .executed_step_results
            .push(ok_step("step_2", "list_dir", "a\nb\nc\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "上一个和上上个哪个更多，只回答目录名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "'上一个'=assistant[-1](document,2), '上上个'=assistant[-2](scripts,3); scripts 更多"
                .to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::QuantityComparison,
                locator_hint: "scripts".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("scripts")
        );
    }

    #[test]
    fn direct_answer_summarizes_git_status_dirty_worktree() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "git_basic",
            "exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查当前仓库是否存在未提交的改动，用一句话返回结果".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("当前仓库有未提交改动。")
        );
    }

    #[test]
    fn direct_answer_preserves_run_cmd_directory_entry_names() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_output_test_{}_run_cmd_names",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "act_plan.log\nclawd.log\nfeishud.log\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("act_plan.log\nclawd.log\nfeishud.log")
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_truncates_run_cmd_directory_entry_names_to_requested_limit() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_output_test_{}_run_cmd_limit",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "a\nb\nc\nd\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("a\nb")
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_formats_run_cmd_exists_probe_with_resolved_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "clawd_observed_output_test_{}_run_cmd_exists",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("rustclaw.service");
        std::fs::write(&file_path, "unit").expect("write fixture file");
        let resolved = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone())
            .to_string_lossy()
            .to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "EXISTS\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(resolved.clone()),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(format!("有，路径：{resolved}").as_str())
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn direct_answer_formats_run_cmd_not_found_probe_as_no() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "NOT_FOUND\n"));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "rustclaw.service".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("没有")
        );
    }

    #[test]
    fn direct_answer_passes_health_check_json_through_for_act_free_shape() {
        let mut loop_state = LoopState::new(2);
        let body = r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "health_check", body));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "做一次 health check".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(body)
        );
    }

    #[test]
    fn direct_answer_summarizes_health_check_for_act_free_summary_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":7,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent:
                "对系统做一次基础健康检查，只总结操作系统信息，RustClaw 自身不展开总结，仅返回其关键字段"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "macOS 宿主机当前没有明显的系统层告警。 clawd_process_count=7, telegramd_process_count=0, clawd_health_port_open=false"
            )
        );
    }

    #[test]
    fn direct_answer_prefers_health_check_summary_over_later_system_basic_steps() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":12,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"info","os":"macos","hostname":"example"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "Run a basic health check. Summarize only the host operating system, and for RustClaw itself just list the key fields.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "The macOS host has no obvious system-level warning right now. clawd_process_count=12, telegramd_process_count=0, clawd_health_port_open=false"
            )
        );
    }

    #[test]
    fn direct_answer_summarizes_health_check_for_one_sentence_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "clawd_process_count=1, telegramd_process_count=0, clawd_health_port_open=true, clawd_log_errors=0"
            )
        );
    }

    #[test]
    fn direct_answer_summarizes_health_check_when_clawd_is_unhealthy() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":0,"telegramd_process_count":1,"clawd_health_port_open":false,"clawd_log":{"exists":true,"keyword_error_count":3},"telegramd_log":{"exists":true,"keyword_error_count":0}}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "run a basic health check here and summarize only the most important findings"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "clawd_process_count=0, telegramd_process_count=1, clawd_health_port_open=false, clawd_log_errors=3, telegramd_log_errors=0"
            )
        );
    }

    #[test]
    fn direct_answer_summarizes_health_check_when_telegramd_is_stopped() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "clawd_process_count=1, telegramd_process_count=0, clawd_health_port_open=true, clawd_log_errors=0"
            )
        );
    }

    #[test]
    fn direct_answer_uses_original_user_request_language_for_health_check_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            user_request: Some(
                "run a basic health check here and summarize only the most important findings"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "clawd_process_count=1, telegramd_process_count=0, clawd_health_port_open=true, clawd_log_errors=0"
            )
        );
    }

    #[test]
    fn direct_answer_keeps_rustclaw_fields_when_fallback_route_only_mentions_os_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":12,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "做一次基础健康检查，只返回操作系统层面的关键字段，不要包含 RustClaw 自身的状态摘要"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_failed_fallback_router".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            user_request: Some(
                "做一次基础健康检查，只总结操作系统；RustClaw 自身不要总结，直接给我关键字段。"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "macOS 宿主机当前没有明显的系统层告警。 clawd_process_count=12, telegramd_process_count=0, clawd_health_port_open=false"
            )
        );
    }

    #[test]
    fn direct_answer_adds_os_summary_but_keeps_rustclaw_as_fields() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":true,"keyword_error_count":0},"system_health":{"os_family":"linux","warnings":["disk_root_low"]}}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "run a basic health check here and summarize only the most important findings"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            user_request: Some(
                "run a basic health check here and summarize only the most important findings"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "The Linux host has a system-level warning worth checking first: Root filesystem available space is low. clawd_process_count=1, telegramd_process_count=1, clawd_health_port_open=true, clawd_log_errors=0, telegramd_log_errors=0"
            )
        );
    }

    #[test]
    fn direct_answer_summarizes_process_basic_port_list_for_brief_requests() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "process_basic",
            "exit=0\nCOMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nclawd 4498 testuser 12u IPv4 0x0 0t0 TCP *:8787 (LISTEN)\nnginx 51129 testuser 6u IPv4 0x0 0t0 TCP *:80 (LISTEN)\nss-local 424 testuser 6u IPv4 0x0 0t0 TCP 127.0.0.1:1086 (LISTEN)\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "看看这台机器现在有哪些端口在监听，然后挑最值得注意的几个简单说一下"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        let answer =
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .expect("process_basic direct answer");
        assert!(answer.contains("clawd(8787)"));
        assert!(answer.contains("nginx(80)"));
        assert!(!answer.contains("COMMAND PID USER"));
    }

    #[test]
    fn direct_answer_summarizes_http_basic_success_for_one_sentence_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "http_basic",
            "status=200\n{\"ok\":true}\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "请求一下 http://127.0.0.1:8787/v1/health ，如果能通就简短总结结果"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Url,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("请求成功，返回 HTTP 200。")
        );
    }

    #[test]
    fn direct_answer_preserves_http_basic_raw_scalar_for_free_shape() {
        let mut loop_state = LoopState::new(2);
        let body = "status=200\n{\"ok\":true}\n";
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "http_basic", body));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "请求接口并返回原始结果".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Url,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("status=200")
        );
    }

    #[test]
    fn direct_answer_summarizes_service_control_status_for_chinese_brief_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=stopped","post_state":"telegramd=stopped","verified":true,"key_evidence":["telegramd process_count=0 memory_rss_bytes=Some(0)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=stopped"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "帮我检查 telegramd 现在是不是在运行，顺手简短解释状态".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "telegramd".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("telegramd 当前没有运行，检查到进程数为 0。")
        );
    }

    #[test]
    fn direct_answer_summarizes_service_control_status_for_english_brief_request() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=running","post_state":"telegramd=running","verified":true,"key_evidence":["telegramd process_count=1 memory_rss_bytes=Some(1024)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=running"}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "check whether telegramd is running right now and briefly explain the status"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "telegramd".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };
        assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some("telegramd is running; process count is 1.")
        );
    }

    #[test]
    fn observed_entries_compact_log_analyze_json_into_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "log_analyze",
            r#"{"path":"/tmp/test.log","total_lines":120,"keyword_counts":{"error":9,"panic":1},"recent_matches":["10: error one","20: panic two"]}"#,
        ));
        let entries = observed_output_entries(&loop_state);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("log_analyze path=/tmp/test.log total_lines=120"));
        assert!(entries[0].contains("keyword_counts: error=9, panic=1"));
        assert!(entries[0].contains("recent_matches:\n- 10: error one\n- 20: panic two"));
        assert!(!entries[0].contains(r#""keyword_counts""#));
    }
}
