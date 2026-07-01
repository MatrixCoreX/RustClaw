use serde_json::Value;

use super::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, RouteDecision, ScheduleKind,
};

pub(super) fn inline_json_transform_fallback_decision(req: &str) -> Option<RouteDecision> {
    if !inline_structural_transform_candidate(req) {
        return None;
    }

    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_unavailable_inline_json_transform".to_string(),
        confidence: Some(0.82),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            ..Default::default()
        },
    })
}

fn inline_structural_transform_candidate(req: &str) -> bool {
    crate::intent::surface_signals::inline_json_transform_request(req)
        || inline_object_rename_transform_candidate(req)
}

fn inline_object_rename_transform_candidate(req: &str) -> bool {
    let Some(raw) = crate::extract_first_json_value_any(req) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };
    if obj.is_empty()
        || obj.contains_key("action")
        || obj.contains_key("skill")
        || obj.contains_key("operation")
    {
        return false;
    }
    let input_keys = obj.keys().map(String::as_str).collect::<Vec<_>>();
    let instruction = req
        .rfind(&raw)
        .map(|start| {
            let end = start.saturating_add(raw.len());
            format!("{} {}", &req[..start], &req[end..])
        })
        .unwrap_or_else(|| req.to_string());
    let tokens = inline_transform_schema_tokens(&instruction);
    let mut source_positions = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| input_keys.iter().any(|key| key == &token.as_str()))
        .collect::<Vec<_>>();
    source_positions.dedup_by(|(_, left), (_, right)| left == right);
    if source_positions.len() != 1 {
        return false;
    }
    let (source_index, source_token) = source_positions[0];
    let target_candidates = tokens
        .iter()
        .skip(source_index + 1)
        .filter(|token| !input_keys.iter().any(|key| key == &token.as_str()))
        .filter(|token| inline_transform_schema_shaped_target_token(token, source_token))
        .fold(Vec::<&String>::new(), |mut acc, token| {
            if !acc
                .iter()
                .any(|existing| existing.as_str() == token.as_str())
            {
                acc.push(token);
            }
            acc
        });
    target_candidates.len() == 1
}

fn inline_transform_schema_field_token(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '-' || ch.is_ascii_alphanumeric())
}

fn inline_transform_schema_shaped_target_token(candidate: &str, source: &str) -> bool {
    inline_transform_schema_field_token(candidate)
        && candidate != source
        && !candidate.chars().all(|ch| ch.is_ascii_uppercase())
        && (candidate.contains('_')
            || candidate.contains('-')
            || candidate.chars().any(|ch| ch.is_ascii_digit())
            || source.contains('_')
            || source.contains('-')
            || source.chars().any(|ch| ch.is_ascii_digit()))
}

fn inline_transform_schema_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch == '_' || ch == '-' || ch.is_ascii_alphanumeric() {
            current.push(ch);
            continue;
        }
        if inline_transform_schema_field_token(&current) {
            tokens.push(std::mem::take(&mut current));
        } else {
            current.clear();
        }
    }
    if inline_transform_schema_field_token(&current) {
        tokens.push(current);
    }
    tokens
}

pub(super) fn parsed_inline_json_transform_repair_decision(
    req: &str,
    needs_clarify: bool,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> Option<RouteDecision> {
    if !needs_clarify {
        return None;
    }
    if wants_file_delivery || !matches!(schedule_kind, ScheduleKind::None) {
        return None;
    }
    if execution_recipe_hint.is_some_and(|spec| {
        !matches!(
            spec.kind,
            crate::execution_recipe::ExecutionRecipeKind::None
        )
    }) {
        return None;
    }

    let mut decision = inline_json_transform_fallback_decision(req)?;
    decision.reason = "parsed_inline_json_transform_contract_repair".to_string();
    Some(decision)
}
