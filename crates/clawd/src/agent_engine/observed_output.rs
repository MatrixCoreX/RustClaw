use std::path::Path;

use serde::Deserialize;

use super::{AgentRunContext, LoopState};
use crate::{llm_gateway, AppState, ClaimedTask};

const OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/observed_answer_fallback_prompt.md");
const OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH: &str =
    "prompts/observed_answer_fallback_prompt.md";

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
            && (crate::finalizer::classify_observed_content_status(body)
                == crate::finalizer::ObservedContentStatus::ContentAvailable
                || structured_scalar_candidate(&step.skill, body, None).is_some()
                || structured_observed_body(&step.skill, body).is_some())
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
    let listing = normalized_observed_listing(
        step.output.as_deref().unwrap_or_default(),
        max_entries,
    )?;
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        let mut lines = listing.lines().map(str::trim).filter(|line| !line.is_empty());
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
        let ones = chars
            .get(1)
            .and_then(|ch| digit_value(*ch))
            .unwrap_or(0);
        let consumed = if ones > 0 { 2 } else { 1 };
        return Some((10 + ones, consumed));
    }
    let tens = digit_value(chars[0])?;
    if chars.get(1) == Some(&'十') {
        let ones = chars
            .get(2)
            .and_then(|ch| digit_value(*ch))
            .unwrap_or(0);
        let consumed = if ones > 0 { 3 } else { 2 };
        return Some((tens * 10 + ones, consumed));
    }
    Some((tens, 1))
}

fn parse_positive_number_prefix(text: &str) -> Option<usize> {
    let trimmed = text.trim_start();
    let digit_len = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    if digit_len > 0 {
        return trimmed[..digit_len].parse::<usize>().ok().filter(|n| *n > 0);
    }
    parse_small_zh_number_prefix(trimmed).map(|(value, _)| value).filter(|n| *n > 0)
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
    None
}

fn requested_listing_limit(route: Option<&crate::RouteResult>) -> Option<usize> {
    requested_listing_limit_from_intent(route?.resolved_intent.as_str())
}

fn route_requests_scalar_count(route: &crate::RouteResult) -> bool {
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    let intent = route.resolved_intent.trim().to_ascii_lowercase();
    if intent.is_empty() {
        return false;
    }
    intent.contains("how many")
        || intent.contains("count ")
        || intent.contains(" count")
        || intent.contains("number of")
        || route.resolved_intent.contains("多少")
        || route.resolved_intent.contains("几个")
        || route.resolved_intent.contains("几项")
        || route.resolved_intent.contains("数量")
        || route.resolved_intent.contains("数一下")
        || route.resolved_intent.contains("统计")
}

fn route_requests_quantity_comparison(route: &crate::RouteResult) -> bool {
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    let intent = route.resolved_intent.trim().to_ascii_lowercase();
    if intent.is_empty() {
        return false;
    }
    intent.contains("which more")
        || intent.contains("which has more")
        || intent.contains("more")
        || intent.contains("fewer")
        || intent.contains("less")
        || route.resolved_intent.contains("哪个更多")
        || route.resolved_intent.contains("更多")
        || route.resolved_intent.contains("更少")
        || route.resolved_intent.contains("一样多")
}

fn route_requests_scalar_path_only(route: &crate::RouteResult) -> bool {
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    let intent = route.resolved_intent.trim();
    if intent.is_empty() {
        return false;
    }
    let lower = intent.to_ascii_lowercase();
    intent.contains("只输出路径")
        || intent.contains("只回路径")
        || intent.contains("只给路径")
        || intent.contains("绝对路径")
        || lower.contains("path only")
        || lower.contains("only the path")
        || lower.contains("only path")
        || lower.contains("absolute path")
        || lower.contains("output only the path")
}

fn latest_list_dir_listing(loop_state: &LoopState) -> Option<String> {
    let idx = latest_successful_step_index(loop_state, |_| true)?;
    let step = &loop_state.executed_step_results[idx];
    if !step.is_ok() || step.skill != "list_dir" {
        return None;
    }
    normalized_observed_listing(step.output.as_deref().unwrap_or_default(), None)
}

fn comparison_winner_from_route(route: &crate::RouteResult, loop_state: &LoopState) -> Option<String> {
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

fn count_answer_from_latest_listing(route: &crate::RouteResult, loop_state: &LoopState) -> Option<String> {
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

fn service_control_direct_answer_candidate(
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
        return Some(if prefer_english {
            format!("{service_name} status check failed: {reason}.")
        } else {
            format!("检查 {service_name} 状态失败：{reason}")
        });
    }

    let runtime_state = service_control_runtime_state(value)?;
    let process_count = service_control_process_count(value);
    Some(match (prefer_english, runtime_state, process_count) {
        (true, ServiceControlRuntimeState::Running, Some(count)) => {
            format!("{service_name} is running; process count is {count}.")
        }
        (true, ServiceControlRuntimeState::Running, None) => {
            format!("{service_name} is running.")
        }
        (true, ServiceControlRuntimeState::Stopped, Some(count)) => {
            format!("{service_name} is not running; process count is {count}.")
        }
        (true, ServiceControlRuntimeState::Stopped, None) => {
            format!("{service_name} is not running.")
        }
        (true, ServiceControlRuntimeState::Unknown, _) => {
            format!("{service_name} status is unknown.")
        }
        (false, ServiceControlRuntimeState::Running, Some(count)) => {
            format!("{service_name} 当前正在运行，检查到进程数为 {count}。")
        }
        (false, ServiceControlRuntimeState::Running, None) => {
            format!("{service_name} 当前正在运行。")
        }
        (false, ServiceControlRuntimeState::Stopped, Some(count)) => {
            format!("{service_name} 当前没有运行，检查到进程数为 {count}。")
        }
        (false, ServiceControlRuntimeState::Stopped, None) => {
            format!("{service_name} 当前没有运行。")
        }
        (false, ServiceControlRuntimeState::Unknown, _) => {
            format!("{service_name} 当前状态未知。")
        }
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
    if lines.is_empty() || lines.len() > 200 {
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

fn run_cmd_presence_with_path_candidate(
    body: &str,
    auto_locator_path: Option<&str>,
    english_answer: bool,
) -> Option<String> {
    let scalar = normalized_scalar_candidate(body)?;
    match scalar.as_str() {
        "EXISTS" => Some(
            match auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
            {
                Some(path) if english_answer => format!("yes, path: {path}"),
                Some(path) => format!("有，路径：{path}"),
                None if english_answer => "yes".to_string(),
                None => "有".to_string(),
            },
        ),
        "NOT_FOUND" => Some(if english_answer {
            "no".to_string()
        } else {
            "没有".to_string()
        }),
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
    value: &serde_json::Value,
    locator_hint: Option<&str>,
) -> Option<String> {
    let (results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return Some("没有找到匹配项".to_string());
    }
    if results.len() == 1 {
        return Some(results[0].clone());
    }
    let pattern = normalized_find_name_pattern(pattern.as_deref())
        .or_else(|| normalized_find_name_pattern(locator_hint))?;
    preferred_fs_search_exact_match(&results, &pattern)
}

fn fs_search_direct_answer_candidate(
    value: &serde_json::Value,
    locator_hint: Option<&str>,
) -> Option<String> {
    let (results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return Some("没有找到匹配项".to_string());
    }
    if results.len() == 1 {
        return Some(format!("有，路径：{}", results[0]));
    }
    if let Some(pattern) = normalized_find_name_pattern(pattern.as_deref())
        .or_else(|| normalized_find_name_pattern(locator_hint))
    {
        if let Some(preferred) = preferred_fs_search_exact_match(&results, &pattern) {
            return Some(format!("有，路径：{}", preferred));
        }
        let ranked = rank_fs_search_candidates(&results, &pattern);
        if !ranked.is_empty() {
            return Some(format!(
                "我找到 {} 个最接近的候选，请确认要哪一个：{}",
                ranked.len(),
                ranked.join("；")
            ));
        }
    }
    Some(format!(
        "找到 {} 个匹配项：{}",
        results.len().min(count.max(results.len())),
        results.into_iter().take(3).collect::<Vec<_>>().join("；")
    ))
}

fn log_analyze_direct_answer_candidate(value: &serde_json::Value) -> Option<String> {
    let display_path = value
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("日志");
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
        return Some(format!("{display_path} 未发现明显异常。"));
    }
    let (keyword, count) = counts.into_iter().next()?;
    let mut out = format!("{display_path} 里最值得注意的是 {keyword}={count}。");
    if let Some(line) = recent {
        out.push_str(&format!(" 最近一条相关记录：{line}"));
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

fn process_basic_port_list_summary_candidate(body: &str) -> Option<String> {
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
    Some(if has_public {
        format!(
            "最值得注意的是 {}；其余大多是本地回环或辅助监听端口。",
            highlights.join("、")
        )
    } else {
        format!("当前主要看到这些监听端口：{}。", highlights.join("、"))
    })
}

fn git_basic_status_summary_candidate(body: &str) -> Option<String> {
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
        Some("当前仓库有未提交改动。".to_string())
    } else {
        Some("当前仓库没有未提交改动。".to_string())
    }
}

fn structured_scalar_candidate(
    skill: &str,
    body: &str,
    locator_hint: Option<&str>,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if skill == "db_basic" {
        return db_basic_scalar_candidate(&value);
    }
    if skill == "service_control" {
        return service_control_summary_candidate(&value);
    }
    if skill == "fs_search" {
        return fs_search_scalar_candidate(&value, locator_hint);
    }
    if skill == "process_basic" {
        return process_basic_port_list_summary_candidate(body);
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
            Some(format!("{field_path} 字段不存在"))
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
        "fs_search" => fs_search_direct_answer_candidate(&value, None),
        "log_analyze" => compact_log_analyze_excerpt(&value),
        _ => None,
    }
}

pub(crate) fn extract_direct_scalar_from_generic_output_with_locator_hint(
    loop_state: &LoopState,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
) -> Option<String> {
    if let Some(answer) = latest_successful_list_dir_answer_candidate(
        loop_state,
        Some(crate::OutputResponseShape::Scalar),
        None,
        auto_locator_path,
        prefer_full_path,
    ) {
        if !crate::finalizer::looks_like_planner_artifact(&answer)
            && !crate::finalizer::looks_like_internal_trace_artifact(&answer)
        {
            return Some(answer);
        }
    }
    let observed_output = extract_latest_generic_successful_output(loop_state)?;
    let answer = structured_scalar_candidate(
        &observed_output.skill,
        &observed_output.body,
        locator_hint.filter(|hint| !hint.trim().is_empty()),
    )
    .or_else(|| normalized_scalar_candidate(&observed_output.body))?;
    if crate::finalizer::looks_like_planner_artifact(&answer)
        || crate::finalizer::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    Some(answer)
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
    extract_direct_scalar_from_generic_output_with_locator_hint(
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
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
    let summary = match status_code {
        200..=299 => {
            if prefer_english {
                format!("Request succeeded with HTTP {status_code}.")
            } else {
                format!("请求成功，返回 HTTP {status_code}。")
            }
        }
        300..=399 => {
            if prefer_english {
                format!("Request completed with HTTP {status_code} redirection.")
            } else {
                format!("请求已完成，但返回了 HTTP {status_code} 重定向。")
            }
        }
        _ => {
            if prefer_english {
                format!("Request returned HTTP {status_code}.")
            } else {
                format!("请求返回 HTTP {status_code}。")
            }
        }
    };
    Some(summary)
}

pub(crate) fn extract_direct_answer_from_generic_output(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let response_shape = route.map(|route| route.output_contract.response_shape);
    let routed_mode = route.map(|route| route.routed_mode);
    let requested_listing_limit = requested_listing_limit(route);
    let locator_hint = route
        .map(|route| route.output_contract.locator_hint.as_str())
        .filter(|hint| !hint.trim().is_empty());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let prefers_english_presence_answer = route
        .map(|route| route.resolved_intent.to_ascii_lowercase())
        .is_some_and(|intent| intent.contains("yes or no") || intent.contains("path"));
    let prefers_english_free_text = route
        .map(|route| route.resolved_intent.trim())
        .filter(|intent| !intent.is_empty())
        .is_some_and(|intent| !text_contains_cjk(intent));
    let health_check_prefers_raw_payload = matches!(routed_mode, Some(crate::RoutedMode::Act))
        && !matches!(
            response_shape,
            Some(crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar)
        );
    let prefer_full_path = route.is_some_and(route_requests_scalar_path_only);

    let answer = latest_successful_list_dir_answer_candidate(
        loop_state,
        response_shape,
        requested_listing_limit,
        auto_locator_path,
        prefer_full_path,
    )
    .or_else(|| {
            let observed_output = extract_latest_generic_successful_output(loop_state)?;
            if observed_output.skill == "run_cmd" {
                run_cmd_presence_with_path_candidate(
                    &observed_output.body,
                    auto_locator_path,
                    prefers_english_presence_answer,
                )
                .or_else(|| {
                    run_cmd_directory_entry_list_candidate(
                        &observed_output.body,
                        auto_locator_path,
                        requested_listing_limit,
                    )
                })
            } else {
                None
            }
            .or_else(|| match observed_output.skill.as_str() {
                "health_check" => {
                    health_check_prefers_raw_payload.then_some(observed_output.body.clone())
                }
                "http_basic" => http_basic_summary_candidate(
                    &observed_output.body,
                    response_shape,
                    prefers_english_free_text,
                ),
                "process_basic" => process_basic_port_list_summary_candidate(&observed_output.body),
                "service_control" => {
                    serde_json::from_str::<serde_json::Value>(&observed_output.body)
                        .ok()
                        .and_then(|value| {
                            service_control_direct_answer_candidate(
                                &value,
                                prefers_english_free_text,
                            )
                        })
                }
                "fs_search" => serde_json::from_str::<serde_json::Value>(&observed_output.body)
                    .ok()
                    .and_then(|value| fs_search_direct_answer_candidate(&value, locator_hint)),
                "git_basic" => git_basic_status_summary_candidate(&observed_output.body),
                "log_analyze" => serde_json::from_str::<serde_json::Value>(&observed_output.body)
                    .ok()
                    .and_then(|value| log_analyze_direct_answer_candidate(&value)),
                "system_basic" => {
                    let value =
                        serde_json::from_str::<serde_json::Value>(&observed_output.body).ok()?;
                    let action = value.get("action").and_then(|v| v.as_str())?;
                    if action == "read_range"
                        && matches!(routed_mode, Some(crate::RoutedMode::Act))
                        && !matches!(
                            response_shape,
                            Some(crate::OutputResponseShape::OneSentence)
                        )
                    {
                        value
                            .get("excerpt")
                            .and_then(|v| v.as_str())
                            .and_then(normalize_read_range_excerpt)
                    } else if action == "inventory_dir"
                        && matches!(routed_mode, Some(crate::RoutedMode::Act))
                    {
                        inventory_dir_direct_answer_candidate(&value, requested_listing_limit)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .or_else(|| {
                structured_scalar_candidate(
                    &observed_output.skill,
                    &observed_output.body,
                    locator_hint,
                )
            })
            .or_else(|| normalized_scalar_candidate(&observed_output.body))
        })?;
    if crate::finalizer::looks_like_planner_artifact(&answer)
        || crate::finalizer::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    Some(answer)
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
    (crate::finalizer::classify_observed_content_status(body)
        == crate::finalizer::ObservedContentStatus::ContentAvailable)
        .then(|| body.to_string())
}

fn observed_step_entry(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let output = observed_step_body(step)?;
    if crate::finalizer::looks_like_planner_artifact(&output)
        || crate::finalizer::looks_like_internal_trace_artifact(&output)
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
                &state.command_intent.default_locale,
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
    let semantically_publishable = if !answer.is_empty() && !parsed.needs_clarify {
        if parsed.publishable {
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
                crate::finalizer::FinalizerDisposition::QualifiedCompletion
            } else {
                crate::finalizer::FinalizerDisposition::AllowFallback
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

pub(crate) fn normalized_observed_listing(observed: &str, max_entries: Option<usize>) -> Option<String> {
    trim_listing_to_max_entries(observed, max_entries)
}

#[cfg(test)]
mod tests {
    use super::super::LoopState;
    use super::{
        extract_direct_answer_from_generic_output, extract_direct_scalar_from_generic_output,
        extract_direct_scalar_from_generic_output_with_locator_hint, normalized_observed_listing,
        observed_output_entries, AgentRunContext,
    };
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    use crate::{
        IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
        ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
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
            super::fs_search_direct_answer_candidate(&value, None).as_deref(),
            Some("我找到 3 个最接近的候选，请确认要哪一个：abcd_report.md；my_abcd.txt；x_abcd_log.txt")
        );
    }

    #[test]
    fn fs_search_direct_answer_prefers_exact_match_before_confirmation() {
        let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"README.md","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        )
        .expect("json");
        assert_eq!(
            super::fs_search_direct_answer_candidate(&value, None).as_deref(),
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
            super::fs_search_direct_answer_candidate(&value, Some("abcd")).as_deref(),
            Some("我找到 3 个最接近的候选，请确认要哪一个：scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md；scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt；scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt")
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
    fn direct_answer_uses_inventory_dir_names_for_system_basic() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["act_plan.log","clawd.log","feishud.log"]}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "logs".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "logs".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "archive".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "logs".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "report.md".to_string(),
            },
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
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "a\nb\nc\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::Act,
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "scripts".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "scripts".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: String::new(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "logs".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "logs".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "rustclaw.service".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "rustclaw.service".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: String::new(),
            },
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
    fn direct_answer_does_not_pass_health_check_json_through_for_one_sentence_summary() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#,
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: String::new(),
            },
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
    fn direct_answer_summarizes_process_basic_port_list_for_brief_requests() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "process_basic",
            "exit=0\nCOMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nclawd 4498 testuser 12u IPv4 0x0 0t0 TCP *:8787 (LISTEN)\nnginx 51129 testuser 6u IPv4 0x0 0t0 TCP *:80 (LISTEN)\nss-local 424 testuser 6u IPv4 0x0 0t0 TCP 127.0.0.1:1086 (LISTEN)\n",
        ));
        let route_result = RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: String::new(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Url,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Url,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "telegramd".to_string(),
            },
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
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                locator_hint: "telegramd".to_string(),
            },
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
