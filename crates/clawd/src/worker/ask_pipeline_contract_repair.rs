pub(super) fn repair_compound_file_names_plus_content_summary_contract(
    route_result: &mut crate::RouteResult,
) {
    let repairs_file_names_contract = super::route_reason_has_marker(
        route_result,
        "llm_semantic_contract_repair:compound_request_requires_repair_to_file_names_plus_content_summary",
    ) || super::route_reason_has_marker_prefix(
        route_result,
        "llm_semantic_contract_repair:malformed_contract_listing_vs_content_synthesis_conflict",
    );
    let repairs_file_paths_contract =
        super::route_reason_has_marker_prefix(route_result, "llm_semantic_contract_repair")
            && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths;
    let repairs_file_paths_contract = repairs_file_paths_contract
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        );
    if !(repairs_file_names_contract || repairs_file_paths_contract)
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames | crate::OutputSemanticKind::FilePaths
        )
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
    {
        return;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route_result.output_contract.response_shape = crate::OutputResponseShape::Free;
    let repair_marker = if repairs_file_paths_contract {
        if route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
            && route_result.output_contract.locator_hint.trim().is_empty()
        {
            route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
            super::append_route_reason(
                route_result,
                "compound_file_paths_summary_bound_to_current_workspace",
            );
        }
        "compound_file_paths_plus_content_summary_contract_repaired"
    } else {
        "compound_file_names_plus_content_summary_contract_repaired"
    };
    super::append_route_reason(route_result, repair_marker);
}

pub(super) fn repair_session_alias_listing_plus_content_summary_contract(
    state: &crate::AppState,
    prompt: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    route_result: &mut crate::RouteResult,
) {
    if !matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryEntryGroups
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::FilePaths
    ) || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
    {
        return;
    }
    let Some(conversation_state) = session_snapshot.conversation_state.as_ref() else {
        return;
    };
    let mut distinct_targets = Vec::new();
    let mut has_directory_target = false;
    let mut has_file_target = false;
    for binding in crate::conversation_state::alias_bindings_mentioned_in_prompt(
        &conversation_state.alias_bindings,
        prompt,
    ) {
        let target = binding.target.trim();
        if target.is_empty() || distinct_targets.iter().any(|seen| seen == target) {
            continue;
        }
        distinct_targets.push(target.to_string());
        let raw_path = std::path::Path::new(target);
        let target_path = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else {
            state.skill_rt.workspace_root.join(raw_path)
        };
        has_directory_target |= target_path.is_dir();
        has_file_target |= target_path.is_file();
    }
    if distinct_targets.len() < 2 || !has_directory_target || !has_file_target {
        return;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route_result.output_contract.response_shape = crate::OutputResponseShape::Free;
    super::append_route_reason(
        route_result,
        "session_alias_listing_plus_content_summary_contract_repaired",
    );
}

pub(super) fn repair_summary_only_content_excerpt_with_summary_contract(
    route_result: &mut crate::RouteResult,
) {
    if route_result.output_contract.semantic_kind
        != crate::OutputSemanticKind::ContentExcerptWithSummary
        || route_result.output_contract.response_shape != crate::OutputResponseShape::OneSentence
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
    {
        return;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    super::append_route_reason(
        route_result,
        "summary_only_content_excerpt_with_summary_contract_repaired",
    );
}

pub(super) fn repair_generic_path_content_grounded_summary_contract(
    route_result: &mut crate::RouteResult,
) -> bool {
    if repair_command_observation_marker_contract(route_result) {
        return true;
    }
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::FileToken
        )
        || !matches!(
            super::effective_auto_locator_kind(route_result),
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }

    let Some(shape) = crate::contract_matrix::final_answer_shape_for_output_contract(
        &route_result.output_contract,
    ) else {
        return false;
    };
    if shape.class() != crate::contract_matrix::FinalAnswerShapeClass::GroundedSummary {
        return false;
    }

    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    super::append_route_reason(
        route_result,
        "generic_path_content_grounded_summary_contract_repaired",
    );
    true
}

fn repair_command_observation_marker_contract(route_result: &mut crate::RouteResult) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
    {
        return false;
    }
    let semantic_kind = if super::route_reason_has_marker(
        route_result,
        "explicit_command_requires_command_output_summary_execution",
    ) || super::route_reason_has_marker(
        route_result,
        "command_payload_requires_command_output_summary_execution",
    ) {
        crate::OutputSemanticKind::CommandOutputSummary
    } else if super::route_reason_has_marker(
        route_result,
        "explicit_command_requires_fresh_execution",
    ) || super::route_reason_has_marker(
        route_result,
        "command_payload_requires_raw_output_execution",
    ) {
        crate::OutputSemanticKind::RawCommandOutput
    } else {
        return false;
    };

    route_result.output_contract.semantic_kind = semantic_kind;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    super::append_route_reason(route_result, "command_observation_marker_contract_repaired");
    true
}

pub(super) fn repair_sqlite_path_excerpt_judgment_contract(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || route_result.output_contract.semantic_kind
            != crate::OutputSemanticKind::ExcerptKindJudgment
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
        return false;
    }
    let Some(path) =
        sqlite_database_locator_from_route_or_text(state, prompt, resolved_prompt, route_result)
    else {
        return false;
    };
    route_result.output_contract.semantic_kind =
        crate::OutputSemanticKind::SqliteDatabaseKindJudgment;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    super::append_route_reason(
        route_result,
        "sqlite_path_excerpt_judgment_contract_repaired",
    );
    true
}

pub(super) fn repair_sqlite_structured_version_contract(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
        )
        || !sqlite_version_selector(route_result)
    {
        return false;
    }
    let Some(path) =
        sqlite_database_locator_from_route_or_text(state, prompt, resolved_prompt, route_result)
    else {
        return false;
    };
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteSchemaVersion;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    super::append_route_reason(route_result, "sqlite_structured_version_contract_repaired");
    true
}

pub(super) fn repair_config_validation_findings_contract(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ConfigValidation
        )
        || !config_validation_findings_selector(route_result)
    {
        return false;
    }
    let Some(path) =
        structured_config_locator_from_route_or_text(state, prompt, resolved_prompt, route_result)
    else {
        return false;
    };
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    route_result.output_contract.response_shape = crate::OutputResponseShape::Free;
    route_result
        .output_contract
        .self_extension
        .structured_field_selector = None;
    super::append_route_reason(route_result, "config_validation_findings_contract_repaired");
    true
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
    candidates.into_iter().find_map(|candidate| {
        let path = super::resolve_existing_workspace_locator_hint(state, &candidate)?;
        sqlite_database_path(&path).then_some(path)
    })
}

fn sqlite_database_path(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("sqlite" | "db")
    )
}

#[cfg(test)]
#[path = "ask_pipeline_contract_repair_tests.rs"]
mod tests;
