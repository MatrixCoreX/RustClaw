use serde_json::{json, Value};
use sha2::Digest;

use super::{
    context_budget_slots, context_slot_present, ContextTokenScope, ContextWindowPolicy,
    ExecutionContextBudgetTier, TaskContextBundle,
};
use crate::memory;
use crate::{AppState, ClaimedTask};

const CONTINUITY_REF_NAMESPACES: &[&str] = &[
    "artifact",
    "child",
    "constraint",
    "decision",
    "evidence",
    "fact",
    "failure",
    "goal",
    "owner",
    "permission",
    "side_effect",
    "window",
];
const CURRENT_STATE_REF_NAMESPACES: &[&str] = &["next", "open", "risk"];
const SPACED_SCALAR_REF_NAMESPACES: &[&str] = &["owner"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextCompactionPlan {
    pub(crate) generation: u64,
    pub(crate) before_char_count: usize,
    pub(crate) before_token_estimate: usize,
    pub(crate) transcript_char_count: usize,
    pub(crate) threshold_chars: usize,
    pub(crate) provider_context_window_tokens: Option<usize>,
    pub(crate) provider_compaction_threshold_tokens: Option<usize>,
    pub(crate) provider_name: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) token_scope: ContextTokenScope,
    pub(crate) prefix_token_estimate: usize,
    pub(crate) body_token_estimate: usize,
    pub(crate) adjusted_token_estimate: usize,
    pub(crate) reserved_output_tokens: usize,
    pub(crate) reserved_tool_observation_tokens: usize,
    pub(crate) estimator_safety_margin_tokens: usize,
    pub(crate) estimator_multiplier_millis: usize,
    pub(crate) trigger_basis: &'static str,
    pub(crate) context_policy_digest: Option<String>,
    pub(crate) compaction_focus: Option<String>,
    pub(crate) focus_digest: Option<String>,
    pub(crate) trigger_codes: Vec<&'static str>,
    source_refs: Vec<Value>,
    source_task_ids: Vec<String>,
    source_event_range: Value,
    source_event_ranges: Vec<Value>,
}

impl ContextCompactionPlan {
    pub(crate) fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    pub(crate) fn source_snapshot(&self) -> Value {
        json!({
            "schema_version": 1,
            "source_task_ids": self.source_task_ids,
            "source_event_range": self.source_event_range,
            "source_event_ranges": self.source_event_ranges,
        })
    }

    pub(crate) fn hook_metadata(&self) -> Value {
        json!({
            "compaction_kind": "deterministic_context_budget",
            "generation": self.generation,
            "before_char_count": self.before_char_count,
            "before_token_estimate": self.before_token_estimate,
            "transcript_char_count": self.transcript_char_count,
            "threshold_chars": self.threshold_chars,
            "provider_context_window_tokens": self.provider_context_window_tokens,
            "provider_compaction_threshold_tokens": self.provider_compaction_threshold_tokens,
            "provider_name": self.provider_name,
            "model": self.model,
            "token_scope": self.token_scope,
            "prefix_token_estimate": self.prefix_token_estimate,
            "body_token_estimate": self.body_token_estimate,
            "adjusted_token_estimate": self.adjusted_token_estimate,
            "reserved_output_tokens": self.reserved_output_tokens,
            "reserved_tool_observation_tokens": self.reserved_tool_observation_tokens,
            "estimator_safety_margin_tokens": self.estimator_safety_margin_tokens,
            "estimator_multiplier_millis": self.estimator_multiplier_millis,
            "trigger_basis": self.trigger_basis,
            "context_policy_digest": self.context_policy_digest,
            "focus_present": self.compaction_focus.is_some(),
            "focus_char_count": self.compaction_focus.as_deref().map(|value| value.chars().count()).unwrap_or(0),
            "focus_digest": self.focus_digest,
            "trigger_codes": self.trigger_codes,
            "source_ref_count": self.source_refs.len(),
            "source_task_count": self.source_task_ids.len(),
            "source_event_range": self.source_event_range,
        })
    }
}

#[cfg(test)]
pub(crate) fn plan_agent_loop_context_compaction_with_provider_window(
    bundle: &TaskContextBundle,
    provider_context_window_tokens: Option<usize>,
) -> Option<ContextCompactionPlan> {
    let policy = provider_context_window_tokens.map(provider_only_policy);
    plan_context_compaction(bundle, policy.as_ref(), false, None)
}

pub(crate) fn plan_agent_loop_context_compaction_with_policy(
    bundle: &TaskContextBundle,
    policy: &ContextWindowPolicy,
) -> Option<ContextCompactionPlan> {
    plan_context_compaction(bundle, Some(policy), false, None)
}

#[cfg(test)]
pub(crate) fn force_agent_loop_context_compaction_plan(
    bundle: &TaskContextBundle,
    provider_context_window_tokens: Option<usize>,
) -> Option<ContextCompactionPlan> {
    let policy = provider_context_window_tokens.map(provider_only_policy);
    plan_context_compaction(bundle, policy.as_ref(), true, None)
}

pub(crate) fn force_agent_loop_context_compaction_plan_with_policy(
    bundle: &TaskContextBundle,
    policy: Option<&ContextWindowPolicy>,
    compaction_focus: Option<&str>,
) -> Option<ContextCompactionPlan> {
    plan_context_compaction(bundle, policy, true, compaction_focus)
}

fn plan_context_compaction(
    bundle: &TaskContextBundle,
    policy: Option<&ContextWindowPolicy>,
    force: bool,
    compaction_focus: Option<&str>,
) -> Option<ContextCompactionPlan> {
    let view = bundle.execution_view.as_ref()?;
    let slots = context_budget_slots(view);
    let before_char_count = slots
        .iter()
        .filter(|(_, value)| context_slot_present(value))
        .map(|(_, value)| value.chars().count())
        .sum::<usize>();
    let before_token_estimate = slots
        .iter()
        .filter(|(_, value)| context_slot_present(value))
        .map(|(_, value)| crate::token_estimator::estimate_generic_tokens(value).provider_tokens)
        .sum::<usize>();
    let transcript_char_count = [
        view.recent_turns_full.as_str(),
        view.last_turn_full.as_str(),
    ]
    .into_iter()
    .filter(|value| context_slot_present(value))
    .map(|value| value.chars().count())
    .sum::<usize>();
    let prefix_token_estimate = [
        view.runtime_context.as_str(),
        view.goal_context.as_str(),
        view.active_task_context.as_str(),
        view.active_execution_anchor_context.as_str(),
        view.session_alias_context.as_str(),
        view.compacted_history_context.as_str(),
    ]
    .into_iter()
    .filter(|value| context_slot_present(value))
    .map(|value| crate::token_estimator::estimate_generic_tokens(value).provider_tokens)
    .sum::<usize>();
    let mut trigger_codes = Vec::new();
    let decision =
        policy.map(|policy| policy.evaluate(before_token_estimate, prefix_token_estimate));
    if decision.as_ref().is_some_and(|decision| decision.trigger) {
        trigger_codes.push("provider_context_window_pressure");
    }
    if force {
        trigger_codes.push("explicit_conversation_compaction");
    }
    if trigger_codes.is_empty() {
        return None;
    }
    let focus = compaction_focus
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let focus_digest = focus
        .as_deref()
        .map(|value| format!("sha256:{:x}", sha2::Sha256::digest(value.as_bytes())));
    let source_refs = slots
        .iter()
        .filter(|(_, value)| context_slot_present(value))
        .map(|(source_ref, value)| {
            json!({
                "ref": source_ref,
                "char_count": value.chars().count(),
                "provenance": source_provenance(source_ref),
            })
        })
        .collect();
    Some(ContextCompactionPlan {
        generation: bundle.compaction_records.len() as u64 + 1,
        before_char_count,
        before_token_estimate,
        transcript_char_count,
        threshold_chars: decision
            .as_ref()
            .map(|decision| decision.scoped_input_budget_tokens)
            .map(|tokens| tokens.saturating_mul(4))
            .unwrap_or(usize::MAX),
        provider_context_window_tokens: policy.map(|policy| policy.context_window_tokens),
        provider_compaction_threshold_tokens: decision
            .as_ref()
            .map(|decision| decision.scoped_input_budget_tokens),
        provider_name: policy.map(|policy| policy.provider_name.clone()),
        model: policy.map(|policy| policy.model.clone()),
        token_scope: policy.map_or(ContextTokenScope::Total, |policy| policy.token_scope),
        prefix_token_estimate,
        body_token_estimate: before_token_estimate.saturating_sub(prefix_token_estimate),
        adjusted_token_estimate: decision.as_ref().map_or(before_token_estimate, |decision| {
            decision.adjusted_token_estimate
        }),
        reserved_output_tokens: policy.map_or(0, |policy| policy.output_reserve_tokens),
        reserved_tool_observation_tokens: policy
            .map_or(0, |policy| policy.tool_observation_reserve_tokens),
        estimator_safety_margin_tokens: policy
            .map_or(0, |policy| policy.estimator_safety_margin_tokens),
        estimator_multiplier_millis: policy
            .map_or(1_000, |policy| policy.estimator_multiplier_millis),
        trigger_basis: decision
            .as_ref()
            .map_or("explicit_compaction", |decision| decision.trigger_basis),
        context_policy_digest: policy.map(|policy| policy.policy_digest.clone()),
        compaction_focus: focus,
        focus_digest,
        trigger_codes,
        source_refs,
        source_task_ids: bundle.context_source_task_ids.clone(),
        source_event_range: json!({"start": Value::Null, "end": Value::Null}),
        source_event_ranges: Vec::new(),
    })
}

#[cfg(test)]
fn provider_only_policy(context_window_tokens: usize) -> ContextWindowPolicy {
    ContextWindowPolicy::new(
        "provider-window-only".to_string(),
        "unspecified".to_string(),
        context_window_tokens,
        0,
        0,
        0,
        1_000,
        ContextTokenScope::Total,
    )
}

pub(crate) fn hydrate_agent_loop_context_compaction_plan(
    state: &AppState,
    task: &ClaimedTask,
    plan: &mut ContextCompactionPlan,
) {
    let Ok(db) = state.core.db.get() else {
        return;
    };
    let mut max_generation = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            [&task.task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .as_deref()
        .map(max_compaction_generation)
        .unwrap_or(0);
    if plan.source_task_ids.is_empty() {
        plan.generation = plan.generation.max(max_generation.saturating_add(1));
        return;
    }
    let event_stream_available = db
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type = 'table' AND name = 'task_event_stream'
            )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|present| present != 0)
        .unwrap_or(false);
    let mut source_rows = Vec::new();
    for task_id in &plan.source_task_ids {
        let task_row = db.query_row(
            "SELECT
                CAST(created_at AS TEXT),
                CAST(updated_at AS TEXT),
                result_json
             FROM tasks
             WHERE task_id = ?1
             LIMIT 1",
            [task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        );
        let Ok((created_at, updated_at, result_json)) = task_row else {
            continue;
        };
        if let Some(result_json) = result_json.as_deref() {
            max_generation = max_generation.max(max_compaction_generation(result_json));
        }
        let (start_seq, end_seq) = if event_stream_available {
            db.query_row(
                "SELECT MIN(seq), MAX(seq)
                 FROM task_event_stream
                 WHERE task_id = ?1",
                [task_id],
                |row| Ok((row.get::<_, Option<u64>>(0)?, row.get::<_, Option<u64>>(1)?)),
            )
            .unwrap_or((None, None))
        } else {
            (None, None)
        };
        source_rows.push(json!({
            "task_id": task_id,
            "created_at": created_at,
            "updated_at": updated_at,
            "start_seq": start_seq,
            "end_seq": end_seq,
        }));
    }
    plan.generation = plan.generation.max(max_generation.saturating_add(1));
    plan.source_event_ranges = source_rows.clone();
    plan.source_event_range = json!({
        "start": source_rows.first().map(source_range_start).unwrap_or(Value::Null),
        "end": source_rows.last().map(source_range_end).unwrap_or(Value::Null),
    });
}

fn source_range_start(source: &Value) -> Value {
    json!({
        "task_id": source.get("task_id"),
        "timestamp": source.get("created_at"),
        "event_seq": source.get("start_seq"),
    })
}

fn source_range_end(source: &Value) -> Value {
    json!({
        "task_id": source.get("task_id"),
        "timestamp": source.get("updated_at"),
        "event_seq": source.get("end_seq"),
    })
}

fn max_compaction_generation(result_json: &str) -> u64 {
    let Ok(result) = serde_json::from_str::<Value>(result_json) else {
        return 0;
    };
    const RECORD_POINTERS: [&str; 6] = [
        "/task_journal/summary/transcript_compaction_records",
        "/task_journal/trace/transcript_compaction_records",
        "/result/task_journal/summary/transcript_compaction_records",
        "/result/task_journal/trace/transcript_compaction_records",
        "/final_result_json/task_journal/summary/transcript_compaction_records",
        "/final_result_json/task_journal/trace/transcript_compaction_records",
    ];
    RECORD_POINTERS
        .iter()
        .filter_map(|pointer| result.pointer(pointer).and_then(Value::as_array))
        .flat_map(|records| records.iter())
        .filter_map(|record| record.get("generation").and_then(Value::as_u64))
        .max()
        .unwrap_or(0)
}

pub(crate) fn apply_agent_loop_context_compaction(
    state: &AppState,
    task: &ClaimedTask,
    planner_user_request: &str,
    chat_memory_budget_chars: usize,
    bundle: &mut TaskContextBundle,
    plan: &ContextCompactionPlan,
    model_summary: Option<Value>,
    model_status_code: &'static str,
) -> Value {
    let memory_settings_snapshot = memory::settings::revocation_fenced_task_memory_settings(
        state,
        task,
        bundle.memory_settings_snapshot.as_ref(),
    );
    let Some(view) = bundle.execution_view.as_mut() else {
        return Value::Null;
    };
    let has_active_session_state = [
        view.active_task_context.as_str(),
        view.active_execution_anchor_context.as_str(),
        view.session_alias_context.as_str(),
    ]
    .into_iter()
    .any(context_slot_present);
    let planner_memory_decision = memory::use_policy::decide_planner_memory_use_policy(
        state,
        ExecutionContextBudgetTier::Light,
    );
    let chat_memory_decision = memory::use_policy::decide_chat_memory_use_policy(
        state,
        ExecutionContextBudgetTier::Light,
        has_active_session_state,
        chat_memory_budget_chars,
    );
    let compacted_memory_ctx = memory::service::prepare_prompt_with_memory_for_policy_snapshot(
        state,
        task,
        planner_user_request,
        &planner_memory_decision,
        &chat_memory_decision,
        memory_settings_snapshot.as_ref(),
    );
    let compacted_last_turn_total_chars = plan
        .provider_compaction_threshold_tokens
        .unwrap_or(plan.before_token_estimate.max(1))
        .saturating_mul(4)
        .saturating_div(5)
        .max(1);
    let compacted_last_turn_segment_chars =
        compacted_last_turn_total_chars.saturating_div(2).max(1);
    let compacted_last_turn = memory::build_last_turn_full_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        crate::conversation_state::task_conversation_id(task).as_deref(),
        compacted_last_turn_segment_chars,
        compacted_last_turn_total_chars,
    );
    apply_context_compaction_with_inputs(
        &task.task_id,
        bundle,
        plan,
        compacted_memory_ctx,
        compacted_last_turn,
        model_summary,
        model_status_code,
    )
}

pub(super) fn apply_context_compaction_with_inputs(
    task_id: &str,
    bundle: &mut TaskContextBundle,
    plan: &ContextCompactionPlan,
    compacted_memory_ctx: crate::memory::service::PromptMemoryContext,
    compacted_last_turn: String,
    model_summary: Option<Value>,
    model_status_code: &'static str,
) -> Value {
    let Some(view) = bundle.execution_view.as_mut() else {
        return Value::Null;
    };
    let continuity_refs = deterministic_continuity_refs(view);
    let current_state_refs =
        extract_machine_refs(view.last_turn_full.as_str(), CURRENT_STATE_REF_NAMESPACES);
    let model_summary_attached = model_summary.is_some();
    let mut compacted_summary = model_summary.clone();
    if let Some(summary) = compacted_summary.as_mut() {
        attach_continuity_refs(summary, &continuity_refs);
        attach_current_state_refs(summary, &current_state_refs);
    } else if !continuity_refs.is_empty() {
        compacted_summary = Some(json!({
            "schema_version": 1,
            "summary_kind": "deterministic_machine_reference_continuity",
            "continuity_refs": continuity_refs,
            "current_state_refs": current_state_refs,
        }));
    }
    let continuity_summary_attached = compacted_summary.is_some();
    let input_digest = sha256_json(&json!({
        "source_refs": plan.source_refs,
        "source_event_range": plan.source_event_range,
        "source_event_ranges": plan.source_event_ranges,
    }));
    let output_digest = sha256_json(compacted_summary.as_ref().unwrap_or(&Value::Null));
    view.memory_ctx = compacted_memory_ctx;
    view.budget_tier = ExecutionContextBudgetTier::Light;
    view.recent_turns_full = "<none>".to_string();
    view.last_turn_full = compacted_last_turn;
    view.recent_execution_context = "<none>".to_string();
    view.compacted_history_context = compacted_summary
        .as_ref()
        .map(render_compacted_history_context)
        .unwrap_or_else(|| "<none>".to_string());

    let after_char_count = context_budget_slots(view)
        .iter()
        .filter(|(_, value)| context_slot_present(value))
        .map(|(_, value)| value.chars().count())
        .sum::<usize>();
    let active_goal_refs = context_slot_present(&view.goal_context)
        .then(|| Value::String("goal_context".to_string()))
        .into_iter()
        .collect::<Vec<_>>();
    let artifact_refs = context_slot_present(view.image_context.as_deref().unwrap_or("<none>"))
        .then(|| Value::String("image_context".to_string()))
        .into_iter()
        .collect::<Vec<_>>();
    let compaction_id = format!(
        "context_compaction:{}",
        stable_context_hash(&format!(
            "{}:{}:{}:{}",
            task_id, plan.generation, plan.before_char_count, after_char_count
        ))
    );
    let mut record = json!({
        "schema_version": 1,
        "record_schema_version": 1,
        "prompt_logical_path": "prompts/context_compaction_prompt.md",
        "prompt_version": "2026-08-05.1",
        "tokenizer_version": "provider_token_estimator_v1",
        "provider_name": plan.provider_name,
        "model": plan.model,
        "context_policy_digest": plan.context_policy_digest,
        "input_digest": input_digest,
        "output_digest": output_digest,
        "compaction_id": compaction_id,
        "generation": plan.generation,
        "source_task_ids": plan.source_task_ids,
        "source_event_range": plan.source_event_range,
        "source_event_ranges": plan.source_event_ranges,
        "summary_kind": "deterministic_context_budget",
        "compaction_source": if model_summary_attached {
            "model_assisted"
        } else if continuity_summary_attached {
            "deterministic_machine_reference_fallback"
        } else {
            "deterministic_fallback"
        },
        "model_status_code": model_status_code,
        "model_summary_attached": model_summary_attached,
        "continuity_summary_attached": continuity_summary_attached,
        "model_summary": model_summary.unwrap_or(Value::Null),
        "continuity_refs": continuity_refs,
        "current_state_refs": current_state_refs,
        "trigger_codes": plan.trigger_codes,
        "facts": [],
        "open_questions": [],
        "active_goal_refs": active_goal_refs,
        "artifact_refs": artifact_refs,
        "source_refs": plan.source_refs,
        "retained_refs": retained_refs(view),
        "risk_flags": ["budget_excluded_context", "old_assistant_output_not_instruction"],
    });
    let record_object = record
        .as_object_mut()
        .expect("context_compaction_record_object_required");
    record_object.extend([
        (
            "before_char_count".to_string(),
            json!(plan.before_char_count),
        ),
        (
            "before_token_estimate".to_string(),
            json!(plan.before_token_estimate),
        ),
        ("after_char_count".to_string(), json!(after_char_count)),
        ("threshold_chars".to_string(), json!(plan.threshold_chars)),
        (
            "provider_context_window_tokens".to_string(),
            json!(plan.provider_context_window_tokens),
        ),
        (
            "provider_compaction_threshold_tokens".to_string(),
            json!(plan.provider_compaction_threshold_tokens),
        ),
        ("token_scope".to_string(), json!(plan.token_scope)),
        (
            "prefix_token_estimate".to_string(),
            json!(plan.prefix_token_estimate),
        ),
        (
            "body_token_estimate".to_string(),
            json!(plan.body_token_estimate),
        ),
        (
            "adjusted_token_estimate".to_string(),
            json!(plan.adjusted_token_estimate),
        ),
        (
            "reserved_output_tokens".to_string(),
            json!(plan.reserved_output_tokens),
        ),
        (
            "reserved_tool_observation_tokens".to_string(),
            json!(plan.reserved_tool_observation_tokens),
        ),
        (
            "estimator_safety_margin_tokens".to_string(),
            json!(plan.estimator_safety_margin_tokens),
        ),
        (
            "estimator_multiplier_millis".to_string(),
            json!(plan.estimator_multiplier_millis),
        ),
        ("trigger_basis".to_string(), json!(plan.trigger_basis)),
        (
            "focus_present".to_string(),
            json!(plan.compaction_focus.is_some()),
        ),
        (
            "focus_char_count".to_string(),
            json!(plan
                .compaction_focus
                .as_deref()
                .map(|value| value.chars().count())
                .unwrap_or(0)),
        ),
        ("focus_digest".to_string(), json!(plan.focus_digest)),
        (
            "coverage".to_string(),
            json!({
                "status": "projection_bounded_canonical_available",
                "original_events_preserved": true,
                "canonical_source_refs_available": true,
            }),
        ),
    ]);
    bundle.compaction_records.push(record.clone());
    record
}

fn attach_continuity_refs(summary: &mut Value, continuity_refs: &[Value]) {
    let Some(object) = summary.as_object_mut() else {
        return;
    };
    object.insert(
        "continuity_refs".to_string(),
        Value::Array(continuity_refs.to_vec()),
    );
}

fn attach_current_state_refs(summary: &mut Value, current_state_refs: &[String]) {
    let Some(object) = summary.as_object_mut() else {
        return;
    };
    object.insert(
        "current_state_refs".to_string(),
        Value::Array(
            current_state_refs
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
}

fn deterministic_continuity_refs(view: &super::ExecutionContextView) -> Vec<Value> {
    let mut refs = Vec::new();
    for (source_ref, value) in [
        ("runtime_context", view.runtime_context.as_str()),
        ("goal_context", view.goal_context.as_str()),
        ("active_task_context", view.active_task_context.as_str()),
        (
            "active_execution_anchor_context",
            view.active_execution_anchor_context.as_str(),
        ),
        ("session_alias_context", view.session_alias_context.as_str()),
        ("last_turn_full", view.last_turn_full.as_str()),
        (
            "recent_execution_anchor",
            view.recent_execution_anchor.as_str(),
        ),
        (
            "compacted_history_context",
            view.compacted_history_context.as_str(),
        ),
        ("recent_turns_full", view.recent_turns_full.as_str()),
    ] {
        if !context_slot_present(value) {
            continue;
        }
        let machine_refs = if matches!(
            source_ref,
            "runtime_context"
                | "goal_context"
                | "active_task_context"
                | "active_execution_anchor_context"
                | "session_alias_context"
        ) {
            extract_complete_machine_refs(value, CONTINUITY_REF_NAMESPACES)
        } else {
            extract_machine_refs(value, CONTINUITY_REF_NAMESPACES)
        };
        for machine_ref in machine_refs {
            if refs.iter().any(|item: &Value| {
                item.get("ref").and_then(Value::as_str) == Some(machine_ref.as_str())
            }) {
                continue;
            }
            refs.push(json!({
                "ref": machine_ref,
                "source_ref": source_ref,
                "provenance": source_provenance(source_ref),
            }));
        }
    }
    refs
}

fn extract_complete_machine_refs(value: &str, namespaces: &[&str]) -> Vec<String> {
    let mut complete = String::with_capacity(value.len().saturating_add(1));
    complete.push_str(value);
    complete.push('\n');
    extract_machine_refs(&complete, namespaces)
}

fn extract_machine_refs(value: &str, namespaces: &[&str]) -> Vec<String> {
    let bytes = value.as_bytes();
    let mut refs = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_lowercase()
            || index
                .checked_sub(1)
                .is_some_and(|previous| is_machine_ref_char(bytes[previous]))
        {
            index += 1;
            continue;
        }
        let namespace_start = index;
        index += 1;
        while index < bytes.len() && is_machine_namespace_char(bytes[index]) {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b':' {
            continue;
        }
        let namespace = &value[namespace_start..index];
        if !namespaces.contains(&namespace) {
            index += 1;
            continue;
        }
        index += 1;
        let scalar_namespace = SPACED_SCALAR_REF_NAMESPACES.contains(&namespace);
        if scalar_namespace {
            while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
                index += 1;
            }
            if bytes[index..].starts_with(namespace.as_bytes())
                && bytes.get(index + namespace.len()) == Some(&b':')
            {
                index += namespace.len() + 1;
                while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
                    index += 1;
                }
            }
        }
        let value_start = index;
        while index < bytes.len()
            && if scalar_namespace {
                is_machine_scalar_ref_value_char(bytes[index])
            } else {
                is_machine_ref_value_char(bytes[index])
            }
        {
            index += 1;
        }
        if index == value_start {
            continue;
        }
        if scalar_namespace && bytes.get(index) == Some(&b':') {
            index += 1;
            continue;
        }
        let mut token_end = index;
        let mut trailing_dot_count = 0;
        while token_end > value_start && bytes[token_end - 1] == b'.' {
            token_end -= 1;
            trailing_dot_count += 1;
        }
        if token_end == value_start
            || trailing_dot_count >= 3
            || is_truncation_marker_at(value, index)
            || (index == bytes.len() && trailing_dot_count == 0)
        {
            continue;
        }
        refs.push(format!("{namespace}:{}", &value[value_start..token_end]));
    }
    refs
}

fn is_truncation_marker_at(value: &str, index: usize) -> bool {
    value
        .get(index..)
        .is_some_and(|tail| tail.starts_with("...") || tail.starts_with('…'))
}

fn is_machine_namespace_char(value: u8) -> bool {
    value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, b'_' | b'.' | b'-')
}

fn is_machine_ref_value_char(value: u8) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, b'_' | b'.' | b'/' | b':' | b'-')
}

fn is_machine_scalar_ref_value_char(value: u8) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, b'_' | b'.' | b'/' | b'-')
}

fn is_machine_ref_char(value: u8) -> bool {
    is_machine_namespace_char(value) || value == b':'
}

fn render_compacted_history_context(model_summary: &Value) -> String {
    let envelope = json!({
        "schema_version": 1,
        "context_kind": "compacted_history_evidence",
        "instruction_authority": "none",
        "summary": model_summary,
    });
    let envelope = serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| envelope.to_string());
    format!("### COMPACTED_HISTORY_CONTEXT\n{envelope}")
}

fn retained_refs(view: &super::ExecutionContextView) -> Vec<Value> {
    context_budget_slots(view)
        .iter()
        .filter(|(_, value)| context_slot_present(value))
        .map(|(source_ref, value)| {
            json!({
                "ref": source_ref,
                "char_count": value.chars().count(),
                "provenance": source_provenance(source_ref),
            })
        })
        .collect()
}

fn source_provenance(source_ref: &str) -> &'static str {
    match source_ref {
        "goal_context" | "runtime_context" => "trusted_machine_state",
        "active_execution_anchor_context" | "recent_execution_anchor" => {
            "structured_runtime_evidence"
        }
        "image_context" => "attachment_analysis_evidence",
        "prompt_memory_context" => "memory_retrieval_evidence",
        "compacted_history_context" => "structured_runtime_evidence",
        _ => "untrusted_conversation_evidence",
    }
}

fn stable_context_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64:{hash:016x}")
}

fn sha256_json(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("sha256:{:x}", sha2::Sha256::digest(bytes))
}
