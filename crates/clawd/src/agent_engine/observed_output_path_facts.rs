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

pub(super) fn run_cmd_contract_listing_text_candidate(
    route: &crate::RouteResult,
    body: &str,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is_any(
        route,
        &[
            crate::OutputSemanticKind::DirectoryNames,
            crate::OutputSemanticKind::FileNames,
            crate::OutputSemanticKind::DirectoryEntryGroups,
            crate::OutputSemanticKind::FilePaths,
        ],
    ) {
        return None;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar
    ) {
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
    if lines
        .iter()
        .any(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
    {
        return None;
    }
    normalized_listing_text(&lines.join("\n"))
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

pub(super) fn run_cmd_presence_with_path_candidate(
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

pub(super) fn existence_with_path_target_hint(
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

pub(super) fn candidate_exists_with_path_text(
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

pub(super) fn candidate_exists_scalar_text(
    state: Option<&AppState>,
    prefer_english: bool,
) -> String {
    observed_t(state, "clawd.msg.exists_yes", "有", "yes", prefer_english)
}

pub(super) fn candidate_exists_with_path_and_size_text(
    state: Option<&AppState>,
    path: Option<&str>,
    size_bytes: u64,
    prefer_english: bool,
) -> String {
    match path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => observed_t_with_vars(
            state,
            "clawd.msg.exists_with_path_and_size",
            "有，路径：{path}，大小：{size_bytes} 字节",
            "yes, path: {path}, size: {size_bytes} bytes",
            prefer_english,
            &[("path", path), ("size_bytes", &size_bytes.to_string())],
        ),
        None => observed_t_with_vars(
            state,
            "clawd.msg.exists_with_size",
            "有，大小：{size_bytes} 字节",
            "yes, size: {size_bytes} bytes",
            prefer_english,
            &[("size_bytes", &size_bytes.to_string())],
        ),
    }
}

pub(super) fn candidate_not_found_text(state: Option<&AppState>, prefer_english: bool) -> String {
    observed_t(state, "clawd.msg.exists_no", "没有", "no", prefer_english)
}

pub(super) fn candidate_not_found_with_path_text(
    state: Option<&AppState>,
    path: Option<&str>,
    prefer_english: bool,
) -> String {
    match path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => observed_t_with_vars(
            state,
            "clawd.msg.exists_no_path_not_found",
            "没有，路径不存在：{path}",
            "no, path not found: {path}",
            prefer_english,
            &[("path", path)],
        ),
        None => candidate_not_found_text(state, prefer_english),
    }
}

pub(super) fn normalize_system_basic_match_path(
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
    if candidate.exists() {
        return Some(canonical_existing_path(candidate));
    }
    let root = resolved_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(Path::new)?;
    let rooted = root.join(candidate);
    if rooted.exists() {
        Some(canonical_existing_path(&rooted))
    } else {
        Some(rooted.to_string_lossy().to_string())
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
        .then(|| path_batch_fact_preferred_path(entry).map(normalize_observed_scalar_path))
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

pub(super) fn system_basic_path_batch_file_basename_candidate(
    value: &serde_json::Value,
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
    if !exists {
        return None;
    }
    let path = path_batch_fact_preferred_path(entry)?;
    let basename = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    Some(basename.to_string())
}

pub(super) fn route_requires_single_file_delivery(route: &crate::RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) || matches!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    ) || (route.wants_file_delivery
        && !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryBatchFiles
        ))
}

pub(super) fn path_batch_file_delivery_token_candidate(
    route: Option<&crate::RouteResult>,
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

pub(super) fn path_batch_facts_requests_size(value: &serde_json::Value) -> bool {
    value
        .get("fields")
        .and_then(|fields| fields.as_array())
        .map(|fields| {
            fields.iter().any(|field| {
                field.as_str().is_some_and(|field| {
                    let field = field.trim().to_ascii_lowercase();
                    field == "size" || field == "size_bytes" || field == "file_size"
                })
            })
        })
        .unwrap_or(false)
}

pub(super) fn path_batch_fact_size_bytes(
    entry: &serde_json::Map<String, serde_json::Value>,
) -> Option<u64> {
    entry
        .get("fact")
        .and_then(|v| v.as_object())
        .and_then(|fact| fact.get("size_bytes"))
        .and_then(|v| v.as_u64())
        .or_else(|| entry.get("size_bytes").and_then(|v| v.as_u64()))
}

pub(super) fn display_path_kind(kind: &str, prefer_english: bool) -> String {
    let normalized = kind.trim().to_ascii_lowercase();
    match (normalized.as_str(), prefer_english) {
        ("dir" | "directory", true) => "directory".to_string(),
        ("dir" | "directory", false) => "目录".to_string(),
        ("file", true) => "file".to_string(),
        ("file", false) => "文件".to_string(),
        ("symlink", true) => "symlink".to_string(),
        ("symlink", false) => "符号链接".to_string(),
        ("other", true) => "other".to_string(),
        ("other", false) => "其他".to_string(),
        _ => kind.trim().to_string(),
    }
}

pub(super) fn route_prefers_path_kind_fact_answer(route: &crate::RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && !route.output_contract.delivery_required
        && super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ExistenceWithPath,
        )
}

pub(super) fn path_batch_fact_path_kind_candidate(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("path_batch_facts")
        || path_batch_facts_requests_size(value)
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
    let fact = entry.get("fact").and_then(|v| v.as_object())?;
    let path = path_batch_fact_preferred_path(entry)?;
    let kind = fact
        .get("kind")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(format!(
        "{path} | {}",
        display_path_kind(kind, prefer_english)
    ))
}

pub(super) fn system_basic_existence_with_path_candidate(
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
                return multi_path_batch_facts_candidate(state, facts, prefer_english);
            }
            let entry = facts.first()?.as_object()?;
            let exists = entry
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !exists {
                return Some(candidate_not_found_with_path_text(
                    state,
                    path_batch_fact_preferred_path(entry),
                    prefer_english,
                ));
            }
            let path = path_batch_fact_preferred_path(entry);
            if path_batch_facts_requests_size(value) {
                if let Some(size_bytes) = path_batch_fact_size_bytes(entry) {
                    return Some(candidate_exists_with_path_and_size_text(
                        state,
                        path,
                        size_bytes,
                        prefer_english,
                    ));
                }
            }
            Some(candidate_exists_with_path_text(state, path, prefer_english))
        }
        _ => None,
    }
}

pub(super) fn multi_path_batch_facts_candidate(
    _state: Option<&AppState>,
    facts: &[serde_json::Value],
    prefer_english: bool,
) -> Option<String> {
    let lines = facts
        .iter()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            let path = path_batch_fact_preferred_path(entry).unwrap_or("-");
            let exists = entry
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !exists {
                return Some(if prefer_english {
                    format!("{path}: not found")
                } else {
                    format!("{path}: 不存在")
                });
            }
            let kind = entry
                .get("fact")
                .and_then(|v| v.as_object())
                .and_then(|fact| fact.get("kind"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("unknown");
            Some(if prefer_english {
                format!("{path}: exists, type {}", display_path_kind(kind, true))
            } else {
                format!("{path}: 存在，类型：{}", display_path_kind(kind, false))
            })
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

pub(super) fn system_basic_scalar_existence_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    match value.get("action").and_then(|v| v.as_str())? {
        "path_batch_facts" => {
            let facts = value.get("facts").and_then(|v| v.as_array())?;
            if facts.len() != 1 {
                return None;
            }
            let exists = facts
                .first()?
                .as_object()?
                .get("exists")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(if exists {
                candidate_exists_scalar_text(state, prefer_english)
            } else {
                candidate_not_found_text(state, prefer_english)
            })
        }
        _ => None,
    }
}
