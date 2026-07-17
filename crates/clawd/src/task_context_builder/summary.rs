use serde_json::Value;

use super::{ExecutionContextView, TaskContextBundle};

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
    let transcript_compaction_records = Value::Array(bundle.compaction_records.clone()).to_string();
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
