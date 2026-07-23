use serde_json::json;

use super::*;

pub(in crate::answer_verifier) fn evidence_policy_context_prompt_block(
    route_result: &AnswerContract,
) -> String {
    crate::evidence_policy::compact_prompt_line_for_output_contract(&route_result.output_contract)
        .unwrap_or_default()
}

pub(in crate::answer_verifier) fn output_contract_prompt_block(
    route_result: &AnswerContract,
) -> String {
    let evidence_policy_trace = verifier_evidence_policy_prompt_trace(route_result);
    let final_answer_shape = crate::evidence_policy::final_answer_shape_for_output_contract(
        &route_result.output_contract,
    );
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

fn verifier_evidence_policy_prompt_trace(
    route_result: &AnswerContract,
) -> Option<serde_json::Value> {
    let mut trace =
        crate::evidence_policy::trace_snapshot_for_output_contract(&route_result.output_contract)?;
    if let Some(obj) = trace.as_object_mut() {
        obj.remove("contract_marker");
        obj.remove("trace_policy");
        obj.remove("observation_extractors");
        obj.remove("observation_sources");
        obj.remove("artifact_kind");
        obj.remove("channel_visibility");
        obj.insert(
            "compact_line".to_string(),
            serde_json::Value::String(
                crate::evidence_policy::compact_prompt_line_for_output_contract(
                    &route_result.output_contract,
                )
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
        "structured_output_projection": provider_safe_structured_output_projection(step),
        "output_excerpt_present": step.output_excerpt.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "output_excerpt_hash": step.output_excerpt.as_deref().map(provider_safe_excerpt_hash),
        "error_excerpt_present": step.error_excerpt.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "error_excerpt_hash": step.error_excerpt.as_deref().map(provider_safe_excerpt_hash),
    })
}

fn provider_safe_structured_output_projection(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> Option<String> {
    const MAX_PROJECTION_CHARS: usize = 6_000;

    let output = step.output_excerpt.as_deref()?.trim();
    serde_json::from_str::<serde_json::Value>(output).ok()?;
    let normalized =
        crate::agent_engine::observed_output::normalized_success_body_for_observed_output(output);
    let sanitized = crate::visible_text::sanitize_user_visible_text(&normalized);
    serde_json::from_str::<serde_json::Value>(&sanitized).ok()?;
    let mut chars = sanitized.chars();
    let projection = chars
        .by_ref()
        .take(MAX_PROJECTION_CHARS)
        .collect::<String>();
    if chars.next().is_none() {
        Some(projection)
    } else {
        Some(format!("{projection}...(truncated)"))
    }
}

fn provider_safe_capability_result_evidence(
    result: &claw_core::capability_result::CapabilityResultEnvelope,
) -> serde_json::Value {
    const MAX_STRUCTURED_RESULT_CHARS: usize = 8_000;

    let Ok(serialized) = serde_json::to_string(result) else {
        return json!({
            "projection": "unavailable",
            "reason_code": "capability_result_serialize_failed",
        });
    };
    let sanitized = crate::visible_text::sanitize_user_visible_text(&serialized);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&sanitized) else {
        return json!({
            "projection": "unavailable",
            "reason_code": "capability_result_sanitize_failed",
        });
    };
    let sanitized_chars = sanitized.chars().count();
    if sanitized_chars <= MAX_STRUCTURED_RESULT_CHARS {
        return json!({
            "projection": "structured_result",
            "result": value,
        });
    }

    let mut scalar_facts = Vec::new();
    collect_provider_safe_scalar_facts("", &value, &mut scalar_facts);
    let total_scalar_facts = scalar_facts.len();
    let head = scalar_facts.iter().take(48);
    let tail_start = total_scalar_facts.saturating_sub(48).max(48);
    let tail = scalar_facts.iter().skip(tail_start);
    let bounded_facts = head.chain(tail).cloned().collect::<Vec<_>>();
    json!({
        "projection": "bounded_scalar_facts",
        "truncated": true,
        "original_chars": sanitized_chars,
        "total_scalar_facts": total_scalar_facts,
        "facts": bounded_facts,
    })
}

fn collect_provider_safe_scalar_facts(
    path: &str,
    value: &serde_json::Value,
    out: &mut Vec<serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                let child_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                collect_provider_safe_scalar_facts(&child_path, child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                collect_provider_safe_scalar_facts(&format!("{path}[{index}]"), child, out);
            }
        }
        serde_json::Value::String(text) => {
            const MAX_SCALAR_STRING_CHARS: usize = 512;
            let text_chars = text.chars().count();
            let value = if text_chars <= MAX_SCALAR_STRING_CHARS {
                serde_json::Value::String(text.clone())
            } else {
                serde_json::Value::String(format!(
                    "{}...(truncated)",
                    text.chars()
                        .take(MAX_SCALAR_STRING_CHARS)
                        .collect::<String>()
                ))
            };
            out.push(json!({
                "path": path,
                "value": value,
                "original_chars": text_chars,
            }));
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            out.push(json!({
                "path": path,
                "value": value,
            }));
        }
    }
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
    let capability_results = journal
        .capability_results
        .iter()
        .rev()
        .take(MAX_VERIFIER_STEPS)
        .map(provider_safe_capability_result_evidence)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&json!({
        "step_evidence": steps,
        "capability_result_evidence": capability_results,
    }))
    .unwrap_or_else(|_| "{}".to_string())
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
