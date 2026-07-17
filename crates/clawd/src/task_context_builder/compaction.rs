use serde_json::{json, Value};

use super::{
    context_budget_slots, context_slot_present, ExecutionContextBudgetTier, TaskContextBundle,
};
use crate::memory;
use crate::{AppState, ClaimedTask};

const CONTEXT_COMPACTION_THRESHOLD_CHARS: usize = 24_000;
const TRANSCRIPT_COMPACTION_THRESHOLD_CHARS: usize = 12_000;
const COMPACTED_LAST_TURN_SEGMENT_CHARS: usize = 800;
const COMPACTED_LAST_TURN_TOTAL_CHARS: usize = 1_600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextCompactionPlan {
    pub(crate) generation: u64,
    pub(crate) before_char_count: usize,
    pub(crate) transcript_char_count: usize,
    pub(crate) threshold_chars: usize,
    pub(crate) trigger_codes: Vec<&'static str>,
    source_refs: Vec<Value>,
}

impl ContextCompactionPlan {
    pub(crate) fn hook_metadata(&self) -> Value {
        json!({
            "compaction_kind": "deterministic_context_budget",
            "generation": self.generation,
            "before_char_count": self.before_char_count,
            "transcript_char_count": self.transcript_char_count,
            "threshold_chars": self.threshold_chars,
            "trigger_codes": self.trigger_codes,
            "source_ref_count": self.source_refs.len(),
        })
    }
}

pub(crate) fn plan_agent_loop_context_compaction(
    bundle: &TaskContextBundle,
) -> Option<ContextCompactionPlan> {
    let view = bundle.execution_view.as_ref()?;
    let slots = context_budget_slots(view);
    let before_char_count = slots
        .iter()
        .filter(|(_, value)| context_slot_present(value))
        .map(|(_, value)| value.chars().count())
        .sum::<usize>();
    let transcript_char_count = [
        view.recent_turns_full.as_str(),
        view.last_turn_full.as_str(),
    ]
    .into_iter()
    .filter(|value| context_slot_present(value))
    .map(|value| value.chars().count())
    .sum::<usize>();
    let mut trigger_codes = Vec::new();
    if before_char_count > CONTEXT_COMPACTION_THRESHOLD_CHARS {
        trigger_codes.push("context_budget_exceeded");
    }
    if transcript_char_count > TRANSCRIPT_COMPACTION_THRESHOLD_CHARS {
        trigger_codes.push("transcript_budget_exceeded");
    }
    if trigger_codes.is_empty() {
        return None;
    }
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
        transcript_char_count,
        threshold_chars: CONTEXT_COMPACTION_THRESHOLD_CHARS,
        trigger_codes,
        source_refs,
    })
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
        memory::use_policy::PlannerMemoryContextHint::Default,
    );
    let chat_memory_decision = memory::use_policy::decide_chat_memory_use_policy(
        state,
        ExecutionContextBudgetTier::Light,
        has_active_session_state,
        chat_memory_budget_chars,
        memory::use_policy::ChatMemoryContextHint::Default,
    );
    let compacted_memory_ctx = memory::service::prepare_prompt_with_memory_for_policy(
        state,
        task,
        planner_user_request,
        &planner_memory_decision,
        &chat_memory_decision,
    );
    let compacted_last_turn = memory::build_last_turn_full_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        COMPACTED_LAST_TURN_SEGMENT_CHARS,
        COMPACTED_LAST_TURN_TOTAL_CHARS,
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
    view.memory_ctx = compacted_memory_ctx;
    view.budget_tier = ExecutionContextBudgetTier::Light;
    view.recent_turns_full = "<none>".to_string();
    view.last_turn_full = compacted_last_turn;
    view.recent_execution_context = "<none>".to_string();
    view.compacted_history_context = model_summary
        .as_ref()
        .map(render_compacted_history_context)
        .unwrap_or_else(|| "<none>".to_string());

    let model_summary_attached = model_summary.is_some();
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
    let record = json!({
        "schema_version": 1,
        "compaction_id": compaction_id,
        "generation": plan.generation,
        "source_task_ids": [task_id],
        "source_event_range": {"start": Value::Null, "end": Value::Null},
        "summary_kind": "deterministic_context_budget",
        "compaction_source": if model_summary_attached { "model_assisted" } else { "deterministic_fallback" },
        "model_status_code": model_status_code,
        "model_summary_attached": model_summary_attached,
        "model_summary": model_summary.unwrap_or(Value::Null),
        "before_char_count": plan.before_char_count,
        "after_char_count": after_char_count,
        "threshold_chars": plan.threshold_chars,
        "trigger_codes": plan.trigger_codes,
        "facts": [],
        "open_questions": [],
        "active_goal_refs": active_goal_refs,
        "artifact_refs": artifact_refs,
        "source_refs": plan.source_refs,
        "retained_refs": retained_refs(view),
        "risk_flags": ["budget_excluded_context", "old_assistant_output_not_instruction"],
    });
    bundle.compaction_records.push(record.clone());
    record
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
