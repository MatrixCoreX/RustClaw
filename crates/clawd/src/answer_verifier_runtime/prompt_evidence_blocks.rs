use serde_json::json;

use super::*;

pub(in crate::answer_verifier) fn evidence_policy_context_prompt_block(
    route_result: &RouteResult,
) -> String {
    crate::evidence_policy::evidence_policy_context_prompt_line_for_route(route_result)
}

pub(in crate::answer_verifier) fn output_contract_prompt_block(
    route_result: &RouteResult,
) -> String {
    let evidence_policy_trace = verifier_evidence_policy_prompt_trace(route_result);
    let final_answer_shape = crate::evidence_policy::final_answer_shape_for_route(route_result);
    serde_json::to_string_pretty(&json!({
        "response_shape": route_result.output_contract.response_shape.as_str(),
        "final_answer_shape": final_answer_shape.map(crate::evidence_policy::FinalAnswerShape::as_str),
        "final_answer_shape_class": final_answer_shape.map(|shape| shape.class().as_str()),
        "requires_content_evidence": route_result.output_contract.requires_content_evidence,
        "delivery_required": route_result.output_contract.delivery_required,
        "locator_kind": route_result.output_contract.locator_kind.as_str(),
        "delivery_intent": route_result.output_contract.delivery_intent.as_str(),
        "locator_hint": route_result.output_contract.locator_hint,
        "evidence_policy": evidence_policy_trace,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn verifier_evidence_policy_prompt_trace(route_result: &RouteResult) -> Option<serde_json::Value> {
    let mut trace = crate::evidence_policy::trace_snapshot_for_route(route_result)?;
    if let Some(obj) = trace.as_object_mut() {
        obj.remove("contract_marker");
        obj.remove("semantic_kind");
        obj.remove("trace_policy");
        obj.remove("observation_extractors");
        obj.remove("observation_sources");
        obj.remove("artifact_kind");
        obj.remove("channel_visibility");
        obj.insert(
            "compact_line".to_string(),
            serde_json::Value::String(
                crate::evidence_policy::compact_prompt_line_for_route(route_result)
                    .unwrap_or_default(),
            ),
        );
    }
    Some(trace)
}

pub(in crate::answer_verifier) fn provider_safe_excerpt_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64:{hash:016x}")
}

pub(in crate::answer_verifier) fn provider_safe_numeric_evidence(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> Vec<serde_json::Value> {
    let Some(output) = step.output_excerpt.as_deref() else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    collect_provider_safe_numeric_evidence("", &value, &mut items);
    items.truncate(32);
    items
}

pub(in crate::answer_verifier) fn collect_provider_safe_numeric_evidence(
    prefix: &str,
    value: &serde_json::Value,
    out: &mut Vec<serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let field = if prefix.is_empty() {
                    key.to_string()
                } else {
                    format!("{prefix}.{key}")
                };
                if provider_safe_numeric_evidence_leaf(key)
                    && matches!(
                        child,
                        serde_json::Value::Number(_) | serde_json::Value::Bool(_)
                    )
                {
                    out.push(json!({
                        "field": field,
                        "value": child,
                    }));
                }
                collect_provider_safe_numeric_evidence(&field, child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let field = format!("{prefix}[{index}]");
                collect_provider_safe_numeric_evidence(&field, child, out);
            }
        }
        _ => {}
    }
}

pub(in crate::answer_verifier) fn provider_safe_numeric_evidence_leaf(key: &str) -> bool {
    matches!(
        key,
        "count"
            | "dirs"
            | "exists"
            | "files"
            | "hidden"
            | "size_bytes"
            | "total"
            | "total_size_bytes"
    )
}

pub(in crate::answer_verifier) fn provider_safe_step_evidence(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> serde_json::Value {
    json!({
        "step_id": step.step_id,
        "skill": step.skill,
        "status": step.status.as_str(),
        "observed_evidence": crate::task_journal::observed_evidence_for_step_trace(step),
        "key_numeric_evidence": provider_safe_numeric_evidence(step),
        "output_excerpt_present": step.output_excerpt.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "output_excerpt_hash": step.output_excerpt.as_deref().map(provider_safe_excerpt_hash),
        "error_excerpt_present": step.error_excerpt.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "error_excerpt_hash": step.error_excerpt.as_deref().map(provider_safe_excerpt_hash),
    })
}

pub(in crate::answer_verifier) fn execution_evidence_prompt_block(
    journal: &crate::task_journal::TaskJournal,
) -> String {
    let mut steps = journal
        .step_results
        .iter()
        .filter(|step| step_can_supply_verifier_prompt_observation(step))
        .rev()
        .take(MAX_VERIFIER_STEPS)
        .map(provider_safe_step_evidence)
        .collect::<Vec<_>>();
    steps.reverse();
    serde_json::to_string_pretty(&steps).unwrap_or_else(|_| "[]".to_string())
}

pub(in crate::answer_verifier) fn current_context_prompt_block(
    journal: &crate::task_journal::TaskJournal,
) -> String {
    const MAX_CHARS: usize = 12_000;
    let Some(summary) = journal.context_bundle_summary.as_deref() else {
        return "<none>".to_string();
    };
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return "<none>".to_string();
    }
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_CHARS).collect()
}
