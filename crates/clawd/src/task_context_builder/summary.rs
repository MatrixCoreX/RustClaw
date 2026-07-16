use serde_json::{json, Value};

use super::{ExecutionContextBudgetTier, ExecutionContextView, TaskContextBundle};

pub(super) fn task_context_bundle_summary(bundle: &TaskContextBundle) -> String {
    let execution_attached = bundle.execution_view.is_some();
    let execution_budget = bundle
        .execution_view
        .as_ref()
        .map(|view| view.budget_tier.as_str())
        .unwrap_or("n/a");
    let execution_profile = bundle
        .execution_view
        .as_ref()
        .map(execution_context_profile)
        .unwrap_or("n/a");
    let context_profile = bundle
        .execution_view
        .as_ref()
        .map(execution_context_profile)
        .unwrap_or("planner_only");
    let visible_skills = bundle.planner_view.visible_skills.len();
    let has_resume_context = value_present(&bundle.raw_sources.resume_context);
    let has_binding_context = value_present(&bundle.raw_sources.binding_context);
    let has_goal_context = bundle
        .execution_view
        .as_ref()
        .is_some_and(|view| value_present(&view.goal_context));
    let context_budget_report = bundle
        .execution_view
        .as_ref()
        .map(super::execution_context_budget_report_json)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "{}".to_string());
    let transcript_compaction_records = transcript_compaction_records_json(bundle).to_string();
    format!(
        "execution_view={} execution_budget={} execution_profile={} context_profile={} visible_skills={} resume_context={} binding_context={} goal_context={} context_budget_report={} transcript_compaction_records={}",
        execution_attached,
        execution_budget,
        execution_profile,
        context_profile,
        visible_skills,
        has_resume_context,
        has_binding_context,
        has_goal_context,
        context_budget_report,
        transcript_compaction_records
    )
}

fn transcript_compaction_records_json(bundle: &TaskContextBundle) -> Value {
    let Some(view) = bundle.execution_view.as_ref() else {
        return Value::Array(Vec::new());
    };
    if !matches!(view.budget_tier, ExecutionContextBudgetTier::Light) {
        return Value::Array(Vec::new());
    }
    let source_refs = transcript_compaction_source_refs(view);
    if source_refs.is_empty() {
        return Value::Array(Vec::new());
    }
    let hash_input = serde_json::to_string(&source_refs).unwrap_or_default();
    let active_goal_refs = if value_present(&view.goal_context) {
        vec![Value::String("goal_context".to_string())]
    } else {
        Vec::new()
    };
    json!([{
        "schema_version": 1,
        "compaction_id": format!("context_compaction:{}", stable_context_hash(&hash_input)),
        "source_task_ids": [],
        "source_event_range": {"start": null, "end": null},
        "summary_kind": "deterministic_context_budget",
        "facts": [],
        "open_questions": [],
        "active_goal_refs": active_goal_refs,
        "artifact_refs": [],
        "source_refs": source_refs,
        "risk_flags": ["budget_excluded_context", "old_assistant_output_not_instruction"],
    }])
}

fn transcript_compaction_source_refs(view: &ExecutionContextView) -> Vec<Value> {
    let budget = super::execution_context_budget_report_json(view);
    budget
        .get("excluded_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| {
            item.get("ref")
                .and_then(Value::as_str)
                .is_some_and(is_transcript_context_ref)
        })
        .cloned()
        .collect()
}

fn is_transcript_context_ref(slot: &str) -> bool {
    matches!(slot, "recent_turns_full" | "last_turn_full")
}

fn stable_context_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64:{hash:016x}")
}

fn execution_context_profile(view: &ExecutionContextView) -> &'static str {
    match view.budget_tier {
        super::ExecutionContextBudgetTier::Light => {
            if value_present(&view.goal_context) {
                "execution_light_goal"
            } else if value_present(&view.active_task_context) {
                "execution_light_active_task"
            } else if value_present(&view.active_execution_anchor_context)
                || value_present(&view.recent_execution_anchor)
                || value_present(&view.recent_execution_context)
            {
                "execution_light_anchor"
            } else {
                "execution_light_bounded"
            }
        }
        super::ExecutionContextBudgetTier::Full => {
            if value_present(&view.goal_context) {
                "execution_full_goal"
            } else if value_present(&view.recent_turns_full)
                || value_present(&view.last_turn_full)
                || value_present(&view.recent_execution_context)
            {
                "execution_full_history"
            } else {
                "execution_full_minimal"
            }
        }
    }
}

fn value_present(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed != "<none>"
}
