use std::time::Duration;

use serde_json::{json, Value};
use tracing::{info, warn};

use crate::task_context_builder::{ContextCompactionPlan, TaskContextBundle};
use crate::{llm_gateway, AppState, ClaimedTask};

const CONTEXT_COMPACTION_PROMPT_LOGICAL_PATH: &str = "prompts/context_compaction_prompt.md";
const CONTEXT_COMPACTION_MAX_TOKENS: u64 = 8_192;
const CONTEXT_COMPACTION_MIN_TIMEOUT_SECONDS: u64 = 120;
const CONTEXT_COMPACTION_MAX_TIMEOUT_SECONDS: u64 = 300;
const CONTEXT_COMPACTION_TIMEOUT_GRACE_SECONDS: u64 = 30;
const MAX_COMPACTION_ITEMS: usize = 64;
const MAX_CONTEXT_SOURCE_TOTAL_CHARS: usize = 48_000;
const MAX_CONTEXT_SOURCE_ITEM_CHARS: usize = 24_000;
const MAX_FACT_VALUE_CHARS: usize = 1_024;
const MAX_REF_CHARS: usize = 256;
const MIN_RESERVED_LLM_CALLS_AFTER_COMPACTION: u64 = 2;
const FORBIDDEN_INSTRUCTION_FIELDS: &[&str] = &[
    "action",
    "actions",
    "capability",
    "command",
    "current_user_instruction",
    "user_instruction",
    "assistant_instruction",
    "next_instruction",
    "next_action",
    "task_directive",
    "tool",
];
const PROVENANCE_TOKENS: &[&str] = &[
    "trusted_machine_state",
    "structured_runtime_evidence",
    "memory_retrieval_evidence",
    "attachment_analysis_evidence",
    "untrusted_conversation_evidence",
];

pub(crate) async fn run_model_assisted_context_compaction(
    state: &AppState,
    task: &ClaimedTask,
    bundle: &TaskContextBundle,
    plan: &ContextCompactionPlan,
) -> (Option<Value>, &'static str) {
    let current_calls = state.task_llm_call_count(&task.task_id);
    if current_calls
        .saturating_add(1)
        .saturating_add(MIN_RESERVED_LLM_CALLS_AFTER_COMPACTION)
        > state.worker.llm_max_calls_per_task
    {
        return (None, "context_compaction_llm_budget_reserved");
    }
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        CONTEXT_COMPACTION_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(error) => {
            warn!(
                "context_compaction_prompt_missing task_id={} error={}",
                task.task_id, error
            );
            return (None, "context_compaction_prompt_missing");
        }
    };
    let source_bundle = context_source_bundle(bundle, plan);
    let source_json =
        serde_json::to_string_pretty(&source_bundle).unwrap_or_else(|_| source_bundle.to_string());
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[("__CONTEXT_SOURCE_BUNDLE__", &source_json)],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "context_compaction_prompt",
        &resolved.source,
        resolved.version.as_deref(),
        None,
    );
    let call = llm_gateway::run_with_fallback_with_hints(
        state,
        task,
        &prompt,
        &resolved.source,
        crate::ChatRequestHints {
            temperature: Some(0.0),
            max_tokens: Some(CONTEXT_COMPACTION_MAX_TOKENS),
        },
    );
    let timeout_seconds = context_compaction_timeout_seconds(
        state
            .task_llm_providers(task)
            .iter()
            .map(|provider| provider.config.timeout_seconds)
            .max(),
    );
    let raw = match tokio::time::timeout(Duration::from_secs(timeout_seconds), call).await {
        Ok(Ok(raw)) => raw,
        Ok(Err(error)) => {
            warn!(
                "context_compaction_provider_failed task_id={} error={}",
                task.task_id,
                crate::truncate_for_log(&error)
            );
            return (None, "context_compaction_provider_failed");
        }
        Err(_) => {
            warn!(
                "context_compaction_provider_timeout task_id={} timeout_seconds={}",
                task.task_id, timeout_seconds
            );
            return (None, "context_compaction_provider_timeout");
        }
    };
    let validated = match crate::prompt_utils::validate_against_schema::<Value>(
        &raw,
        crate::prompt_utils::PromptSchemaId::ContextCompaction,
    ) {
        Ok(validated) => validated.value,
        Err(error) => {
            info!(
                "context_compaction_schema_rejected task_id={} error={}",
                task.task_id, error
            );
            return (None, "context_compaction_schema_rejected");
        }
    };
    let Some(normalized) = normalize_model_assisted_compaction_output(&validated) else {
        return (None, "context_compaction_safety_rejected");
    };
    if !compaction_summary_provenance_valid(&normalized, &source_bundle) {
        return (None, "context_compaction_provenance_rejected");
    }
    info!(
        "context_compaction_model_completed task_id={} output_chars={}",
        task.task_id,
        normalized.to_string().chars().count()
    );
    (Some(normalized), "context_compaction_model_completed")
}

fn context_compaction_timeout_seconds(provider_timeout_seconds: Option<u64>) -> u64 {
    provider_timeout_seconds
        .unwrap_or(CONTEXT_COMPACTION_MIN_TIMEOUT_SECONDS)
        .saturating_add(CONTEXT_COMPACTION_TIMEOUT_GRACE_SECONDS)
        .clamp(
            CONTEXT_COMPACTION_MIN_TIMEOUT_SECONDS,
            CONTEXT_COMPACTION_MAX_TIMEOUT_SECONDS,
        )
}

fn context_source_bundle(bundle: &TaskContextBundle, plan: &ContextCompactionPlan) -> Value {
    let mut sources = Vec::new();
    let mut remaining_chars = MAX_CONTEXT_SOURCE_TOTAL_CHARS;
    if let Some(view) = bundle.execution_view.as_ref() {
        for (source_ref, value) in [
            ("runtime_context", view.runtime_context.as_str()),
            ("goal_context", view.goal_context.as_str()),
            ("active_task_context", view.active_task_context.as_str()),
            ("last_turn_full", view.last_turn_full.as_str()),
            ("recent_turns_full", view.recent_turns_full.as_str()),
            (
                "active_execution_anchor_context",
                view.active_execution_anchor_context.as_str(),
            ),
            ("session_alias_context", view.session_alias_context.as_str()),
            (
                "recent_execution_anchor",
                view.recent_execution_anchor.as_str(),
            ),
            (
                "image_context",
                view.image_context.as_deref().unwrap_or("<none>"),
            ),
            (
                "recent_execution_context",
                view.recent_execution_context.as_str(),
            ),
        ] {
            if !context_value_present(value) || remaining_chars == 0 {
                continue;
            }
            let source_budget = remaining_chars.min(MAX_CONTEXT_SOURCE_ITEM_CHARS);
            let bounded_value = bounded_text(value, source_budget);
            let included_char_count = bounded_value.chars().count();
            remaining_chars = remaining_chars.saturating_sub(included_char_count);
            sources.push(json!({
                "ref": source_ref,
                "provenance": source_provenance(source_ref),
                "char_count": value.chars().count(),
                "included_char_count": included_char_count,
                "truncated": included_char_count < value.chars().count(),
                "value": bounded_value,
            }));
        }
    }
    let included_source_char_count = MAX_CONTEXT_SOURCE_TOTAL_CHARS - remaining_chars;
    json!({
        "schema_version": 1,
        "generation": plan.generation,
        "before_char_count": plan.before_char_count,
        "transcript_char_count": plan.transcript_char_count,
        "threshold_chars": plan.threshold_chars,
        "trigger_codes": plan.trigger_codes,
        "source_char_budget": MAX_CONTEXT_SOURCE_TOTAL_CHARS,
        "included_source_char_count": included_source_char_count,
        "sources": sources,
    })
}

fn compaction_summary_provenance_valid(summary: &Value, source_bundle: &Value) -> bool {
    let Some(sources) = source_bundle.get("sources").and_then(Value::as_array) else {
        return false;
    };
    let source_provenance = |source_ref: &str| {
        sources.iter().find_map(|source| {
            (source.get("ref").and_then(Value::as_str) == Some(source_ref))
                .then(|| source.get("provenance").and_then(Value::as_str))
                .flatten()
        })
    };
    let facts_valid = summary
        .get("facts")
        .and_then(Value::as_array)
        .is_some_and(|facts| {
            facts.iter().all(|fact| {
                let source_ref = fact.get("source_ref").and_then(Value::as_str);
                let provenance = fact.get("provenance").and_then(Value::as_str);
                source_ref.and_then(&source_provenance) == provenance
            })
        });
    let decisions_valid = summary
        .get("decisions")
        .and_then(Value::as_array)
        .is_some_and(|decisions| {
            decisions.iter().all(|decision| {
                decision
                    .get("source_ref")
                    .and_then(Value::as_str)
                    .and_then(&source_provenance)
                    .is_some()
            })
        });
    let refs_valid = summary
        .get("source_refs")
        .and_then(Value::as_array)
        .is_some_and(|refs| {
            refs.iter().all(|source| {
                let source_ref = source.get("ref").and_then(Value::as_str);
                let provenance = source.get("provenance").and_then(Value::as_str);
                source_ref.and_then(&source_provenance) == provenance
            })
        });
    facts_valid && decisions_valid && refs_valid
}

pub(super) fn normalize_model_assisted_compaction_output(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    if object.get("schema_version").and_then(Value::as_u64) != Some(1)
        || object.get("summary_kind").and_then(Value::as_str)
            != Some("model_assisted_context_compaction")
        || contains_forbidden_instruction_field(value)
    {
        return None;
    }
    Some(json!({
        "schema_version": 1,
        "summary_kind": "model_assisted_context_compaction",
        "facts": bounded_fact_array(value)?,
        "decisions": bounded_decision_array(value)?,
        "open_questions": bounded_string_array_field(value, "open_questions", MAX_FACT_VALUE_CHARS, false)?,
        "active_goal_refs": bounded_string_array_field(value, "active_goal_refs", MAX_REF_CHARS, false)?,
        "constraint_refs": bounded_string_array_field(value, "constraint_refs", MAX_REF_CHARS, false)?,
        "evidence_refs": bounded_string_array_field(value, "evidence_refs", MAX_REF_CHARS, false)?,
        "artifact_refs": bounded_string_array_field(value, "artifact_refs", MAX_REF_CHARS, false)?,
        "completed_side_effect_refs": bounded_string_array_field(value, "completed_side_effect_refs", MAX_REF_CHARS, false)?,
        "failure_refs": bounded_string_array_field(value, "failure_refs", MAX_REF_CHARS, false)?,
        "permission_state_refs": bounded_string_array_field(value, "permission_state_refs", MAX_REF_CHARS, false)?,
        "child_task_refs": bounded_string_array_field(value, "child_task_refs", MAX_REF_CHARS, false)?,
        "resume_entrypoint": normalized_resume_entrypoint(value)?,
        "source_refs": bounded_source_ref_array(value)?,
        "risk_flags": bounded_string_array_field(value, "risk_flags", MAX_REF_CHARS, true)?,
    }))
}

fn bounded_fact_array(value: &Value) -> Option<Value> {
    let items = value
        .get("facts")
        .and_then(Value::as_array)?
        .iter()
        .take(MAX_COMPACTION_ITEMS)
        .map(|item| {
            let object = item.as_object()?;
            let fact_key = bounded_machine_token(object.get("fact_key")?.as_str()?, 96)?;
            let fact_value = bounded_non_empty_or_empty(
                object.get("fact_value")?.as_str()?,
                MAX_FACT_VALUE_CHARS,
            );
            let source_ref = bounded_non_empty(object.get("source_ref")?.as_str()?, MAX_REF_CHARS)?;
            let provenance = normalized_provenance(object.get("provenance")?.as_str()?)?;
            Some(json!({
                "fact_key": fact_key,
                "fact_value": fact_value,
                "source_ref": source_ref,
                "provenance": provenance,
            }))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Value::Array(items))
}

fn bounded_decision_array(value: &Value) -> Option<Value> {
    let items = value
        .get("decisions")
        .and_then(Value::as_array)?
        .iter()
        .take(MAX_COMPACTION_ITEMS)
        .map(|item| {
            let object = item.as_object()?;
            let decision_key = bounded_machine_token(object.get("decision_key")?.as_str()?, 96)?;
            let decision_value = bounded_non_empty_or_empty(
                object.get("decision_value")?.as_str()?,
                MAX_FACT_VALUE_CHARS,
            );
            let source_ref = bounded_non_empty(object.get("source_ref")?.as_str()?, MAX_REF_CHARS)?;
            Some(json!({
                "decision_key": decision_key,
                "decision_value": decision_value,
                "source_ref": source_ref,
            }))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Value::Array(items))
}

fn bounded_source_ref_array(value: &Value) -> Option<Value> {
    let items = value
        .get("source_refs")
        .and_then(Value::as_array)?
        .iter()
        .take(MAX_COMPACTION_ITEMS)
        .map(|item| {
            let object = item.as_object()?;
            let source_ref = bounded_non_empty(object.get("ref")?.as_str()?, MAX_REF_CHARS)?;
            let provenance = normalized_provenance(object.get("provenance")?.as_str()?)?;
            Some(json!({"ref": source_ref, "provenance": provenance}))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Value::Array(items))
}

fn bounded_string_array_field(
    value: &Value,
    key: &str,
    max_chars: usize,
    machine_tokens_only: bool,
) -> Option<Value> {
    let items = value
        .get(key)
        .and_then(Value::as_array)?
        .iter()
        .take(MAX_COMPACTION_ITEMS)
        .map(|item| {
            let item = item.as_str()?;
            if machine_tokens_only {
                bounded_machine_token(item, max_chars)
            } else {
                bounded_non_empty(item, max_chars)
            }
        })
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .map(Value::String)
        .collect();
    Some(Value::Array(items))
}

fn normalized_resume_entrypoint(value: &Value) -> Option<Value> {
    match value.get("resume_entrypoint")? {
        Value::Null => Some(Value::Null),
        Value::String(entrypoint) => match entrypoint.as_str() {
            entrypoint @ ("next_planner_round"
            | "poll_async_job"
            | "await_user_input"
            | "verify_and_finalize") => Some(Value::String(entrypoint.to_string())),
            _ => None,
        },
        _ => None,
    }
}

fn contains_forbidden_instruction_field(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, child)| {
            FORBIDDEN_INSTRUCTION_FIELDS.contains(&key.as_str())
                || contains_forbidden_instruction_field(child)
        }),
        Value::Array(items) => items.iter().any(contains_forbidden_instruction_field),
        _ => false,
    }
}

fn bounded_machine_token(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "_.:-".contains(ch))
    {
        return None;
    }
    Some(value.chars().take(max_chars).collect())
}

fn bounded_non_empty(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.chars().take(max_chars).collect())
}

fn bounded_non_empty_or_empty(value: &str, max_chars: usize) -> String {
    value.trim().chars().take(max_chars).collect()
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn normalized_provenance(value: &str) -> Option<&'static str> {
    PROVENANCE_TOKENS
        .iter()
        .copied()
        .find(|candidate| *candidate == value.trim())
}

fn context_value_present(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value != "<none>"
}

fn source_provenance(source_ref: &str) -> &'static str {
    match source_ref {
        "goal_context" | "runtime_context" => "trusted_machine_state",
        "active_execution_anchor_context" | "recent_execution_anchor" => {
            "structured_runtime_evidence"
        }
        "image_context" => "attachment_analysis_evidence",
        "prompt_memory_context" => "memory_retrieval_evidence",
        _ => "untrusted_conversation_evidence",
    }
}

#[cfg(test)]
#[path = "context_compaction_tests.rs"]
mod tests;
