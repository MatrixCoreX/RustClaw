use super::*;

pub(super) fn directory_has_unique_entry_for_search_name(root: &str, token: &str) -> bool {
    let root = Path::new(root);
    if !root.is_dir() {
        return false;
    }
    let token = token.to_ascii_lowercase();
    if token.len() < 2 {
        return false;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut visits = 0usize;
    let mut matches = 0usize;
    while let Some(dir) = stack.pop() {
        visits = visits.saturating_add(1);
        if visits > 10_000 || matches > 1 {
            return false;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let name = name.to_ascii_lowercase();
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase());
            if name == token || stem.as_deref() == Some(token.as_str()) {
                matches = matches.saturating_add(1);
                if matches > 1 {
                    return false;
                }
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    matches == 1
}

pub(super) fn single_name_target_for_directory_locator(
    route: &RouteResult,
    current_user_text: &str,
) -> Option<String> {
    if has_multiple_quoted_search_name_targets(current_user_text) {
        return None;
    }
    single_filename_target_for_directory_locator(route, current_user_text)
        .or_else(|| single_quoted_search_name_target(current_user_text))
        .or_else(|| single_quoted_search_name_target(&route.resolved_intent))
        .or_else(|| single_identifier_search_name_target_outside_locators(current_user_text))
}

pub(super) fn archive_entry_target_for_route_or_text(
    route: &RouteResult,
    current_user_text: &str,
    archive_path: &str,
) -> Option<String> {
    let archive_path = archive_path.trim();
    if archive_path.is_empty() || !is_supported_archive_path(archive_path) {
        return None;
    }

    let mut path_candidates = Vec::new();
    let mut filename_candidates = Vec::new();
    for text in [current_user_text, route.resolved_intent.as_str()] {
        for locator in
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        {
            push_archive_entry_target_candidate(
                &mut path_candidates,
                &mut filename_candidates,
                &locator.locator_hint,
                archive_path,
            );
        }
        for filename in crate::delivery_utils::extract_filename_candidates(text) {
            push_archive_entry_target_candidate(
                &mut path_candidates,
                &mut filename_candidates,
                &filename,
                archive_path,
            );
        }
    }

    path_candidates
        .into_iter()
        .next()
        .or_else(|| filename_candidates.into_iter().next())
}

pub(super) fn push_archive_entry_target_candidate(
    path_candidates: &mut Vec<String>,
    filename_candidates: &mut Vec<String>,
    candidate: &str,
    archive_path: &str,
) {
    let Some(candidate) = normalize_archive_entry_target_candidate(candidate, archive_path) else {
        return;
    };
    let target = if candidate.contains('/') || candidate.contains('\\') {
        path_candidates
    } else {
        filename_candidates
    };
    if !target
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        target.push(candidate);
    }
}

pub(super) fn normalize_archive_entry_target_candidate(
    candidate: &str,
    archive_path: &str,
) -> Option<String> {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
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
    });
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains("://")
        || Path::new(trimmed).is_absolute()
        || is_supported_archive_path(trimmed)
        || is_sqlite_database_path(trimmed)
        || archive_locator_candidate_matches_archive(trimmed, archive_path)
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(trimmed)
    {
        return None;
    }
    if !trimmed.contains('/')
        && !trimmed.contains('\\')
        && !archive_entry_target_candidate_has_extension(trimmed)
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(super) fn archive_locator_candidate_matches_archive(
    candidate: &str,
    archive_path: &str,
) -> bool {
    let candidate_norm = candidate.replace('\\', "/");
    let archive_norm = archive_path.trim().replace('\\', "/");
    if candidate_norm.eq_ignore_ascii_case(&archive_norm) {
        return true;
    }
    let archive_name = archive_norm
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(archive_norm.as_str());
    candidate_norm.eq_ignore_ascii_case(archive_name)
}

pub(super) fn archive_entry_target_candidate_has_extension(candidate: &str) -> bool {
    let basename = candidate
        .rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .unwrap_or(candidate);
    let Some((stem, ext)) = basename.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=16).contains(&ext.len())
        && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
}

pub(super) fn existence_with_path_locator_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = existence_with_path_locator_observation_plan(
        route_result,
        auto_locator_path,
        current_user_text,
    )?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn file_paths_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::FilePaths
    {
        return None;
    }

    let hint = route.output_contract.locator_hint.trim();
    let auto_dir = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| Path::new(path).is_dir());
    let hint_looks_like_path = hint.contains(['/', '\\']) || hint.starts_with('.');
    let hint_allows_directory_locator = hint.is_empty()
        || auto_dir.is_some_and(|path| locator_path_matches_hint(path, hint))
        || Path::new(hint).is_dir()
        || matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
        || hint_looks_like_path;
    if hint_allows_directory_locator {
        if let Some(path) = route_directory_locator_path(route, auto_locator_path)
            .filter(|path| Path::new(path).is_dir())
        {
            if let Some(ext) = structural_extension_filter_for_directory_inventory(route, "", None)
            {
                let max_results = requested_file_paths_result_limit(route, "", None).unwrap_or(100);
                return Some(vec![AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "find_entries",
                        "root": path,
                        "ext": ext,
                        "target_kind": "file",
                        "max_results": max_results,
                        "recursive": true,
                    }),
                }]);
            }
            let max_entries = requested_file_paths_result_limit(route, "", None).unwrap_or(1000);
            let selector = &route.output_contract.self_extension.list_selector;
            let sort_by = selector
                .sort_by
                .clone()
                .or_else(|| contract_hint_selector_sort_by(&route.route_reason))
                .unwrap_or_else(|| "size_desc".to_string());
            let target_kind = if selector.target_kind != crate::OutputScalarCountTargetKind::Any {
                selector.target_kind
            } else {
                contract_hint_selector_target_kind(&route.route_reason)
                    .and_then(|token| selector_target_kind_from_machine_token(&token))
                    .unwrap_or(crate::OutputScalarCountTargetKind::File)
            };
            let mut args = serde_json::json!({
                "action": "list_dir",
                "path": path,
                "names_only": false,
                "max_entries": max_entries,
                "sort_by": sort_by,
            });
            if target_kind == crate::OutputScalarCountTargetKind::Dir {
                args["dirs_only"] = Value::Bool(true);
            } else {
                args["files_only"] = Value::Bool(true);
            }
            return Some(vec![AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args,
            }]);
        }
    }

    if !hint.is_empty() {
        if Path::new(hint).is_dir() {
            return None;
        }
        if let Some((root, pattern)) = split_path_like_file_locator_hint(hint) {
            return Some(vec![AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "find_entries",
                    "root": root,
                    "pattern": pattern,
                    "target_kind": "file",
                    "max_results": 50,
                }),
            }]);
        }
        return Some(vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "find_entries",
                "root": ".",
                "pattern": hint,
                "target_kind": "file",
                "max_results": 50,
            }),
        }]);
    }

    let path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| Path::new(path).is_file())?;
    let name = Path::new(path).file_name()?.to_string_lossy().to_string();
    let root = Path::new(path)
        .parent()
        .and_then(|parent| parent.to_str())
        .filter(|parent| !parent.is_empty())
        .unwrap_or(".");
    Some(vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": root,
            "pattern": name,
            "target_kind": "file",
            "max_results": 50,
        }),
    }])
}

pub(super) fn split_path_like_file_locator_hint(hint: &str) -> Option<(String, String)> {
    let trimmed = hint.trim();
    if trimmed.is_empty() || (!trimmed.contains('/') && !trimmed.contains('\\')) {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let (root, file_name) = normalized.rsplit_once('/')?;
    let file_name = file_name.trim();
    if file_name.is_empty() {
        return None;
    }
    let root = if root.trim().is_empty() {
        if normalized.starts_with('/') {
            "/"
        } else {
            "."
        }
    } else {
        root.trim()
    };
    Some((root.to_string(), file_name.to_string()))
}

pub(super) fn file_paths_locator_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = file_paths_locator_observation_plan(route_result, auto_locator_path)?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn doc_parse_supported_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.trim().to_ascii_lowercase().as_str(),
                "md" | "txt" | "html" | "htm" | "pdf" | "docx"
            )
        })
        .unwrap_or(false)
}

pub(super) fn doc_parse_is_enabled(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("doc_parse")
}

pub(super) fn log_analyze_is_enabled(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("log_analyze")
}

pub(super) fn log_analyze_supported_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.trim().eq_ignore_ascii_case("log"))
        .unwrap_or(false)
}

pub(super) fn directory_contains_log_like_files(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    entries.flatten().take(64).any(|entry| {
        let path = entry.path();
        path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.trim().eq_ignore_ascii_case("log"))
    })
}

pub(super) fn log_analyze_supported_target(path: &str) -> bool {
    let path = Path::new(path);
    if path.is_file() {
        return log_analyze_supported_path(path.to_string_lossy().as_ref());
    }
    directory_contains_log_like_files(path)
}

pub(super) fn contract_allows_log_analyze_for_path(route: &RouteResult, path: &str) -> bool {
    let args = serde_json::json!({
        "path": path,
        "max_matches": 50,
    });
    crate::contract_matrix::action_policy_for_output_contract(
        Some(&route.output_contract),
        "log_analyze",
        &args,
    )
    .map(|policy| policy.is_allowed())
    .unwrap_or(true)
}

pub(super) fn generic_path_content_log_analyze_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::ContentExcerptSummary
        )
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return None;
    }
    if auto_locator_path
        .map(str::trim)
        .filter(|path| Path::new(path).is_dir())
        .is_some()
        && explicit_log_file_target_under_directory_locator(route, auto_locator_path).is_some()
    {
        return None;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        })
        .filter(|path| log_analyze_supported_target(path))
        .map(ToString::to_string)
}

pub(super) fn explicit_log_file_target_under_directory_locator(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let directory = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        })?;
    let directory = Path::new(directory);
    if !directory.is_dir() {
        return None;
    }

    let mut candidates = Vec::new();
    for text in [route.resolved_intent.as_str(), route.route_reason.as_str()] {
        for locator in
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        {
            candidates.push(locator.locator_hint);
        }
        for filename in crate::delivery_utils::extract_filename_candidates(text) {
            candidates.push(filename);
        }
        for key in ["target_path", "target_file", "file_path", "log_file"] {
            if let Some(value) = route_machine_token_value(text, key) {
                candidates.push(value);
            }
        }
    }

    candidates
        .into_iter()
        .find_map(|candidate| {
            let candidate = candidate.trim();
            if candidate.is_empty() {
                return None;
            }
            let candidate_path = Path::new(candidate);
            let file_name = candidate_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(candidate)
                .trim();
            if file_name.is_empty() || !log_analyze_supported_path(file_name) {
                return None;
            }
            let resolved = if candidate_path.is_absolute() {
                candidate_path.to_path_buf()
            } else {
                directory.join(file_name)
            };
            (resolved.is_file() && log_analyze_supported_path(resolved.to_string_lossy().as_ref()))
                .then(|| resolved.display().to_string())
        })
        .or_else(|| {
            route_log_name_filter(route)
                .and_then(|filter| unique_log_file_matching_name_filter(directory, &filter))
        })
}

fn route_machine_token_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    text.split_whitespace().find_map(|token| {
        let token = token.trim_matches(machine_token_outer_delimiter);
        let value = token.strip_prefix(&prefix)?;
        let value = value.trim_matches(machine_token_outer_delimiter);
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn machine_token_outer_delimiter(ch: char) -> bool {
    matches!(
        ch,
        ',' | '，'
            | '。'
            | ';'
            | '；'
            | ':'
            | '：'
            | ')'
            | '）'
            | ']'
            | '}'
            | '>'
            | '》'
            | '"'
            | '\''
    )
}

fn route_log_name_filter(route: &RouteResult) -> Option<String> {
    [route.resolved_intent.as_str(), route.route_reason.as_str()]
        .into_iter()
        .find_map(|text| route_machine_token_value(text, "name_filter"))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| value.len() >= 2)
}

fn unique_log_file_matching_name_filter(directory: &Path, filter: &str) -> Option<String> {
    let entries = fs::read_dir(directory).ok()?;
    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !log_analyze_supported_path(path.to_string_lossy().as_ref()) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        if lower == filter || stem.as_deref() == Some(filter) || lower.contains(filter) {
            matches.push(path);
            if matches.len() > 1 {
                return None;
            }
        }
    }
    matches.pop().map(|path| path.display().to_string())
}

fn filename_prefix_pattern(path: &str) -> Option<String> {
    let file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    let stem = file_name
        .split('.')
        .next()
        .map(str::trim)
        .filter(|value| value.len() >= 2)
        .unwrap_or(file_name);
    Some(stem.to_string())
}

pub(super) fn content_excerpt_summary_directory_log_slice_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || !route
            .output_contract
            .semantic_kind
            .is_content_excerpt_summary()
    {
        return None;
    }
    let spec = route_content_slice_spec(route)?;
    let path = explicit_log_file_target_under_directory_locator(route, auto_locator_path)?;
    let root = Path::new(&path)
        .parent()
        .map(|parent| parent.display().to_string())?;
    let mut read_args = serde_json::json!({
        "action": "read_text_range",
        "path": path.clone()
    });
    if let Some(obj) = read_args.as_object_mut() {
        apply_content_slice_spec_to_read_args(obj, Some(spec), "tail", 80);
    }
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "find_entries",
                "root": root,
                "pattern": filename_prefix_pattern(&path).unwrap_or_else(|| path.clone()),
                "target_kind": "file",
                "max_results": 50
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: read_args,
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s1".to_string(), "s2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn generic_path_content_log_analyze_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || !log_analyze_is_enabled(state)
    {
        return None;
    }
    let route = route_result?;
    let path = generic_path_content_log_analyze_target_path(route_result, auto_locator_path)?;
    if !contract_allows_log_analyze_for_path(route, &path) {
        return None;
    }
    let actions = vec![
        AgentAction::CallSkill {
            skill: "log_analyze".to_string(),
            args: serde_json::json!({
                "path": path,
                "max_matches": 50
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn route_allows_single_file_content_understanding(route: &RouteResult) -> bool {
    matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::DocumentHeading
            | crate::OutputSemanticKind::ExcerptKindJudgment
    ) && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
}

pub(super) fn single_file_content_understanding_target_path(
    state: &AppState,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify || !route_allows_single_file_content_understanding(route) {
        return None;
    }
    let mut candidates = Vec::new();
    for candidate in auto_locator_path
        .into_iter()
        .chain(std::iter::once(route.output_contract.locator_hint.as_str()))
        .chain(route_locator_targets(route).iter().map(String::as_str))
    {
        let candidate = candidate.trim();
        if !candidate.is_empty()
            && !candidates
                .iter()
                .any(|existing: &String| existing == candidate)
        {
            candidates.push(candidate.to_string());
        }
    }
    for source in [route.resolved_intent.as_str(), route.route_reason.as_str()] {
        for path in collect_existing_file_targets_from_text(state, source) {
            if !candidates.iter().any(|existing| existing == &path) {
                candidates.push(path);
            }
        }
    }
    candidates.into_iter().find_map(|candidate| {
        let path = resolve_workspace_path(&state.skill_rt.workspace_root, &candidate);
        path.is_file().then(|| path.display().to_string())
    })
}

pub(super) fn content_excerpt_summary_auto_locator_observation_plan(
    state: &AppState,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    let semantic_kind = route.output_contract.semantic_kind;
    let path =
        single_file_content_understanding_target_path(state, route_result, auto_locator_path)?;
    let mut actions = Vec::new();
    let is_excerpt_kind_judgment = semantic_kind == crate::OutputSemanticKind::ExcerptKindJudgment;
    if is_excerpt_kind_judgment || repo_text_artifact_prefers_bounded_fs_read(&path) {
        let spec = route_result.and_then(route_content_slice_spec);
        let allow_default_head = matches!(
            Some(semantic_kind),
            Some(crate::OutputSemanticKind::DocumentHeading)
        ) || is_excerpt_kind_judgment
            || repo_prompt_artifact_allows_default_head(&path);
        if spec.is_none() && !allow_default_head {
            return None;
        }
        let mut args = serde_json::json!({
            "action": "read_text_range",
            "path": path
        });
        if let Some(obj) = args.as_object_mut() {
            let default_n = if is_excerpt_kind_judgment { 80 } else { 120 };
            apply_content_slice_spec_to_read_args(obj, spec, "head", default_n);
        }
        actions.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args,
        });
        return Some(actions);
    }
    if !doc_parse_supported_path(&path) {
        return None;
    }
    actions.push(AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: serde_json::json!({
            "action": "parse_doc",
            "path": path,
            "max_chars": 12000,
            "include_metadata": true
        }),
    });
    Some(actions)
}

pub(super) fn repo_text_artifact_prefers_bounded_fs_read(path: &str) -> bool {
    let path = Path::new(path);
    let Some(ext) = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.trim().to_ascii_lowercase())
    else {
        return false;
    };
    if !matches!(
        ext.as_str(),
        "md" | "txt" | "toml" | "json" | "yaml" | "yml" | "rs"
    ) {
        return false;
    }
    path.components().any(|component| {
        matches!(
            component,
            std::path::Component::Normal(value)
                if matches!(
                    value.to_str(),
                    Some("prompts" | "crates" | "configs" | "docker" | "scripts" | "UI" | "docs")
                )
        )
    })
}

pub(super) fn repo_prompt_artifact_allows_default_head(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    normalized.contains("/prompts/layers/generated/skills/") && normalized.ends_with(".md")
}

pub(super) fn content_excerpt_summary_auto_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if !matches!(
        route_result.map(|route| route.output_contract.semantic_kind),
        Some(
            crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::DocumentHeading
                | crate::OutputSemanticKind::ExcerptKindJudgment
        )
    ) {
        return None;
    }
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = content_excerpt_summary_auto_locator_observation_plan(
        state,
        route_result,
        auto_locator_path,
    )?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let mut actions = ensure_content_excerpt_summary_has_bounded_content(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    if matches!(
        route_result.map(|route| route.output_contract.semantic_kind),
        Some(crate::OutputSemanticKind::ExcerptKindJudgment)
    ) && !actions
        .iter()
        .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        let evidence_refs = observation_action_evidence_refs(&actions);
        if !evidence_refs.is_empty() {
            actions.push(AgentAction::SynthesizeAnswer {
                evidence_refs: evidence_refs.clone(),
            });
            actions.push(AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            });
            info!(
                "plan_insert_excerpt_kind_judgment_synthesis refs={}",
                evidence_refs.join(",")
            );
        }
    }
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn archive_basic_enabled_for_planning(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("archive_basic")
}

pub(super) fn archive_list_auto_locator_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return None;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| is_supported_archive_path(path))
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        })
        .filter(|path| is_supported_archive_path(path))
        .filter(|path| route_allows_archive_list_auto_locator(route, path))
        .map(ToString::to_string)
}

pub(super) fn archive_read_locator_parts(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<(String, String)> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return None;
    }

    let hint = route.output_contract.locator_hint.trim();
    let hint_parts =
        if route.output_contract.semantic_kind == crate::OutputSemanticKind::ArchiveRead {
            hint.split('|')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
    let auto_archive = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| is_supported_archive_path(path))
        .map(str::to_string);
    let hint_archive = hint_parts
        .first()
        .copied()
        .or_else(|| (!hint.is_empty()).then_some(hint))
        .map(str::to_string);
    let text_archive = archive_path_target_for_route_or_text(route, current_user_text);
    let archive = auto_archive
        .or_else(|| choose_archive_path_candidate(hint_archive, text_archive))
        .filter(|path| is_supported_archive_path(path))?;

    let member = if route.output_contract.semantic_kind == crate::OutputSemanticKind::ArchiveRead {
        let mut parts = hint_parts;
        if parts.len() >= 2 {
            if parts
                .first()
                .is_some_and(|part| is_supported_archive_path(part))
            {
                parts.remove(0);
            }
            Some(parts.join("/"))
        } else {
            archive_entry_target_for_route_or_text(route, current_user_text, &archive)
        }
    } else if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
            | crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
    ) {
        archive_entry_target_for_route_or_text(route, current_user_text, &archive)
    } else {
        return None;
    }?;

    if !archive_member_path_is_safe(&member) {
        return None;
    }
    Some((archive, member))
}

pub(super) fn choose_archive_path_candidate(
    hint_archive: Option<String>,
    text_archive: Option<String>,
) -> Option<String> {
    match (hint_archive, text_archive) {
        (Some(hint), Some(text)) if archive_path_candidate_is_more_specific_match(&hint, &text) => {
            Some(text)
        }
        (Some(hint), _) => Some(hint),
        (None, Some(text)) => Some(text),
        (None, None) => None,
    }
}

pub(super) fn archive_path_candidate_is_more_specific_match(hint: &str, text: &str) -> bool {
    let hint = hint.trim();
    let text = text.trim();
    if hint.is_empty() || text.is_empty() || hint.eq_ignore_ascii_case(text) {
        return false;
    }
    let hint_name = Path::new(hint)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(hint);
    let text_name = Path::new(text)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(text);
    hint_name.eq_ignore_ascii_case(text_name)
        && !hint.contains('/')
        && !hint.contains('\\')
        && (text.contains('/') || text.contains('\\'))
}

pub(super) fn archive_path_target_for_route_or_text(
    route: &RouteResult,
    current_user_text: &str,
) -> Option<String> {
    for text in [current_user_text, route.resolved_intent.as_str()] {
        for locator in
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        {
            let candidate = locator.locator_hint.trim();
            if is_supported_archive_path(candidate) {
                return Some(candidate.to_string());
            }
        }
    }
    for text in [current_user_text, route.resolved_intent.as_str()] {
        for filename in crate::delivery_utils::extract_filename_candidates(text) {
            let candidate = filename.trim();
            if is_supported_archive_path(candidate) {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

pub(super) fn archive_member_path_is_safe(member: &str) -> bool {
    let member = member.trim();
    if member.is_empty() {
        return false;
    }
    let path = Path::new(member);
    !path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

pub(super) fn has_archive_read_observation(
    loop_state: &LoopState,
    archive: &str,
    member: &str,
) -> bool {
    loop_state.executed_step_results.iter().rev().any(|step| {
        if !step.is_ok() || step.skill != "archive_basic" {
            return false;
        }
        let Some(output) = step.output.as_deref() else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(output) else {
            return false;
        };
        value.get("action").and_then(Value::as_str) == Some("read")
            && value
                .get("archive")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|observed| observed == archive)
            && value
                .get("member")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|observed| observed == member)
    })
}

pub(super) fn archive_read_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<PlanResult> {
    if !archive_basic_enabled_for_planning(state) {
        return None;
    }
    let (archive, member) =
        archive_read_locator_parts(route_result, auto_locator_path, current_user_text)?;
    if has_archive_read_observation(loop_state, &archive, &member) {
        return None;
    }
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: serde_json::json!({
            "action": "read",
            "archive": archive,
            "member": member,
        }),
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        if loop_state.round_no <= 1 {
            PlanKind::Single
        } else {
            PlanKind::Incremental
        },
        &actions,
    ))
}

pub(super) fn archive_unpack_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    if loop_state.has_tool_or_skill_output || !archive_basic_enabled_for_planning(state) {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ArchiveUnpack
    {
        return None;
    }
    let (archive, dest) = archive_unpack_pair_for_route(route)?;
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: serde_json::json!({
            "action": "unpack",
            "archive": archive,
            "dest": dest,
        }),
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        if loop_state.round_no <= 1 {
            PlanKind::Single
        } else {
            PlanKind::Incremental
        },
        &actions,
    ))
}

pub(super) fn archive_pack_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.has_tool_or_skill_output || !archive_basic_enabled_for_planning(state) {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
    {
        return None;
    }
    let (source, archive) = archive_pack_pair_for_route_or_text(
        &state.skill_rt.workspace_root,
        route,
        user_text,
        original_user_text,
        auto_locator_path,
    )?;
    let action = AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: serde_json::json!({
            "action": "pack",
            "source": source,
            "archive": archive,
            "format": archive_format_for_path(&archive),
        }),
    };
    let AgentAction::CallSkill { skill, args } = &action else {
        return None;
    };
    if crate::contract_matrix::action_policy_for_output_contract(
        Some(&route.output_contract),
        skill,
        args,
    )
    .is_some_and(|policy| !policy.is_allowed())
    {
        return None;
    }
    let actions = vec![action];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        if loop_state.round_no <= 1 {
            PlanKind::Single
        } else {
            PlanKind::Incremental
        },
        &actions,
    ))
}

pub(super) fn route_allows_archive_list_auto_locator(
    route: &RouteResult,
    archive_path: &str,
) -> bool {
    match route.output_contract.semantic_kind {
        crate::OutputSemanticKind::ArchiveRead => false,
        crate::OutputSemanticKind::ExistenceWithPath => {
            archive_entry_target_for_route_or_text(route, &route.resolved_intent, archive_path)
                .is_some()
        }
        crate::OutputSemanticKind::ArchiveList | crate::OutputSemanticKind::ScalarCount => true,
        _ => route_expects_terminal_user_answer(route),
    }
}

pub(super) fn archive_list_auto_locator_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || !archive_basic_enabled_for_planning(state)
    {
        return None;
    }
    let archive = archive_list_auto_locator_target_path(route_result, auto_locator_path)?;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: serde_json::json!({
                "action": "list",
                "archive": archive,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn transform_skill_enabled_for_planning(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("transform")
}

pub(super) fn inline_json_transform_args_from_text(text: &str) -> Option<Value> {
    let explicit_transform = crate::intent::surface_signals::inline_json_transform_request(text)
        .then(|| {
            json_values_any(text)
                .into_iter()
                .rev()
                .filter_map(explicit_transform_args_from_value)
                .next()
                .map(normalize_transform_args)
        })
        .flatten();
    explicit_transform.or_else(|| derive_single_object_rename_args_from_text(text))
}

pub(super) fn explicit_transform_args_from_value(value: Value) -> Option<Value> {
    let args = value
        .as_object()
        .filter(|obj| obj.get("skill").and_then(Value::as_str) == Some("transform"))
        .and_then(|obj| obj.get("args").cloned())
        .unwrap_or(value);
    let obj = args.as_object()?;
    let has_structured_input = obj.contains_key("data")
        || obj.contains_key("records")
        || obj.contains_key("csv_text")
        || obj.contains_key("csv");
    let has_structured_ops = obj
        .get("ops")
        .and_then(Value::as_array)
        .is_some_and(|ops| !ops.is_empty());
    (has_structured_input && has_structured_ops).then_some(args)
}

pub(super) fn json_values_any(text: &str) -> Vec<Value> {
    json_values_any_raw(text)
        .into_iter()
        .map(|(_, value)| value)
        .collect()
}

pub(super) fn json_values_any_raw(text: &str) -> Vec<(String, Value)> {
    let bytes = text.as_bytes();
    let mut values = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let opener = bytes[i];
        if opener != b'{' && opener != b'[' {
            i += 1;
            continue;
        }
        let start = i;
        let mut stack = vec![opener];
        let mut in_string = false;
        let mut escaped = false;
        let mut j = i + 1;
        let mut consumed_until = None;
        while j < bytes.len() {
            let c = bytes[j];
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    in_string = false;
                }
                j += 1;
                continue;
            }
            match c {
                b'"' => in_string = true,
                b'{' | b'[' => stack.push(c),
                b'}' | b']' => {
                    let Some(last) = stack.pop() else {
                        break;
                    };
                    let matched = matches!((last, c), (b'{', b'}') | (b'[', b']'));
                    if !matched {
                        break;
                    }
                    if stack.is_empty() {
                        let raw = &text[start..=j];
                        if let Ok(value) = serde_json::from_str::<Value>(raw) {
                            values.push((raw.to_string(), value));
                            consumed_until = Some(j + 1);
                        }
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        i = consumed_until.unwrap_or(start + 1);
    }
    values
}
