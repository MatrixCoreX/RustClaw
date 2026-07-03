use serde_json::{json, Value};

pub(super) fn registry_capability_contract_observation(
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Option<Value> {
    if route_result.wants_file_delivery
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let capability_refs = registry_capability_refs_from_route(resolved_prompt, route_result);
    if capability_refs.is_empty() {
        return None;
    }
    let has_conflicting_contract = route_result.needs_clarify
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || route_result.output_contract.delivery_required
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::None;
    Some(json!({
        "source": "registry_capability_ref",
        "capability_refs": capability_refs,
        "has_conflicting_route_contract": has_conflicting_contract,
        "needs_clarify": route_result.needs_clarify,
        "locator_kind": route_result.output_contract.locator_kind.as_str(),
        "locator_hint": route_result.output_contract.locator_hint.trim(),
        "delivery_required": route_result.output_contract.delivery_required,
        "delivery_intent": route_result.output_contract.delivery_intent.as_str(),
        "response_shape": route_result.output_contract.response_shape.as_str(),
    }))
}

fn registry_capability_refs_from_route(
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Vec<String> {
    let mut refs = [
        route_result.route_reason.as_str(),
        route_result.resolved_intent.as_str(),
        resolved_prompt,
    ]
    .iter()
    .flat_map(|surface| {
        surface
            .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | '(' | ')'))
            .map(str::trim)
    })
    .filter_map(|part| {
        let capability = part.strip_prefix("capability_ref=")?.trim();
        crate::machine_capability_ref::is_valid_capability_ref_value(capability)
            .then_some(capability.to_string())
    })
    .collect::<Vec<_>>();
    refs.sort();
    refs.dedup();
    refs
}

pub(super) fn contract_repair_candidate_observations(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Vec<Value> {
    let mut candidates = Vec::new();
    if let Some(candidate) = command_observation_marker_contract_candidate(route_result) {
        candidates.push(candidate);
    }
    if let Some(candidate) = sqlite_path_excerpt_judgment_contract_candidate(
        state,
        prompt,
        resolved_prompt,
        route_result,
    ) {
        candidates.push(candidate);
    }
    if let Some(candidate) =
        sqlite_structured_version_contract_candidate(state, prompt, resolved_prompt, route_result)
    {
        candidates.push(candidate);
    }
    if let Some(candidate) = sqlite_structured_table_listing_contract_candidate(
        state,
        prompt,
        resolved_prompt,
        route_result,
    ) {
        candidates.push(candidate);
    }
    if let Some(candidate) =
        sqlite_route_reason_table_contract_candidate(state, prompt, resolved_prompt, route_result)
    {
        candidates.push(candidate);
    }
    if let Some(candidate) =
        config_validation_findings_contract_candidate(state, prompt, resolved_prompt, route_result)
    {
        candidates.push(candidate);
    }
    candidates
}

fn contract_candidate_json(
    source: &'static str,
    contract_ref: &'static str,
    locator_hint: Option<String>,
    response_shape: Option<crate::OutputResponseShape>,
) -> Value {
    json!({
        "source": source,
        "contract_ref": contract_ref,
        "locator_hint": locator_hint.unwrap_or_default(),
        "response_shape": response_shape.map(|shape| shape.as_str()).unwrap_or(""),
    })
}

fn command_observation_marker_contract_candidate(
    route_result: &crate::RouteResult,
) -> Option<Value> {
    if route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
    {
        return None;
    }
    let contract_ref = if super::route_reason_has_marker(
        route_result,
        "explicit_command_requires_command_output_summary_execution",
    ) || super::route_reason_has_marker(
        route_result,
        "command_payload_requires_command_output_summary_execution",
    ) {
        "contract:command_output_summary"
    } else if super::route_reason_has_marker(
        route_result,
        "explicit_command_requires_fresh_execution",
    ) || super::route_reason_has_marker(
        route_result,
        "command_payload_requires_raw_output_execution",
    ) {
        "contract:raw_command_output"
    } else {
        return None;
    };

    Some(contract_candidate_json(
        "command_observation_marker",
        contract_ref,
        None,
        None,
    ))
}

fn sqlite_path_excerpt_judgment_contract_candidate(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Option<Value> {
    if route_result.needs_clarify
        || !super::route_reason_has_marker(route_result, "excerpt_kind_judgment")
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
    {
        return None;
    }
    let Some(path) =
        sqlite_database_locator_from_route_or_text(state, prompt, resolved_prompt, route_result)
    else {
        return None;
    };
    Some(contract_candidate_json(
        "sqlite_path_excerpt_judgment",
        "contract:sqlite_database_kind_judgment",
        Some(path),
        None,
    ))
}

fn sqlite_structured_version_contract_candidate(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Option<Value> {
    if route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !sqlite_version_selector(route_result)
    {
        return None;
    }
    let Some(path) =
        sqlite_database_locator_from_route_or_text(state, prompt, resolved_prompt, route_result)
    else {
        return None;
    };
    Some(contract_candidate_json(
        "sqlite_structured_version",
        "contract:sqlite_schema_version",
        Some(path),
        Some(crate::OutputResponseShape::Scalar),
    ))
}

fn sqlite_structured_table_listing_contract_candidate(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Option<Value> {
    if route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !sqlite_table_listing_selector(route_result)
    {
        return None;
    }
    let Some(path) =
        sqlite_database_locator_from_route_or_text(state, prompt, resolved_prompt, route_result)
    else {
        return None;
    };
    Some(contract_candidate_json(
        "sqlite_structured_table_listing",
        "contract:sqlite_table_listing",
        Some(path),
        Some(crate::OutputResponseShape::Strict),
    ))
}

fn sqlite_route_reason_table_contract_candidate(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Option<Value> {
    if route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
    {
        return None;
    }
    let contract_ref = if super::route_reason_has_marker(route_result, "sqlite_table_names_only") {
        "contract:sqlite_table_names_only"
    } else if super::route_reason_has_marker(route_result, "sqlite_table_listing") {
        "contract:sqlite_table_listing"
    } else {
        return None;
    };
    let Some(path) =
        sqlite_database_locator_from_route_or_text(state, prompt, resolved_prompt, route_result)
    else {
        return None;
    };
    Some(contract_candidate_json(
        "sqlite_route_reason_table",
        contract_ref,
        Some(path),
        Some(crate::OutputResponseShape::Strict),
    ))
}

fn config_validation_findings_contract_candidate(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Option<Value> {
    if route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !config_validation_findings_selector(route_result)
    {
        return None;
    }
    let Some(path) =
        structured_config_locator_from_route_or_text(state, prompt, resolved_prompt, route_result)
    else {
        return None;
    };
    Some(contract_candidate_json(
        "config_validation_findings",
        "contract:config_risk_assessment",
        Some(path),
        Some(crate::OutputResponseShape::Free),
    ))
}

fn config_validation_findings_selector(route_result: &crate::RouteResult) -> bool {
    route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(normalize_machine_selector_tail)
        .is_some_and(|selector| selector == "config_validation_findings")
}

fn sqlite_version_selector(route_result: &crate::RouteResult) -> bool {
    route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(normalize_machine_selector_tail)
        .is_some_and(|selector| matches!(selector.as_str(), "schema_version" | "user_version"))
}

fn sqlite_table_listing_selector(route_result: &crate::RouteResult) -> bool {
    route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(normalize_machine_selector_tail)
        .is_some_and(|selector| {
            matches!(
                selector.as_str(),
                "tables" | "table_names" | "sqlite_tables" | "sqlite_table_names"
            )
        })
}

fn normalize_machine_selector_tail(selector: &str) -> String {
    selector
        .trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '.')
        .to_ascii_lowercase()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn structured_config_locator_from_route_or_text(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Option<String> {
    let mut candidates = Vec::new();
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if !locator_hint.is_empty() {
        candidates.push(locator_hint.to_string());
    }
    for text in [
        prompt,
        resolved_prompt,
        route_result.resolved_intent.as_str(),
    ] {
        candidates.extend(
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(
                text,
            )
            .into_iter()
            .filter(|locator| matches!(locator.locator_kind, crate::OutputLocatorKind::Path))
            .map(|locator| locator.locator_hint),
        );
    }
    candidates.into_iter().find_map(|candidate| {
        let path = super::resolve_existing_workspace_locator_hint(state, &candidate)?;
        structured_config_path(&path).then_some(path)
    })
}

fn structured_config_path(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("json" | "toml" | "yaml" | "yml")
    )
}

fn sqlite_database_locator_from_route_or_text(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &crate::RouteResult,
) -> Option<String> {
    let mut candidates = Vec::new();
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if !locator_hint.is_empty() {
        candidates.push(locator_hint.to_string());
    }
    for text in [
        prompt,
        resolved_prompt,
        route_result.resolved_intent.as_str(),
    ] {
        candidates.extend(
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(
                text,
            )
            .into_iter()
            .filter(|locator| matches!(locator.locator_kind, crate::OutputLocatorKind::Path))
            .map(|locator| locator.locator_hint),
        );
    }
    candidates
        .into_iter()
        .find_map(|candidate| {
            let path = super::resolve_existing_workspace_locator_hint(state, &candidate)?;
            sqlite_database_path(&path).then_some(path)
        })
        .or_else(|| {
            crate::worker::try_resolve_implicit_locator_path(
                state,
                prompt,
                &format!(
                    "{}\n{}",
                    resolved_prompt,
                    route_result.resolved_intent.as_str()
                ),
                crate::OutputLocatorKind::Path,
                None,
            )
            .and_then(|resolution| match resolution {
                crate::worker::LocatorAutoResolution::Direct(path) => {
                    sqlite_database_path(&path).then_some(path)
                }
                crate::worker::LocatorAutoResolution::Fuzzy(_) => None,
            })
        })
}

fn sqlite_database_path(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("sqlite" | "sqlite3" | "db")
    )
}

#[cfg(test)]
#[path = "ask_pipeline_contract_repair_tests.rs"]
mod tests;
