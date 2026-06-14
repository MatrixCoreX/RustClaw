use std::path::Path;

use super::{
    route_has_structured_execution_signal, FirstLayerDecision, IntentOutputContract,
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind,
    RouteDecision, ScheduleKind, SelfExtensionMode, SelfExtensionTrigger,
};

pub(super) fn explicit_surface_path_facts_fallback_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) -> Option<RouteDecision> {
    let targets = explicit_surface_path_fact_targets(req_surface);
    if req_surface.inline_json_shape.is_some()
        || req_surface.has_delivery_token_reference()
        || req_surface.has_deictic_reference()
        || structured_target_refinement_blocks_explicit_path_facts(req_surface, &targets)
    {
        return None;
    }
    if targets.len() < 2 {
        return None;
    }
    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_unavailable_explicit_multi_path_facts".to_string(),
        confidence: Some(0.50),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: workspace_root.display().to_string(),
            ..Default::default()
        },
    })
}

pub(super) fn explicit_surface_path_metadata_clarify_repair_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    needs_clarify: bool,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> Option<RouteDecision> {
    if !(needs_clarify || matches!(first_layer_decision, FirstLayerDecision::Clarify)) {
        return None;
    }
    let targets = explicit_surface_path_fact_targets(req_surface);
    if req_surface.inline_json_shape.is_some()
        || req_surface.has_delivery_token_reference()
        || req_surface.has_deictic_reference()
        || structured_target_refinement_blocks_explicit_path_facts(req_surface, &targets)
        || output_contract.semantic_kind != OutputSemanticKind::QuantityComparison
        || wants_file_delivery
        || !matches!(schedule_kind, ScheduleKind::None)
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || !matches!(output_contract.self_extension.mode, SelfExtensionMode::None)
        || !matches!(
            output_contract.self_extension.trigger,
            SelfExtensionTrigger::None
        )
        || output_contract.self_extension.execute_now
        || execution_recipe_hint.is_some_and(|spec| {
            !matches!(
                spec.kind,
                crate::execution_recipe::ExecutionRecipeKind::None
            )
        })
    {
        return None;
    }
    if targets.len() < 2 {
        return None;
    }
    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_clarify_explicit_multi_path_metadata".to_string(),
        confidence: Some(0.55),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::QuantityComparison,
            locator_hint: workspace_root.display().to_string(),
            ..Default::default()
        },
    })
}

pub(super) fn explicit_surface_path_facts_clarify_repair_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    needs_clarify: bool,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    _execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> Option<RouteDecision> {
    if !(needs_clarify || matches!(first_layer_decision, FirstLayerDecision::Clarify)) {
        return None;
    }
    if route_has_structured_execution_signal(
        output_contract,
        wants_file_delivery,
        schedule_kind,
        None,
    ) {
        return None;
    }
    let mut decision =
        explicit_surface_path_facts_fallback_decision(req, req_surface, workspace_root)?;
    decision.reason = "normalizer_clarify_explicit_multi_path_facts".to_string();
    decision.confidence = Some(0.55);
    Some(decision)
}

pub(super) fn explicit_surface_path_fact_targets(
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Vec<String> {
    let pair_targets = req_surface
        .locator_target_pair
        .as_ref()
        .map(|(left, right)| vec![left.clone(), right.clone()])
        .unwrap_or_default();
    let candidates = if pair_targets.len() >= 2 {
        pair_targets
    } else {
        req_surface.filename_candidates.clone()
    };
    let mut out = Vec::new();
    for candidate in candidates {
        let candidate = trim_structural_path_fact_candidate(&candidate);
        if !candidate.is_empty()
            && token_looks_like_supported_path_fact_target(&candidate)
            && !out
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&candidate))
        {
            out.push(candidate);
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn structured_target_refinement_blocks_explicit_path_facts(
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    targets: &[String],
) -> bool {
    let mentions_block = !req_surface.field_selector_mentions.is_empty()
        && !req_surface
            .field_selector_mentions
            .iter()
            .all(|selector| selector_matches_explicit_path_target(selector, targets));
    mentions_block
        || req_surface
            .dotted_field_selector
            .as_deref()
            .is_some_and(|selector| !selector_matches_explicit_path_target(selector, targets))
}

fn selector_matches_explicit_path_target(selector: &str, targets: &[String]) -> bool {
    let selector = selector.trim();
    !selector.is_empty()
        && targets.iter().any(|target| {
            Path::new(target)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .is_some_and(|name| name.eq_ignore_ascii_case(selector))
        })
}

fn trim_structural_path_fact_candidate(candidate: &str) -> String {
    let trimmed = candidate
        .trim()
        .trim_matches(|ch: char| {
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
        .to_string();
    if let Some(stripped) = trimmed.strip_suffix('.') {
        if token_looks_like_supported_path_fact_target(stripped) {
            return stripped.to_string();
        }
    }
    trimmed
}

fn token_looks_like_supported_path_fact_target(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty()
        || token.starts_with("http://")
        || token.starts_with("https://")
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
    {
        return false;
    }
    let Some(name) = Path::new(token)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return false;
    };
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && matches!(
            extension.to_ascii_lowercase().as_str(),
            "md" | "txt"
                | "json"
                | "toml"
                | "yaml"
                | "yml"
                | "rs"
                | "log"
                | "sqlite"
                | "db"
                | "csv"
        )
}

pub(super) fn ascii_token_present(text: &str, token: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .any(|candidate| candidate.eq_ignore_ascii_case(token))
}

pub(super) fn is_bare_path_only_input_for_clarify(
    text: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 60 {
        return false;
    }
    if trimmed.contains(['?', '？', '!', '！']) {
        return false;
    }
    if surface.inline_json_shape.is_some() || surface.has_structured_target_refinement() {
        return false;
    }
    if trimmed.split_whitespace().count() != 1 {
        return false;
    }
    surface.has_explicit_path_or_url()
        || surface.has_single_filename_candidate()
        || token_looks_like_pathish_filename(trimmed)
}

fn token_looks_like_pathish_filename(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() || token.starts_with('.') || token.contains('/') || token.contains('\\') {
        return false;
    }
    let Some((stem, extension)) = token.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !extension.is_empty()
        && extension.len() <= 8
        && extension
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}
