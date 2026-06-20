use serde_json::json;

use super::{
    ActFinalizeStyle, FirstLayerDecision, IntentOutputContract, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RouteDecision, ScheduleKind,
    TargetTaskPolicy, TurnAnalysis, TurnType,
};

pub(super) fn structural_alias_binding_fallback_decision(
    user_request: &str,
) -> Option<(RouteDecision, TurnAnalysis)> {
    let mut bindings =
        crate::conversation_state::structural_quoted_alias_bindings_from_prompt(user_request);
    if bindings.is_empty() {
        if let Some(binding) =
            crate::conversation_state::structural_quoted_alias_binding_from_single_locator_prompt(
                user_request,
            )
        {
            bindings.push(binding);
        }
    }
    if bindings.is_empty() {
        return None;
    }
    if !structural_alias_binding_prompt_is_binding_only(user_request) {
        return None;
    }
    let state_patch = json!({
        "alias_bindings": bindings
            .iter()
            .map(|binding| json!({
                "alias": binding.alias,
                "target": binding.target,
            }))
            .collect::<Vec<_>>()
    });
    let turn_analysis = TurnAnalysis {
        turn_type: Some(TurnType::PreferenceOrMemory),
        target_task_policy: Some(TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(state_patch),
        attachment_processing_required: false,
    };
    let decision = RouteDecision {
        resolved_user_intent: user_request.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "structured_alias_binding_fallback".to_string(),
        confidence: Some(0.95),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    };
    Some((decision, turn_analysis))
}

fn structural_alias_binding_prompt_is_binding_only(user_request: &str) -> bool {
    let mut locators =
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(
            user_request,
        );
    locators.dedup_by(|left, right| left.locator_hint == right.locator_hint);
    if locators.is_empty() {
        return false;
    }

    let mut segment_start = 0usize;
    let mut last_locator_end = None;
    for locator in locators {
        let Some((_, locator_end)) = find_locator_span_after_for_alias_fast_path(
            user_request,
            &locator.locator_hint,
            segment_start,
        ) else {
            continue;
        };
        segment_start = locator_end;
        last_locator_end = Some(locator_end);
    }
    let Some(last_locator_end) = last_locator_end else {
        return false;
    };
    user_request
        .get(last_locator_end..)
        .unwrap_or_default()
        .chars()
        .all(alias_binding_trailing_char_is_structural)
}

fn find_locator_span_after_for_alias_fast_path(
    user_request: &str,
    locator: &str,
    start: usize,
) -> Option<(usize, usize)> {
    if locator.trim().is_empty() || start >= user_request.len() {
        return None;
    }
    user_request
        .get(start..)
        .and_then(|tail| tail.find(locator).map(|offset| start + offset))
        .or_else(|| user_request.find(locator))
        .map(|idx| (idx, idx + locator.len()))
}

fn alias_binding_trailing_char_is_structural(ch: char) -> bool {
    ch.is_whitespace()
        || ch.is_ascii_punctuation()
        || matches!(
            ch,
            '，' | '。' | '、' | '；' | '：' | '！' | '？' | '）' | '】' | '》' | '”' | '’'
        )
}

pub(super) fn normalize_schedule_intent_from_normalizer(
    schedule_kind: ScheduleKind,
    schedule_intent: Option<crate::ScheduleIntentOutput>,
    resolved_user_intent: &str,
    reason: &str,
    needs_clarify: bool,
    clarify_question: &str,
    confidence: f64,
) -> Option<crate::ScheduleIntentOutput> {
    if matches!(schedule_kind, ScheduleKind::None) {
        return None;
    }
    let mut intent = schedule_intent?;
    let cleaned_kind = crate::schedule_service::clean_schedule_kind(&intent.kind);
    if !cleaned_kind.is_empty() && cleaned_kind != schedule_kind.as_str() {
        return None;
    }
    if cleaned_kind.is_empty() {
        intent.kind = schedule_kind.as_str().to_string();
    }
    if intent.raw.trim().is_empty() {
        intent.raw = resolved_user_intent.trim().to_string();
    }
    if intent.reason.trim().is_empty() {
        intent.reason = reason.trim().to_string();
    }
    if needs_clarify {
        intent.needs_clarify = true;
        if intent.clarify_question.trim().is_empty() && !clarify_question.trim().is_empty() {
            intent.clarify_question = clarify_question.trim().to_string();
        }
    }
    if intent.confidence <= 0.0 {
        intent.confidence = confidence;
    }
    if !schedule_intent_is_complete_enough_for_direct_use(schedule_kind, &intent) {
        return None;
    }
    Some(intent)
}

fn schedule_intent_is_complete_enough_for_direct_use(
    schedule_kind: ScheduleKind,
    intent: &crate::ScheduleIntentOutput,
) -> bool {
    if intent.needs_clarify {
        return !intent.clarify_question.trim().is_empty() || !intent.reason.trim().is_empty();
    }
    match schedule_kind {
        ScheduleKind::Create => {
            let schedule_type =
                crate::schedule_service::clean_schedule_kind(&intent.schedule.r#type);
            let task_kind = crate::schedule_service::clean_schedule_kind(&intent.task.kind);
            matches!(
                schedule_type.as_str(),
                "once" | "daily" | "weekly" | "interval" | "cron"
            ) && matches!(task_kind.as_str(), "ask" | "run_skill")
        }
        ScheduleKind::Update | ScheduleKind::Delete | ScheduleKind::Query => true,
        ScheduleKind::None => false,
    }
}

pub(super) fn apply_schedule_route_contract_repair(
    schedule_kind: ScheduleKind,
    needs_clarify: bool,
    output_contract: &mut IntentOutputContract,
    wants_file_delivery: &mut bool,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if matches!(schedule_kind, ScheduleKind::None) {
        return None;
    }
    let mut changed = false;
    if *wants_file_delivery {
        *wants_file_delivery = false;
        changed = true;
    }
    if output_contract.requires_content_evidence {
        output_contract.requires_content_evidence = false;
        changed = true;
    }
    if output_contract.delivery_required {
        output_contract.delivery_required = false;
        changed = true;
    }
    if !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None) {
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        changed = true;
    }
    if !matches!(output_contract.locator_kind, OutputLocatorKind::None) {
        output_contract.locator_kind = OutputLocatorKind::None;
        changed = true;
    }
    if !output_contract.locator_hint.trim().is_empty() {
        output_contract.locator_hint.clear();
        changed = true;
    }
    if !matches!(output_contract.semantic_kind, OutputSemanticKind::None) {
        output_contract.semantic_kind = OutputSemanticKind::None;
        changed = true;
    }
    if matches!(output_contract.response_shape, OutputResponseShape::Free) {
        output_contract.response_shape = OutputResponseShape::OneSentence;
        changed = true;
    }
    if !needs_clarify
        && !matches!(
            *legacy_normalizer_decision,
            FirstLayerDecision::PlannerExecute
        )
    {
        *legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
        changed = true;
    }
    if !matches!(*execution_finalize_style, ActFinalizeStyle::Plain) {
        *execution_finalize_style = ActFinalizeStyle::Plain;
        changed = true;
    }
    changed.then_some("schedule_route_contract_repair")
}
