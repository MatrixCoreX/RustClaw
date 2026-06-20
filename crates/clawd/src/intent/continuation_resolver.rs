use crate::clarify_followup::ClarifyLocatorReplyRewrite;

#[derive(Debug, Clone)]
pub(crate) enum ClarifyFollowupResolution {
    None,
    NormalizerRewrite { rewritten_prompt: String },
    LocatorReplyRewrite(ClarifyLocatorReplyRewrite),
}

pub(crate) fn immediate_prior_turn_was_clarify(last_turn_full: &str) -> bool {
    crate::clarify_followup::last_turn_was_clarify(last_turn_full)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_clarify_followup(
    prompt: &str,
    last_turn_full: Option<&str>,
    active_followup_frame: Option<&crate::followup_frame::FollowupFrame>,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
    active_observed_facts: Option<&crate::observed_facts::ObservedFacts>,
) -> ClarifyFollowupResolution {
    let surface = super::surface_signals::analyze_prompt_surface(prompt.trim());
    resolve_clarify_followup_with_surface(
        prompt,
        last_turn_full,
        active_followup_frame,
        active_clarify_state,
        active_observed_facts,
        &surface,
    )
}

pub(crate) fn resolve_clarify_followup_with_surface(
    prompt: &str,
    last_turn_full: Option<&str>,
    active_followup_frame: Option<&crate::followup_frame::FollowupFrame>,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
    _active_observed_facts: Option<&crate::observed_facts::ObservedFacts>,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> ClarifyFollowupResolution {
    if let Some(state_hit) = active_clarify_state.and_then(|state| {
        synthesize_clarify_state_reply_resolution_with_surface(state, prompt, &surface)
    }) {
        return state_hit;
    }
    if let Some(frame_hit) = active_followup_frame.and_then(|frame| {
        crate::followup_frame::synthesize_locator_reply_resolved_intent(frame, prompt).map(
            |(resolved_intent, reason)| crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent,
                prior_user_text: frame.source_request.clone(),
                current_user_text: prompt.trim().to_string(),
                reason,
            },
        )
    }) {
        return ClarifyFollowupResolution::LocatorReplyRewrite(frame_hit);
    }
    let Some(last_turn_full) = last_turn_full else {
        return ClarifyFollowupResolution::None;
    };
    if !immediate_prior_turn_was_clarify(last_turn_full) {
        return ClarifyFollowupResolution::None;
    }
    if let Some(hit) = crate::clarify_followup::try_clarify_reply_rewrite(prompt, last_turn_full) {
        return ClarifyFollowupResolution::LocatorReplyRewrite(hit);
    }
    if !surface_has_structural_clarify_target_fill(&surface) {
        return ClarifyFollowupResolution::None;
    }
    let Some(prior_user_text) = crate::clarify_followup::extract_prior_user_text(last_turn_full)
    else {
        return ClarifyFollowupResolution::None;
    };
    let rewritten_prompt = format!(
        "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target or content: {}",
        prior_user_text.trim(),
        prompt.trim()
    );
    ClarifyFollowupResolution::NormalizerRewrite { rewritten_prompt }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_clarify_followup_from_session(
    prompt: &str,
    last_turn_full: Option<&str>,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> ClarifyFollowupResolution {
    let surface = super::surface_signals::analyze_prompt_surface(prompt.trim());
    resolve_clarify_followup_from_session_with_surface(
        prompt,
        last_turn_full,
        session_snapshot,
        &surface,
    )
}

pub(crate) fn resolve_clarify_followup_from_session_with_surface(
    prompt: &str,
    last_turn_full: Option<&str>,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> ClarifyFollowupResolution {
    let active_followup_frame =
        session_snapshot.and_then(|snapshot| snapshot.active_followup_frame.as_ref());
    let active_clarify_state =
        session_snapshot.and_then(|snapshot| snapshot.active_clarify_state.as_ref());
    let active_observed_facts =
        session_snapshot.and_then(|snapshot| snapshot.active_observed_facts.as_ref());
    resolve_clarify_followup_with_surface(
        prompt,
        last_turn_full,
        active_followup_frame,
        active_clarify_state,
        active_observed_facts,
        surface,
    )
}

#[cfg(test)]
pub(crate) fn prompt_can_fill_active_clarify_target(
    prompt: &str,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
) -> bool {
    let surface = super::surface_signals::analyze_prompt_surface(prompt.trim());
    surface_can_fill_active_clarify_target(prompt, active_clarify_state, &surface)
}

pub(crate) fn surface_can_fill_active_clarify_target(
    prompt: &str,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    let Some(clarify_state) = active_clarify_state else {
        return false;
    };
    let _ = prompt;
    if !clarify_state_has_structural_binding_contract(clarify_state) {
        return false;
    }
    surface_has_structural_clarify_target_fill(surface)
}

fn clarify_state_has_structural_binding_contract(
    clarify_state: &crate::clarify_state::ClarifyState,
) -> bool {
    clarify_state.delivery_required
        || clarify_state.output_shape.is_some()
        || clarify_state.semantic_kind.is_some()
        || !clarify_state.candidate_targets.is_empty()
}

fn synthesize_clarify_state_reply_resolution_with_surface(
    clarify_state: &crate::clarify_state::ClarifyState,
    prompt: &str,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> Option<ClarifyFollowupResolution> {
    match clarify_state.missing_slot {
        crate::clarify_state::ClarifyMissingSlot::Locator => {
            if !clarify_state_has_structural_binding_contract(clarify_state) {
                return None;
            }
            if surface.is_structural_locator_only_reply() {
                return Some(ClarifyFollowupResolution::LocatorReplyRewrite(
                    crate::clarify_followup::ClarifyLocatorReplyRewrite {
                        resolved_intent: format!(
                            "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target or content: {}",
                            clarify_state.source_request.trim(),
                            prompt.trim()
                        ),
                        prior_user_text: clarify_state.source_request.trim().to_string(),
                        current_user_text: prompt.trim().to_string(),
                        reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
                    },
                ));
            }
            if surface_has_structural_clarify_target_fill(surface) {
                return Some(ClarifyFollowupResolution::NormalizerRewrite {
                    rewritten_prompt: format!(
                        "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target or content: {}",
                        clarify_state.source_request.trim(),
                        prompt.trim()
                    ),
                });
            }
            None
        }
    }
}

pub(crate) fn surface_has_structural_clarify_target_fill(
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    if surface.token_count == 0 {
        return false;
    }
    if surface.has_deictic_reference() && !surface.has_explicit_path_or_url() {
        return false;
    }
    matches!(
        surface.inline_json_shape,
        Some(crate::intent::surface_signals::InlineJsonShape::WholeValue)
    ) || surface.has_any_locator_reference()
}

#[cfg(test)]
#[path = "continuation_resolver_tests.rs"]
mod tests;
