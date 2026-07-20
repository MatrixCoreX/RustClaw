use super::*;

pub(super) fn is_ignorable_shell_warning(line: &str) -> bool {
    let normalized = line.trim();
    normalized.starts_with("bash: warning: setlocale:")
        || normalized.starts_with("zsh: warning: setlocale:")
}

pub(super) fn run_cmd_directory_entry_list_candidate(
    body: &str,
    auto_locator_path: Option<&str>,
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
        .then(|| normalized_listing_text(&lines.join("\n")))
        .flatten()
}

pub(super) fn run_cmd_listing_text_candidate(
    body: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    run_cmd_shell_listing_entry_names(body)
        .map(|names| names.join("\n"))
        .or_else(|| run_cmd_directory_entry_list_candidate(body, auto_locator_path))
}

pub(super) fn run_cmd_shell_listing_entry_names(body: &str) -> Option<Vec<String>> {
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
    Some(names)
}

pub(super) fn path_fact_machine_answer(
    path: Option<&str>,
    exists: bool,
    kind: Option<&str>,
    size_bytes: Option<u64>,
    source_action: Option<&str>,
) -> String {
    let mut lines = vec![
        "message_key=clawd.msg.path_fact.observed".to_string(),
        "reason_code=path_fact_observed".to_string(),
        format!("exists={exists}"),
    ];
    if let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) {
        push_path_fact_machine_line(&mut lines, "path", path);
    }
    let kind = kind
        .map(normalized_path_kind_token)
        .or_else(|| (!exists).then(|| "missing".to_string()));
    if let Some(kind) = kind.filter(|kind| !kind.is_empty()) {
        lines.push(format!("kind={kind}"));
    }
    if !exists {
        lines.push("error_code=path_not_found".to_string());
    }
    if let Some(size_bytes) = size_bytes {
        lines.push(format!("size_bytes={size_bytes}"));
    }
    if let Some(source_action) = source_action
        .map(str::trim)
        .filter(|source_action| !source_action.is_empty())
    {
        push_path_fact_machine_line(&mut lines, "source_action", source_action);
    }
    lines.join("\n")
}

fn push_path_fact_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let value = crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(value).replace('\n', " "),
    );
    lines.push(format!("{key}={value}"));
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

pub(super) fn path_batch_fact_preferred_path<'a>(
    entry: &'a serde_json::Map<String, serde_json::Value>,
) -> Option<&'a str> {
    let fact = entry.get("fact").and_then(|v| v.as_object());
    fact.and_then(|item| item.get("resolved_path"))
        .or_else(|| fact.and_then(|item| item.get("path")))
        .or_else(|| entry.get("resolved_path"))
        .or_else(|| entry.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

pub(super) fn system_basic_path_batch_scalar_path_candidate(
    value: &serde_json::Value,
    field: &str,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("path_batch_facts") {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    if facts.len() != 1 {
        return None;
    }
    let entry = facts.first()?.as_object()?;
    let exists = entry
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    exists
        .then(|| {
            let fact = entry.get("fact").and_then(|value| value.as_object());
            fact.and_then(|fact| fact.get(field))
                .or_else(|| entry.get(field))
                .and_then(|value| value.as_str())
                .map(normalize_observed_scalar_path)
        })
        .flatten()
}

fn normalize_observed_scalar_path(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return String::new();
    }
    let mut normalized = std::path::PathBuf::new();
    for component in std::path::Path::new(path).components() {
        if matches!(component, std::path::Component::CurDir) {
            continue;
        }
        normalized.push(component.as_os_str());
    }
    let normalized = normalized.to_string_lossy().to_string();
    if normalized.is_empty() {
        ".".to_string()
    } else {
        normalized
    }
}

pub(super) fn route_requires_single_file_delivery(route: &crate::IntentOutputContract) -> bool {
    matches!(route.response_shape, crate::OutputResponseShape::FileToken)
        || matches!(
            route.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        )
        || (route.delivery_required
            && !matches!(
                route.delivery_intent,
                crate::OutputDeliveryIntent::DirectoryBatchFiles
            ))
}

pub(super) fn path_batch_file_delivery_token_candidate(
    route: Option<&crate::IntentOutputContract>,
    value: &serde_json::Value,
) -> Option<String> {
    let route = route?;
    if !route_requires_single_file_delivery(route)
        || value.get("action").and_then(|v| v.as_str()) != Some("path_batch_facts")
    {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    if facts.len() != 1 {
        return None;
    }
    let entry = facts.first()?.as_object()?;
    if !entry
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let path = path_batch_fact_preferred_path(entry)?;
    let fact_kind = entry
        .get("fact")
        .and_then(|value| value.get("kind"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if fact_kind.is_some_and(|kind| !kind.eq_ignore_ascii_case("file")) {
        return None;
    }
    if fact_kind.is_none() && !Path::new(path).is_file() {
        return None;
    }
    Some(format!("FILE:{path}"))
}
