use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub(super) fn repair_structured_field_target_from_prompt(
    route_result: &mut crate::RouteResult,
    prompt: &str,
    workspace_root: &Path,
    default_locator_search_dir: &Path,
) {
    if route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::OneSentence
        )
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return;
    }
    if route_structured_target_path(route_result, workspace_root, default_locator_search_dir)
        .is_some()
    {
        return;
    }
    let Some((path, selector)) = unique_structured_scalar_field_pair_from_prompt(
        route_result,
        prompt,
        workspace_root,
        default_locator_search_dir,
    ) else {
        return;
    };
    if route_preserves_heterogeneous_observation_summary_contract(route_result) {
        route_result
            .output_contract
            .self_extension
            .structured_field_selector = Some(selector);
        return;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path.display().to_string();
    route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route_result
        .output_contract
        .self_extension
        .structured_field_selector = Some(selector);
    route_result
        .route_reason
        .push_str("; structured_field_target_from_prompt_repair");
}

fn set_effective_contract_marker(route_result: &mut crate::RouteResult, marker: &'static str) {
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route_result.route_reason.push_str("; ");
    route_result.route_reason.push_str(marker);
}

pub(super) fn repair_scalar_field_value_contract_for_locator_reply(
    route_result: &mut crate::RouteResult,
    prompt: &str,
) {
    if route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Strict
        )
    {
        return;
    }
    let marker_matches_field_value_request = [
        "contract_valid_minor_repair_fields_only",
        "single_path_field_extraction_semantic_kind_none_is_valid",
        "structured_field_target_from_prompt_repair",
        "structured_field_selector_requires_scalar_value",
        "structured_keys_scalar_response_requires_field_value",
    ]
    .iter()
    .any(|marker| super::route_reason_has_structural_marker(route_result, marker));
    let selector_declares_field_value_request = route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .is_some();
    if !marker_matches_field_value_request && !selector_declares_field_value_request {
        return;
    }
    repair_structured_field_selector_from_target(route_result, prompt);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let target_count = explicit_locator_target_count_excluding_structured_selector(
        prompt,
        route_result
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
    );
    let structured_refinement_present =
        surface.has_structured_target_refinement() || selector_declares_field_value_request;
    if target_count >= 2
        && structured_refinement_present
        && route_preserves_heterogeneous_observation_summary_contract(route_result)
    {
        set_effective_contract_marker(route_result, "contract:command_output_summary");
        if route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar {
            route_result.output_contract.response_shape = crate::OutputResponseShape::Strict;
        }
        route_result
            .route_reason
            .push_str("; multi_locator_structured_field_preserves_summary_contract");
        return;
    }
    if marker_matches_field_value_request
        && super::route_reason_has_structural_marker(route_result, "recent_scalar_equality_check")
    {
        route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route_result
            .route_reason
            .push_str("; scalar_field_value_contract_repair");
        return;
    }
    if (marker_matches_field_value_request || selector_declares_field_value_request)
        && target_count >= 2
        && structured_refinement_present
    {
        set_effective_contract_marker(route_result, "contract:recent_scalar_equality_check");
        if route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar {
            route_result.output_contract.response_shape = crate::OutputResponseShape::Strict;
        }
        route_result
            .route_reason
            .push_str("; scalar_field_pair_contract_repair");
        return;
    }
    if !(selector_declares_field_value_request
        || marker_matches_field_value_request
        || route_has_scalar_field_value_compatible_marker(route_result))
    {
        return;
    }
    route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route_result
        .route_reason
        .push_str("; scalar_field_value_contract_repair");
}

fn explicit_locator_target_count_excluding_structured_selector(
    prompt: &str,
    selector: Option<&str>,
) -> usize {
    let mut candidates = Vec::new();
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt)
    {
        let candidate = locator.locator_hint.trim();
        if selector
            .is_some_and(|selector| candidate_matches_structured_selector(candidate, selector))
        {
            continue;
        }
        push_unique_raw_candidate(&mut candidates, candidate.to_string());
    }
    for candidate in crate::intent::surface_signals::analyze_prompt_surface(prompt)
        .filename_candidates_excluding_field_selectors()
    {
        if selector
            .is_some_and(|selector| candidate_matches_structured_selector(&candidate, selector))
        {
            continue;
        }
        push_unique_raw_candidate(&mut candidates, candidate);
    }
    candidates.len()
}

fn route_preserves_heterogeneous_observation_summary_contract(
    route_result: &crate::RouteResult,
) -> bool {
    super::route_reason_has_structural_marker(route_result, "command_output_summary")
        || super::route_reason_has_structural_marker(route_result, "command_result_synthesis")
        || super::route_reason_has_structural_marker(
            route_result,
            "multi_locator_structured_field_preserves_summary_contract",
        )
}

fn route_has_scalar_field_value_compatible_marker(route_result: &crate::RouteResult) -> bool {
    [
        "structured_keys",
        "existence_with_path",
        "document_heading",
        "recent_scalar_equality_check",
    ]
    .iter()
    .any(|marker| super::route_reason_has_structural_marker(route_result, marker))
}

fn candidate_matches_structured_selector(candidate: &str, selector: &str) -> bool {
    let candidate = candidate.trim();
    let selector = selector.trim();
    !candidate.is_empty()
        && !selector.is_empty()
        && (candidate.eq_ignore_ascii_case(selector)
            || selector_refines_field(candidate, selector)
            || selector_refines_field(selector, candidate))
}

fn repair_structured_field_selector_from_target(
    route_result: &mut crate::RouteResult,
    prompt: &str,
) {
    let Some(target_path) = structured_target_path(route_result) else {
        return;
    };
    let Some(root_value) = parse_structured_file_value(&target_path) else {
        return;
    };
    let current_selector = route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let sources = [Some(prompt), Some(route_result.resolved_intent.as_str())];
    for candidate in sources
        .into_iter()
        .flatten()
        .flat_map(dotted_machine_field_tokens)
    {
        if current_selector
            .as_deref()
            .is_some_and(|selector| !selector_refines_field(&candidate, selector))
        {
            continue;
        }
        if lookup_structured_field_value(&root_value, &candidate).is_some_and(is_scalar_json_value)
        {
            route_result
                .output_contract
                .self_extension
                .structured_field_selector = Some(candidate);
            route_result
                .route_reason
                .push_str("; structured_field_selector_exact_target_repair");
            return;
        }
    }
}

fn unique_structured_scalar_field_pair_from_prompt(
    route_result: &crate::RouteResult,
    prompt: &str,
    workspace_root: &Path,
    default_locator_search_dir: &Path,
) -> Option<(PathBuf, String)> {
    let current_selector = route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let field_candidates = [prompt, route_result.resolved_intent.as_str()]
        .into_iter()
        .flat_map(dotted_machine_field_tokens)
        .filter(|candidate| {
            current_selector.as_deref().is_none_or(|selector| {
                candidate.eq_ignore_ascii_case(selector)
                    || selector_refines_field(candidate, selector)
            })
        })
        .collect::<Vec<_>>();
    if field_candidates.is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    for path in structured_file_candidates_from_prompt(
        prompt,
        route_result,
        workspace_root,
        default_locator_search_dir,
    ) {
        let Some(root_value) = parse_structured_file_value(&path) else {
            continue;
        };
        for candidate in &field_candidates {
            if lookup_structured_field_value(&root_value, candidate)
                .is_some_and(is_scalar_json_value)
            {
                push_unique_field_match(&mut matches, path.clone(), candidate.clone());
            }
        }
    }
    (matches.len() == 1).then(|| matches.remove(0))
}

fn structured_file_candidates_from_prompt(
    prompt: &str,
    route_result: &crate::RouteResult,
    workspace_root: &Path,
    default_locator_search_dir: &Path,
) -> Vec<PathBuf> {
    let mut raw_candidates = Vec::new();
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt)
    {
        push_unique_raw_candidate(&mut raw_candidates, locator.locator_hint);
    }
    for candidate in crate::intent::surface_signals::analyze_prompt_surface(prompt)
        .filename_candidates_excluding_field_selectors()
    {
        push_unique_raw_candidate(&mut raw_candidates, candidate);
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if !locator_hint.is_empty() {
        push_unique_raw_candidate(&mut raw_candidates, locator_hint.to_string());
    }

    let mut paths = Vec::new();
    for raw in raw_candidates {
        if let Some(path) =
            resolve_structured_file_candidate(&raw, workspace_root, default_locator_search_dir)
        {
            push_unique_path(&mut paths, path);
        }
    }
    paths
}

fn push_unique_raw_candidate(candidates: &mut Vec<String>, candidate: String) {
    let candidate = normalize_raw_locator_candidate_token(&candidate);
    if candidate.is_empty() || candidate.contains('\n') || candidate.contains('\r') {
        return;
    }
    if !candidates.iter().any(|existing| {
        existing.eq_ignore_ascii_case(&candidate)
            || raw_locator_candidates_equivalent(existing, &candidate)
    }) {
        candidates.push(candidate);
    }
}

fn normalize_raw_locator_candidate_token(candidate: &str) -> String {
    candidate
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim_end_matches(|ch| {
            matches!(
                ch,
                '.' | ',' | ';' | ':' | ')' | ']' | '}' | '。' | '，' | '；' | '：'
            )
        })
        .trim()
        .to_string()
}

fn raw_locator_candidates_equivalent(left: &str, right: &str) -> bool {
    let left = normalize_raw_locator_candidate_token(left);
    let right = normalize_raw_locator_candidate_token(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let left_name = Path::new(&left).file_name().and_then(|name| name.to_str());
    let right_name = Path::new(&right).file_name().and_then(|name| name.to_str());
    match (left_name, right_name) {
        (Some(left_name), Some(right_name)) => left_name.eq_ignore_ascii_case(right_name),
        _ => false,
    }
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn push_unique_field_match(matches: &mut Vec<(PathBuf, String)>, path: PathBuf, field: String) {
    if !matches
        .iter()
        .any(|(existing_path, existing_field)| existing_path == &path && existing_field == &field)
    {
        matches.push((path, field));
    }
}

fn route_structured_target_path(
    route_result: &crate::RouteResult,
    workspace_root: &Path,
    default_locator_search_dir: &Path,
) -> Option<PathBuf> {
    if !matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
    ) {
        return None;
    }
    resolve_structured_file_candidate(
        route_result.output_contract.locator_hint.trim(),
        workspace_root,
        default_locator_search_dir,
    )
}

fn resolve_structured_file_candidate(
    candidate: &str,
    workspace_root: &Path,
    default_locator_search_dir: &Path,
) -> Option<PathBuf> {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.contains('\n')
        || candidate.contains('\r')
        || candidate.contains("://")
    {
        return None;
    }
    let path = Path::new(candidate);
    if path.is_absolute() {
        return structured_existing_file_path(path);
    }
    structured_existing_file_path(&workspace_root.join(path)).or_else(|| {
        let default_candidate = default_locator_search_dir.join(path);
        (default_candidate != workspace_root.join(path))
            .then(|| structured_existing_file_path(&default_candidate))
            .flatten()
    })
}

fn structured_existing_file_path(path: &Path) -> Option<PathBuf> {
    path.is_file()
        .then(|| {
            path_has_structured_extension(path)
                .then(|| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
        })
        .flatten()
}

fn path_has_structured_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| matches!(ext.as_str(), "json" | "toml" | "yaml" | "yml"))
}

fn structured_target_path(route_result: &crate::RouteResult) -> Option<PathBuf> {
    if !matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
    ) {
        return None;
    }
    let locator = route_result.output_contract.locator_hint.trim();
    if locator.is_empty() || locator.contains('\n') {
        return None;
    }
    let path = Path::new(locator);
    path.is_file().then(|| path.to_path_buf())
}

fn parse_structured_file_value(path: &Path) -> Option<Value> {
    let contents = fs::read_to_string(path).ok()?;
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => serde_json::from_str(&contents).ok(),
        "toml" => toml::from_str::<toml::Value>(&contents)
            .ok()
            .and_then(|value| serde_json::to_value(value).ok()),
        _ => None,
    }
}

fn dotted_machine_field_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in
        text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$' | '.')))
    {
        let token = raw.trim_matches('.');
        if token.contains('.')
            && !token.contains('/')
            && !token.contains('\\')
            && token.split('.').all(machine_field_segment_is_valid)
            && !crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
            && !out
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(token))
        {
            out.push(token.to_string());
        }
    }
    out.sort_by_key(|token| std::cmp::Reverse(token.len()));
    out
}

fn machine_field_segment_is_valid(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$'))
}

fn selector_refines_field(candidate: &str, field: &str) -> bool {
    candidate.len() > field.len()
        && candidate
            .get(..field.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(field))
        && candidate
            .as_bytes()
            .get(field.len())
            .is_some_and(|byte| *byte == b'.')
}

fn lookup_structured_field_value<'a>(value: &'a Value, field_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in field_path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn is_scalar_json_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}
