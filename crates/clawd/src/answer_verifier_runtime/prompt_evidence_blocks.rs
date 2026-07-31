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
    const MAX_STRUCTURED_RESULT_CHARS: usize = 24_000;

    let identity = result.canonical_evidence_identity();
    let explicit_observation = crate::capability_result::explicit_model_observation(&result.data);
    let evidence_value = explicit_observation
        .map(|observation| {
            json!({
                "schema_version": result.schema_version,
                "capability": result.capability,
                "action": result.action,
                "status": result.status,
                "effect": result.effect,
                "model_observation": observation,
            })
        })
        .unwrap_or_else(|| serde_json::to_value(result).unwrap_or(serde_json::Value::Null));
    let Ok(serialized) = serde_json::to_string(&evidence_value) else {
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
            "evidence_id": identity.evidence_id,
            "sha256": identity.sha256,
            "size_bytes": identity.size_bytes,
            "result": value,
        });
    }
    let observed_evidence =
        crate::task_journal::observed_evidence_from_output(Some(sanitized.as_str()));
    let content_evidence_projection = provider_safe_content_evidence_projection(&value);
    json!({
        "projection": "canonical_evidence_reference",
        "truncated": true,
        "original_chars": sanitized_chars,
        "evidence_id": identity.evidence_id,
        "sha256": identity.sha256,
        "size_bytes": identity.size_bytes,
        "observed_evidence": observed_evidence,
        "content_evidence_projection": content_evidence_projection,
        "recovery": "canonical_evidence_catalog.artifact_range",
    })
}

fn provider_safe_content_evidence_projection(value: &serde_json::Value) -> Vec<serde_json::Value> {
    const MAX_FACTS: usize = 64;
    const MAX_TOTAL_CHARS: usize = 24_000;

    let mut facts = Vec::new();
    let mut remaining_chars = MAX_TOTAL_CHARS;
    collect_provider_safe_content_evidence(
        "",
        value,
        0,
        &mut facts,
        &mut remaining_chars,
        MAX_FACTS,
    );
    facts
}

fn collect_provider_safe_content_evidence(
    prefix: &str,
    value: &serde_json::Value,
    depth: usize,
    out: &mut Vec<serde_json::Value>,
    remaining_chars: &mut usize,
    max_facts: usize,
) {
    const MAX_DEPTH: usize = 12;
    const PRIORITY_KEYS: &[&str] = &[
        "model_observation",
        "items",
        "results",
        "candidates",
        "pages",
        "entries",
        "extra",
        "data",
        "output",
    ];

    if depth > MAX_DEPTH || out.len() >= max_facts || *remaining_chars < 96 {
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for key in PRIORITY_KEYS {
                let Some(child) = map.get(*key) else {
                    continue;
                };
                let field = provider_safe_content_field(prefix, key);
                push_provider_safe_content_fact(
                    &field,
                    key,
                    child,
                    out,
                    remaining_chars,
                    max_facts,
                );
                collect_provider_safe_content_evidence(
                    &field,
                    child,
                    depth + 1,
                    out,
                    remaining_chars,
                    max_facts,
                );
            }
            for (key, child) in map {
                if PRIORITY_KEYS.contains(&key.as_str()) {
                    continue;
                }
                let field = provider_safe_content_field(prefix, key);
                push_provider_safe_content_fact(
                    &field,
                    key,
                    child,
                    out,
                    remaining_chars,
                    max_facts,
                );
                if child.is_object() || child.is_array() {
                    collect_provider_safe_content_evidence(
                        &field,
                        child,
                        depth + 1,
                        out,
                        remaining_chars,
                        max_facts,
                    );
                }
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                if out.len() >= max_facts || *remaining_chars < 96 {
                    break;
                }
                let field = format!("{prefix}[{index}]");
                collect_provider_safe_content_evidence(
                    &field,
                    child,
                    depth + 1,
                    out,
                    remaining_chars,
                    max_facts,
                );
            }
        }
        _ => {}
    }
}

fn push_provider_safe_content_fact(
    field: &str,
    key: &str,
    value: &serde_json::Value,
    out: &mut Vec<serde_json::Value>,
    remaining_chars: &mut usize,
    max_facts: usize,
) {
    const MAX_VALUE_CHARS: usize = 4_000;

    if !provider_safe_content_evidence_leaf(key) || out.len() >= max_facts || *remaining_chars < 96
    {
        return;
    }
    let value_budget = MAX_VALUE_CHARS.min(
        remaining_chars
            .saturating_sub(field.chars().count())
            .saturating_sub(80),
    );
    if value_budget == 0 {
        return;
    }
    let projected = match value {
        serde_json::Value::String(text) => {
            serde_json::Value::String(provider_safe_content_value(text, value_budget))
        }
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) => value.clone(),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) if key == "field_value" => {
            serde_json::Value::String(provider_safe_content_value(
                &value.to_string(),
                value_budget,
            ))
        }
        _ => return,
    };
    let fact = json!({
        "field": field,
        "value": projected,
    });
    let fact_chars = fact.to_string().chars().count();
    if fact_chars > *remaining_chars {
        return;
    }
    *remaining_chars -= fact_chars;
    out.push(fact);
}

fn provider_safe_content_evidence_leaf(key: &str) -> bool {
    matches!(
        key,
        "body_preview"
            | "canonical_url"
            | "content_excerpt"
            | "content_sha256"
            | "description"
            | "excerpt"
            | "field_value"
            | "final_url"
            | "published_at"
            | "snippet"
            | "source"
            | "status"
            | "summary"
            | "text"
            | "title"
            | "url"
    )
}

fn provider_safe_content_field(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

fn provider_safe_content_value(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let value = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_none() {
        value
    } else {
        format!("{value}...(truncated)")
    }
}

pub(in crate::answer_verifier) fn execution_evidence_prompt_block(
    journal: &crate::task_journal::TaskJournal,
) -> String {
    let steps = journal
        .step_results
        .iter()
        .filter(|step| step_can_supply_verifier_prompt_observation(step))
        .map(provider_safe_step_evidence)
        .collect::<Vec<_>>();
    let capability_results = journal
        .capability_results
        .iter()
        .map(provider_safe_capability_result_evidence)
        .collect::<Vec<_>>();
    let canonical_evidence_catalogs = journal
        .task_observations
        .iter()
        .filter(|observation| {
            observation
                .get("owner_layer")
                .and_then(serde_json::Value::as_str)
                == Some("canonical_evidence_store")
        })
        .collect::<Vec<_>>();
    let plan_verifier_rejections = journal
        .task_observations
        .iter()
        .filter(|observation| {
            observation
                .get("owner_layer")
                .and_then(serde_json::Value::as_str)
                == Some("plan_verifier")
                && observation
                    .get("observation_kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("plan_verifier_rejection")
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&json!({
        "step_evidence": steps,
        "capability_result_evidence": capability_results,
        "canonical_evidence_catalogs": canonical_evidence_catalogs,
        "plan_verifier_rejection_evidence": plan_verifier_rejections,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

pub(in crate::answer_verifier) fn current_context_prompt_block(
    journal: &crate::task_journal::TaskJournal,
) -> String {
    let Some(summary) = journal.context_bundle_summary.as_deref() else {
        return "<none>".to_string();
    };
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return "<none>".to_string();
    }
    trimmed.to_string()
}
