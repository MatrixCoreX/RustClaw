use std::path::Path;

use super::{
    archive_pair_contract_from_surface, ascii_token_present, IntentOutputContract,
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind,
    RouteDecision, ScheduleKind,
};

pub(super) fn parse_failed_explicit_capability_fallback_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) -> Option<RouteDecision> {
    if !git_repository_state_surface_token_present(req)
        || req_surface.has_explicit_path_or_url()
        || req_surface.has_single_filename_candidate()
        || req_surface.has_filename_candidates()
        || req_surface.has_structured_target_refinement()
        || req_surface.has_delivery_token_reference()
    {
        return None;
    }

    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_parse_failed_explicit_git_repository_state".to_string(),
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
            semantic_kind: OutputSemanticKind::GitRepositoryState,
            locator_hint: workspace_root.display().to_string(),
            ..Default::default()
        },
    })
}

fn git_repository_state_surface_token_present(req: &str) -> bool {
    ascii_token_present(req, "git")
        || ascii_token_present(req, "remote")
        || ascii_token_present(req, "HEAD")
        || ascii_token_present(req, "branch")
}

pub(super) fn parse_failed_explicit_existing_path_observation_fallback_decision(
    req: &str,
    workspace_root: &Path,
) -> Option<RouteDecision> {
    let req_surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let archive_pair_seed = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        ..Default::default()
    };
    if let Some((semantic_kind, locator_hint)) =
        archive_pair_contract_from_surface(&archive_pair_seed, &req_surface)
    {
        let response_shape = match semantic_kind {
            OutputSemanticKind::ArchivePack => OutputResponseShape::Scalar,
            OutputSemanticKind::ArchiveUnpack => OutputResponseShape::OneSentence,
            _ => OutputResponseShape::Free,
        };
        return Some(RouteDecision {
            resolved_user_intent: req.trim().to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: "normalizer_parse_failed_archive_pair_contract_fallback".to_string(),
            confidence: Some(0.55),
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind,
                locator_hint,
                ..Default::default()
            },
        });
    }

    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(req)
    {
        if locator.locator_kind != OutputLocatorKind::Path {
            continue;
        }
        let raw_path = Path::new(locator.locator_hint.trim());
        if raw_path.as_os_str().is_empty() {
            continue;
        }
        let candidate = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else {
            workspace_root.join(raw_path)
        };
        let semantic_kind = if candidate.is_dir() {
            OutputSemanticKind::DirectoryPurposeSummary
        } else if candidate.is_file() {
            OutputSemanticKind::ContentExcerptSummary
        } else {
            continue;
        };
        let locator_hint = candidate
            .canonicalize()
            .unwrap_or(candidate)
            .display()
            .to_string();
        return Some(RouteDecision {
            resolved_user_intent: req.trim().to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: "normalizer_parse_failed_explicit_existing_path_observation".to_string(),
            confidence: Some(0.50),
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
                semantic_kind,
                locator_hint,
                ..Default::default()
            },
        });
    }
    None
}

/// Fallback `RouteDecision` used when normalizer LLM fails or its output cannot be parsed.
/// It intentionally stays on AskClarify instead of using local semantic heuristics as
/// a substitute planner.
pub(super) fn empty_clarify_decision(user_request: &str, reason: &str) -> RouteDecision {
    RouteDecision {
        resolved_user_intent: user_request.trim().to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        reason: reason.to_string(),
        confidence: None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    }
}
