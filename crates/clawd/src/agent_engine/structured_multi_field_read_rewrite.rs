use super::*;

pub(super) fn rewrite_structured_multi_field_read_plan_to_read_fields(
    route_result: Option<&RouteResult>,
    user_text: &str,
    allow_route_resolved_intent_selector: bool,
    _plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract_marker_is(crate::OutputSemanticKind::RecentArtifactsJudgment)
        || !route.output_contract.requires_content_evidence
        || actions.iter().any(action_is_structured_config_validation)
        || actions.iter().any(action_is_structured_scalar_field_read)
        || !actions.iter().any(action_observes_structured_source)
        || actions.iter().any(|action| {
            !action_observes_structured_source(action)
                && !matches!(
                    action,
                    AgentAction::SynthesizeAnswer { .. }
                        | AgentAction::Respond { .. }
                        | AgentAction::Think { .. }
                )
        })
    {
        return actions;
    }
    let Some(path) = structured_scalar_field_read_target_path(route, auto_locator_path, &actions)
    else {
        return actions;
    };
    let field_paths = structured_current_turn_field_selectors(
        route,
        user_text,
        allow_route_resolved_intent_selector,
        Some(&path),
    );
    if field_paths.len() < 2 {
        return actions;
    }

    info!(
        "plan_rewrite_structured_multi_field_read_to_config_basic path={} fields={:?}",
        crate::truncate_for_log(&path),
        field_paths
    );
    vec![config_basic_read_fields_action(path, field_paths)]
}

pub(super) fn action_observes_structured_source(action: &AgentAction) -> bool {
    action_observes_bounded_file_content(action) || action_is_readonly_config_observation(action)
}

pub(super) fn action_is_structured_scalar_field_read(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            (skill.eq_ignore_ascii_case("config_basic")
                && matches!(
                    action_name.to_ascii_lowercase().as_str(),
                    "read_field" | "read_fields"
                ))
                || (skill.eq_ignore_ascii_case("system_basic")
                    && matches!(
                        action_name.to_ascii_lowercase().as_str(),
                        "extract_field" | "extract_fields"
                    ))
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

pub(super) fn structured_scalar_field_read_target_path(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    route_locator_structured_config_path(route)
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            actions
                .iter()
                .find_map(planned_structured_config_observation_path)
                .map(ToString::to_string)
        })
        .or_else(|| {
            actions
                .iter()
                .find_map(planned_bounded_file_read_path)
                .map(ToString::to_string)
        })
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .or_else(|| {
            actions.iter().find_map(
                super::super::planning_path_metadata::planned_single_path_metadata_facts_path,
            )
        })
        .filter(|path| path_has_structured_document_extension(path))
}

pub(super) fn structured_scalar_field_selector(
    route: &RouteResult,
    user_text: &str,
    allow_route_resolved_intent_selector: bool,
    plan_context: Option<&str>,
    target_path: Option<&str>,
) -> Option<String> {
    if let Some(selector) = route
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .and_then(normalize_contract_structured_field_selector)
    {
        return Some(selector);
    }
    structured_field_selectors(
        route,
        user_text,
        allow_route_resolved_intent_selector,
        plan_context,
        target_path,
    )
    .into_iter()
    .next()
}

pub(super) fn normalize_contract_structured_field_selector(raw: &str) -> Option<String> {
    let selector = raw.trim();
    if selector.is_empty() || selector.chars().count() > 256 || selector.contains('\\') {
        return None;
    }
    if selector.starts_with('/') {
        let segments = selector
            .split('/')
            .skip(1)
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        return (!segments.is_empty()).then(|| segments.join("."));
    }
    if structured_field_selector_candidate_is_valid(selector)
        || schema_field_token_is_valid(selector)
    {
        return Some(selector.to_string());
    }
    None
}

pub(super) fn structured_current_turn_field_selectors(
    route: &RouteResult,
    user_text: &str,
    allow_route_resolved_intent_selector: bool,
    target_path: Option<&str>,
) -> Vec<String> {
    let route_resolved_intent_source =
        allow_route_resolved_intent_selector.then_some(route.resolved_intent.as_str());
    let current_turn_sources = [Some(user_text), route_resolved_intent_source];
    structured_field_selectors_from_sources(&current_turn_sources, target_path)
}

pub(super) fn structured_field_selectors(
    route: &RouteResult,
    user_text: &str,
    allow_route_resolved_intent_selector: bool,
    plan_context: Option<&str>,
    target_path: Option<&str>,
) -> Vec<String> {
    let selectors = structured_current_turn_field_selectors(
        route,
        user_text,
        allow_route_resolved_intent_selector,
        target_path,
    );
    if !selectors.is_empty() {
        return selectors;
    }

    let fallback_sources = [plan_context];
    structured_field_selectors_from_sources(&fallback_sources, target_path)
}

pub(super) fn structured_field_selectors_from_sources(
    sources: &[Option<&str>],
    target_path: Option<&str>,
) -> Vec<String> {
    let mut selectors = Vec::new();
    for text in sources.iter().flatten() {
        for candidate in extract_dotted_field_selectors_for_structured_target(text) {
            push_unique_selector(&mut selectors, candidate);
        }
    }
    if !selectors.is_empty() {
        return selectors;
    }

    if let Some(path) = target_path {
        for text in sources.iter().flatten() {
            for candidate in
                super::super::planning_structured_field_exact::extract_exact_structured_field_path_selectors(path, text)
            {
                push_unique_selector(&mut selectors, candidate);
            }
        }
        if !selectors.is_empty() {
            return selectors;
        }

        for text in sources.iter().flatten() {
            for candidate in extract_schema_identity_field_selectors(path, text) {
                push_unique_selector(&mut selectors, candidate);
            }
        }
        if !selectors.is_empty() {
            return selectors;
        }

        for text in sources.iter().flatten() {
            for candidate in extract_schema_backed_field_selectors(path, text) {
                push_unique_selector(&mut selectors, candidate);
            }
        }
        selectors = prefer_non_locator_component_selectors(path, selectors);
    }

    selectors
}

pub(super) fn prefer_non_locator_component_selectors(
    target_path: &str,
    selectors: Vec<String>,
) -> Vec<String> {
    if selectors.len() < 2 {
        return selectors;
    }
    let locator_components = target_path_component_tokens(target_path);
    if locator_components.is_empty() {
        return selectors;
    }
    let filtered = selectors
        .iter()
        .filter(|selector| {
            let token = selector.trim();
            token.contains('.') || !locator_components.contains(&token.to_ascii_lowercase())
        })
        .cloned()
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        selectors
    } else {
        filtered
    }
}

pub(super) fn target_path_component_tokens(target_path: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    for component in Path::new(target_path).components() {
        let Some(text) = component.as_os_str().to_str().map(str::trim) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        tokens.insert(text.to_ascii_lowercase());
        if let Some(stem) = Path::new(text)
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            tokens.insert(stem.to_ascii_lowercase());
        }
    }
    tokens
}

pub(super) fn route_allows_structured_field_token_fallback(route: &RouteResult) -> bool {
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
    {
        return false;
    }
    if route.output_contract_is_unclassified()
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
        && !route.output_contract.locator_hint.trim().is_empty()
    {
        return true;
    }
    [
        "single_path_field_extraction_semantic_kind_none_is_valid",
        "contract_valid_minor_repair_fields_only",
        "structured_field_selector_requires_scalar_value",
        "structured_keys_scalar_response_requires_field_value",
        "structured_identifier_presence_requires_content_evidence",
    ]
    .iter()
    .any(|marker| route_reason_has_structural_marker(route, marker))
}

pub(super) fn structured_scalar_field_selector_from_structural_candidates(
    state: &AppState,
    route: &RouteResult,
    user_text: &str,
    allow_route_resolved_intent_selector: bool,
    plan_context: Option<&str>,
    target_path: &str,
) -> Option<String> {
    if !route_allows_structured_field_token_fallback(route)
        || !path_has_structured_document_extension(target_path)
    {
        return None;
    }
    let route_resolved_intent_source =
        allow_route_resolved_intent_selector.then_some(route.resolved_intent.as_str());
    let current_turn_sources = [Some(user_text), route_resolved_intent_source];
    structured_scalar_field_selector_from_candidate_sources(
        state,
        &current_turn_sources,
        target_path,
    )
    .or_else(|| {
        let fallback_sources = [plan_context];
        structured_scalar_field_selector_from_candidate_sources(
            state,
            &fallback_sources,
            target_path,
        )
    })
}

pub(super) fn structured_scalar_field_selector_from_candidate_sources(
    state: &AppState,
    sources: &[Option<&str>],
    target_path: &str,
) -> Option<String> {
    let current = resolve_workspace_path(&state.skill_rt.workspace_root, target_path);
    let mut selectors = Vec::new();
    for text in sources.iter().flatten() {
        for token in schema_field_candidate_tokens_without_filename_extensions(text) {
            let fields = vec![token.clone()];
            let token_matches_target = structured_file_has_all_fields(&current, &fields)
                || find_structured_field_candidate(
                    &state.skill_rt.workspace_root,
                    &current,
                    &fields,
                    state.skill_rt.locator_scan_max_files,
                )
                .is_some();
            if token_matches_target {
                push_unique_selector(&mut selectors, token);
            }
        }
    }
    if selectors.len() == 1 {
        selectors.into_iter().next()
    } else {
        selectors.clear();
        for text in sources.iter().flatten() {
            for selector in extract_schema_identity_presence_selectors(&current, text) {
                push_unique_selector(&mut selectors, selector);
            }
        }
        (selectors.len() == 1).then(|| selectors.remove(0))
    }
}

pub(super) fn push_unique_selector(out: &mut Vec<String>, candidate: String) {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return;
    }
    if out
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(candidate))
    {
        return;
    }
    out.push(candidate.to_string());
}

pub(super) fn structured_field_selector_candidate_is_valid(candidate: &str) -> bool {
    let token = candidate.trim();
    !token.is_empty()
        && !token.contains('/')
        && !token.contains('\\')
        && !filename_candidate_has_document_extension(token)
        && !path_has_structured_document_extension(token)
        && !crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
}

pub(super) fn extract_dotted_field_selectors_for_structured_target(text: &str) -> Vec<String> {
    static DOTTED_SELECTOR_RE: OnceLock<Regex> = OnceLock::new();
    let re = DOTTED_SELECTOR_RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z_$][A-Za-z0-9_$-]*(?:\.[A-Za-z_$][A-Za-z0-9_$-]*)+\b")
            .expect("valid dotted selector regex")
    });
    let mut out = Vec::new();
    for candidate in re.find_iter(text) {
        let token = candidate.as_str().trim();
        if structured_field_selector_candidate_is_valid(token) {
            push_unique_selector(&mut out, token.to_string());
        }
    }
    out
}

pub(super) fn extract_schema_backed_field_selectors(path: &str, text: &str) -> Vec<String> {
    let Some(value) = parse_structured_file_value(Path::new(path)) else {
        return Vec::new();
    };
    let index = structured_field_leaf_index(&value);
    let tokens = schema_field_candidate_tokens_without_filename_extensions(text);
    let mut out = Vec::new();
    for token in &tokens {
        let lower = token.to_ascii_lowercase();
        if let Some(paths) = index.get(&lower) {
            if paths.iter().any(|path| path.eq_ignore_ascii_case(&token)) {
                push_unique_selector(&mut out, token.clone());
                continue;
            }
            if paths.len() == 1 {
                push_unique_selector(&mut out, paths[0].clone());
                continue;
            }
            let segment_matched = paths
                .iter()
                .filter(|path| structured_field_path_segments_match_tokens(path, &tokens))
                .collect::<Vec<_>>();
            if segment_matched.len() == 1 {
                push_unique_selector(&mut out, (*segment_matched[0]).clone());
                continue;
            }
        }
        let suffix_matched = structured_field_suffix_matches(&index, &lower)
            .into_iter()
            .filter(|path| {
                !out.iter()
                    .any(|existing| existing.eq_ignore_ascii_case(path))
            })
            .collect::<Vec<_>>();
        if suffix_matched.len() == 1 {
            push_unique_selector(&mut out, suffix_matched[0].clone());
        }
    }
    out
}

pub(super) fn extract_schema_identity_field_selectors(path: &str, text: &str) -> Vec<String> {
    let Some(value) = parse_structured_file_value(Path::new(path)) else {
        return Vec::new();
    };
    let tokens = schema_field_candidate_tokens(text);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    collect_schema_identity_field_selectors(&value, &tokens, &mut out);
    out
}

pub(super) fn extract_schema_identity_presence_selectors(path: &Path, text: &str) -> Vec<String> {
    let Some(value) = parse_structured_file_value(path) else {
        return Vec::new();
    };
    let tokens = schema_field_candidate_tokens(text);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    collect_schema_identity_presence_selectors(&value, &tokens, &mut out);
    out
}

pub(super) fn structured_field_path_resolves_scalar_value(path: &str, field_path: &str) -> bool {
    let Some(value) = parse_structured_file_value(Path::new(path)) else {
        return false;
    };
    lookup_structured_field_value_with_identity(&value, field_path).is_some_and(|value| {
        matches!(
            value,
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
        )
    })
}

pub(super) fn lookup_structured_field_value_with_identity<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<&'a Value> {
    lookup_structured_field_value(value, field_path)
        .or_else(|| lookup_structured_array_identity_field_value(value, field_path))
}

pub(super) fn lookup_structured_array_identity_field_value<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<&'a Value> {
    let segments = field_path.split('.').collect::<Vec<_>>();
    if segments.len() < 2 {
        return None;
    }
    let selector_value = segments.first()?.trim();
    if selector_value.is_empty() || selector_value.contains('[') || selector_value.contains(']') {
        return None;
    }
    let nested_field_path = segments[1..].join(".");
    if nested_field_path.trim().is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    collect_structured_array_identity_field_values(
        value,
        selector_value,
        &nested_field_path,
        &mut matches,
    );
    (matches.len() == 1).then(|| matches.remove(0))
}

pub(super) fn collect_structured_array_identity_field_values<'a>(
    value: &'a Value,
    selector_value: &str,
    nested_field_path: &str,
    out: &mut Vec<&'a Value>,
) {
    match value {
        Value::Object(map) => {
            for child in map.values() {
                collect_structured_array_identity_field_values(
                    child,
                    selector_value,
                    nested_field_path,
                    out,
                );
            }
        }
        Value::Array(items) => {
            for item in items {
                if structured_array_item_matches_identity(item, selector_value) {
                    if let Some(nested_value) =
                        lookup_structured_field_value(item, nested_field_path)
                    {
                        out.push(nested_value);
                    }
                }
                collect_structured_array_identity_field_values(
                    item,
                    selector_value,
                    nested_field_path,
                    out,
                );
            }
        }
        _ => {}
    }
}

pub(super) fn structured_array_item_matches_identity(item: &Value, selector_value: &str) -> bool {
    item.as_object().is_some_and(|map| {
        ["name", "id", "key"].iter().any(|identity_key| {
            map.get(*identity_key)
                .and_then(Value::as_str)
                .is_some_and(|value| value == selector_value)
        })
    })
}

pub(super) fn collect_schema_identity_field_selectors(
    value: &Value,
    tokens: &[String],
    out: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            for child in map.values() {
                collect_schema_identity_field_selectors(child, tokens, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(obj) = item.as_object() {
                    for identity_key in ["name", "id", "key"] {
                        let Some(identity_value) = obj.get(identity_key).and_then(Value::as_str)
                        else {
                            continue;
                        };
                        if !schema_text_tokens_contain(tokens, identity_value) {
                            continue;
                        }
                        for field_key in obj.keys() {
                            if field_key == identity_key || !schema_field_token_is_valid(field_key)
                            {
                                continue;
                            }
                            if schema_text_tokens_contain(tokens, field_key) {
                                push_unique_selector(out, format!("{identity_value}.{field_key}"));
                            }
                        }
                    }
                }
                collect_schema_identity_field_selectors(item, tokens, out);
            }
        }
        _ => {}
    }
}

pub(super) fn collect_schema_identity_presence_selectors(
    value: &Value,
    tokens: &[String],
    out: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            for child in map.values() {
                collect_schema_identity_presence_selectors(child, tokens, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(obj) = item.as_object() {
                    for identity_key in ["name", "id", "key"] {
                        let Some(identity_value) = obj.get(identity_key).and_then(Value::as_str)
                        else {
                            continue;
                        };
                        if schema_text_tokens_contain(tokens, identity_value)
                            && schema_field_token_is_valid(identity_key)
                        {
                            push_unique_selector(out, format!("{identity_value}.{identity_key}"));
                        }
                    }
                }
                collect_schema_identity_presence_selectors(item, tokens, out);
            }
        }
        _ => {}
    }
}

pub(super) fn schema_text_tokens_contain(tokens: &[String], needle: &str) -> bool {
    tokens
        .iter()
        .any(|token| token.eq_ignore_ascii_case(needle))
}

pub(super) fn structured_field_leaf_index(value: &Value) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    collect_structured_field_leaf_index(value, "", &mut out);
    out
}

pub(super) fn collect_structured_field_leaf_index(
    value: &Value,
    prefix: &str,
    out: &mut HashMap<String, Vec<String>>,
) {
    let Some(obj) = value.as_object() else {
        return;
    };
    for (key, child) in obj {
        if !schema_field_token_is_valid(key) {
            continue;
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let lower = key.to_ascii_lowercase();
        out.entry(lower).or_insert_with(Vec::new).push(path.clone());
        collect_structured_field_leaf_index(child, &path, out);
    }
}

pub(super) fn schema_field_candidate_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$') {
            current.push(ch);
            continue;
        }
        push_schema_field_candidate_token(&mut out, &mut current);
    }
    push_schema_field_candidate_token(&mut out, &mut current);
    out
}

pub(super) fn schema_field_candidate_tokens_without_filename_extensions(text: &str) -> Vec<String> {
    let extension_tokens = filename_extension_tokens_from_text(text);
    schema_field_candidate_tokens(text)
        .into_iter()
        .filter(|token| {
            !extension_tokens
                .iter()
                .any(|extension| extension.eq_ignore_ascii_case(token))
        })
        .collect()
}

pub(super) fn filename_extension_tokens_from_text(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for filename in crate::delivery_utils::extract_filename_candidates(text) {
        if !filename_candidate_has_document_extension(&filename)
            && !path_has_structured_document_extension(&filename)
        {
            continue;
        }
        let Some(extension) = Path::new(&filename)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| schema_field_token_is_valid(value))
        else {
            continue;
        };
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(extension))
        {
            out.push(extension.to_string());
        }
    }
    out
}

pub(super) fn structured_field_path_segments_match_tokens(path: &str, tokens: &[String]) -> bool {
    path.split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .all(|segment| schema_text_tokens_contain(tokens, segment))
}

pub(super) fn structured_field_suffix_matches(
    index: &HashMap<String, Vec<String>>,
    token_lower: &str,
) -> Vec<String> {
    let token = token_lower.trim();
    if token.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for (leaf, paths) in index {
        if !structured_field_leaf_suffix_matches(leaf, token) {
            continue;
        }
        for path in paths {
            push_unique_selector(&mut matches, path.clone());
        }
    }
    matches
}

pub(super) fn structured_field_leaf_suffix_matches(leaf: &str, token_lower: &str) -> bool {
    leaf.split(['_', '-', '$'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .next_back()
        .is_some_and(|segment| segment.eq_ignore_ascii_case(token_lower))
}

pub(super) fn push_schema_field_candidate_token(out: &mut Vec<String>, current: &mut String) {
    if schema_field_token_is_valid(current)
        && !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(current))
    {
        out.push(current.clone());
    }
    current.clear();
}

pub(super) fn schema_field_token_is_valid(token: &str) -> bool {
    !token.is_empty()
        && !token.contains('/')
        && !token.contains('\\')
        && !path_has_structured_document_extension(token)
        && !crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$'))
}

pub(super) fn config_basic_read_field_action(path: String, field_path: String) -> AgentAction {
    AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "read_field",
            "path": path,
            "field_path": field_path,
        }),
    }
}

pub(super) fn config_basic_read_fields_action(
    path: String,
    field_paths: Vec<String>,
) -> AgentAction {
    AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "read_fields",
            "path": path,
            "field_paths": field_paths,
        }),
    }
}

pub(super) fn parse_config_value_token(token: String) -> Option<Value> {
    if token.eq_ignore_ascii_case("true") {
        return Some(Value::Bool(true));
    }
    if token.eq_ignore_ascii_case("false") {
        return Some(Value::Bool(false));
    }
    if token.eq_ignore_ascii_case("null") {
        return Some(Value::Null);
    }
    if let Ok(value) = token.parse::<i64>() {
        return Some(Value::Number(value.into()));
    }
    if let Ok(value) = token.parse::<f64>() {
        if let Some(number) = serde_json::Number::from_f64(value) {
            return Some(Value::Number(number));
        }
    }
    if token
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':'))
    {
        return Some(Value::String(token));
    }
    None
}

pub(super) fn action_targets_config_edit(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallTool { tool, .. } | AgentAction::CallSkill { skill: tool, .. }
            if tool.eq_ignore_ascii_case("config_edit")
    )
}

pub(super) fn action_is_config_change_preview_observation(action: &AgentAction) -> bool {
    if action_is_readonly_config_observation(action) {
        return true;
    }
    let (skill, args) = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            (tool.as_str(), args)
        }
        _ => return false,
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    (skill.eq_ignore_ascii_case("config_basic")
        && action.eq_ignore_ascii_case("guard_rustclaw_config"))
        || (skill.eq_ignore_ascii_case("config_edit")
            && matches!(
                action.to_ascii_lowercase().as_str(),
                "guard_config" | "validate_config" | "read_back"
            ))
}

pub(super) fn action_is_readonly_config_observation(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            (tool.as_str(), args)
        }
        _ => return false,
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if skill.eq_ignore_ascii_case("config_basic") {
        return matches!(
            action,
            "read_field"
                | "read_fields"
                | "list_keys"
                | "validate"
                | "extract_field"
                | "extract_fields"
                | "structured_keys"
                | "validate_structured"
        );
    }
    skill.eq_ignore_ascii_case("system_basic")
        && matches!(
            action,
            "extract_field" | "extract_fields" | "structured_keys" | "validate_structured"
        )
}

pub(super) fn action_is_obvious_mutation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if tool.eq_ignore_ascii_case("config_edit") {
                return matches!(
                    action.as_str(),
                    "apply_config_change" | "apply_change" | "write_field" | "set_field"
                );
            }
            if tool.eq_ignore_ascii_case("fs_basic") || tool.eq_ignore_ascii_case("system_basic") {
                return action.contains("write")
                    || action.contains("append")
                    || action.contains("patch")
                    || action.contains("delete")
                    || action.contains("remove");
            }
            false
        }
        _ => false,
    }
}

pub(super) fn strip_unrequested_config_edit_actions(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !actions.iter().any(action_targets_config_edit)
        || current_turn_requests_config_edit(route_result, user_text, original_user_text, &actions)
    {
        return actions;
    }
    let before = actions.len();
    let stripped = actions
        .into_iter()
        .filter(|action| !action_targets_config_edit(action))
        .collect::<Vec<_>>();
    let dropped = before.saturating_sub(stripped.len());
    if dropped > 0 {
        info!("plan_strip_unrequested_config_edit_actions dropped={dropped}");
    }
    stripped
}

pub(super) fn current_turn_requests_config_edit(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.needs_clarify || !route.is_execute_gate() {
        return false;
    }
    let request = original_user_text.unwrap_or(user_text).trim();
    if request.is_empty() {
        return false;
    }

    let config_actions = actions
        .iter()
        .filter(|action| action_targets_config_edit(action))
        .collect::<Vec<_>>();
    if route_has_config_change_contract(route)
        && !config_actions.is_empty()
        && config_actions
            .iter()
            .all(|action| config_edit_action_has_structured_config_contract(action))
    {
        return true;
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::ConfigRiskAssessment) {
        return !config_actions.is_empty()
            && config_actions
                .iter()
                .all(|action| config_edit_action_is_route_guard(action, route));
    }
    if route.output_contract.requires_content_evidence
        && !config_actions.is_empty()
        && config_actions
            .iter()
            .all(|action| config_edit_action_is_route_guard(action, route))
    {
        return true;
    }
    !config_actions.is_empty()
        && config_actions
            .iter()
            .all(|action| config_edit_action_has_current_structural_anchor(action, request))
}

fn route_has_config_change_contract(route: &RouteResult) -> bool {
    route.output_contract_marker_is(crate::OutputSemanticKind::ConfigMutation)
        || crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["config"],
            &[
                "apply_change",
                "apply_config_change",
                "plan_change",
                "plan_config_change",
                "set_field",
                "write_field",
            ],
        )
}

fn config_edit_action_has_structured_config_contract(action: &AgentAction) -> bool {
    let (tool, args) = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            (tool.as_str(), args)
        }
        _ => return false,
    };
    if !tool.eq_ignore_ascii_case("config_edit") {
        return false;
    }
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    match action_name {
        "plan_config_change"
        | "apply_config_change"
        | "apply_change"
        | "write_field"
        | "set_field" => {
            json_trimmed_string_arg(args, &["field_path", "field"]).is_some()
                && args.get("value").is_some_and(|value| !value.is_null())
        }
        "read_back" => json_trimmed_string_arg(args, &["field_path", "field"]).is_some(),
        "guard_config" | "validate_config" | "validate" => true,
        _ => false,
    }
}

pub(super) fn config_edit_action_is_route_guard(action: &AgentAction, route: &RouteResult) -> bool {
    let (tool, args) = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            (tool.as_str(), args)
        }
        _ => return false,
    };
    if !tool.eq_ignore_ascii_case("config_edit") {
        return false;
    }
    if args.get("action").and_then(Value::as_str).map(str::trim) != Some("guard_config") {
        return false;
    }
    let Some(path) = json_trimmed_string_arg(args, &["path", "file", "file_path", "config_path"])
    else {
        return false;
    };
    let locator = route.output_contract.locator_hint.trim();
    is_rustclaw_config_guard_path(&path)
        && (locator.is_empty()
            || is_rustclaw_config_guard_path(locator)
            || path
                .replace('\\', "/")
                .eq_ignore_ascii_case(&locator.replace('\\', "/")))
}

pub(super) fn config_edit_action_has_current_structural_anchor(
    action: &AgentAction,
    request: &str,
) -> bool {
    let (tool, args) = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            (tool.as_str(), args)
        }
        _ => return true,
    };
    if !tool.eq_ignore_ascii_case("config_edit") {
        return true;
    }
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if matches!(action_name, "guard_config" | "validate_config" | "validate") {
        return config_edit_path_is_anchored(args, request).unwrap_or(false);
    }

    config_edit_path_is_anchored(args, request).unwrap_or(false)
        && config_edit_field_is_anchored(args, request).unwrap_or(false)
        && config_edit_value_is_anchored(args, request).unwrap_or(true)
}

pub(super) fn config_edit_path_is_anchored(args: &Value, request: &str) -> Option<bool> {
    let path = json_trimmed_string_arg(args, &["path", "file", "file_path", "config_path"])?;
    let mut tokens = vec![path.clone(), path.replace('\\', "/")];
    if let Some(file_name) = Path::new(&path).file_name().and_then(|name| name.to_str()) {
        tokens.push(file_name.to_string());
    }
    Some(
        tokens
            .iter()
            .any(|token| structural_token_present(request, token)),
    )
}

pub(super) fn config_edit_field_is_anchored(args: &Value, request: &str) -> Option<bool> {
    let field_path = json_trimmed_string_arg(args, &["field_path", "field", "key"])?;
    let mut tokens = vec![field_path.clone()];
    if let Some(leaf) = field_path
        .rsplit('.')
        .next()
        .filter(|leaf| !leaf.is_empty())
    {
        tokens.push(leaf.to_string());
    }
    Some(
        tokens
            .iter()
            .any(|token| structural_token_present(request, token)),
    )
}

pub(super) fn config_edit_value_is_anchored(args: &Value, request: &str) -> Option<bool> {
    let token = scalar_value_anchor_token(args.get("value")?)?;
    Some(structural_token_present(request, &token))
}

pub(super) fn json_trimmed_string_arg(args: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn scalar_value_anchor_token(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("null".to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        Value::Array(_) | Value::Object(_) => None,
    }
}

pub(super) fn structural_token_present(text: &str, token: &str) -> bool {
    let token = token
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'))
        .replace('\\', "/");
    if token.is_empty() {
        return false;
    }
    text.replace('\\', "/")
        .to_ascii_lowercase()
        .contains(&token.to_ascii_lowercase())
}

pub(super) fn normalize_terminal_delivery_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    terminal_mixed_last_output_content: Option<String>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions =
        rewrite_observed_terminal_synthesis_concrete_respond(route_result, loop_state, actions);
    let actions = strip_pre_observation_synthesize_before_concrete_respond(loop_state, actions);
    let actions = rewrite_pre_observation_concrete_respond_to_placeholder(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions =
        rewrite_mixed_placeholder_observed_synthesis_respond(route_result, loop_state, actions);
    let actions =
        rewrite_mixed_placeholder_structured_output_respond(route_result, loop_state, actions);
    let actions = rewrite_terminal_synthesis_placeholder_respond(actions);
    let actions = strip_intermediate_synthesize_before_later_execution(actions);
    let actions = append_respond_for_terminal_synthesize_answer(actions);
    let actions = rewrite_terminal_placeholder_respond_to_synthesize_answer(loop_state, actions);
    let actions =
        strip_terminal_placeholder_respond_for_exact_listing_contract(route_result, actions);
    let actions = inject_synthesize_answer_for_bare_placeholder_respond(actions, user_text);
    let actions = append_synthesize_for_observation_only_terminal_answer(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions = restore_terminal_mixed_last_output_respond(
        route_result,
        terminal_mixed_last_output_content,
        actions,
    );
    let actions = strip_service_status_discussion_actions(route_result, actions);
    let actions = rewrite_mixed_file_token_prose_respond_to_synthesize_answer(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions = strip_redundant_make_dir_before_file_delivery_write(state, route_result, actions);
    let actions =
        append_file_token_after_generated_file_write_delivery(state, route_result, actions);
    let actions =
        append_file_token_after_existing_file_delivery_observation(state, route_result, actions);
    mark_missing_target_repairable_actions(state, route_result, actions)
}
