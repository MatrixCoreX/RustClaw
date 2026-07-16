use super::plan_step_label;
use crate::{plan_step_from_agent_action, AgentAction, PlanKind, PlanResult};

pub(super) fn build_plan_result_with_notes(
    goal: &str,
    raw_plan_text: &str,
    plan_kind: PlanKind,
    actions: &[AgentAction],
    planner_notes: &str,
) -> PlanResult {
    let mut previous_actionable_step_id: Option<String> = None;
    let mut steps = Vec::new();
    for (idx, action) in actions.iter().enumerate() {
        let step_id = format!("step_{}", idx + 1);
        let depends_on = previous_actionable_step_id
            .as_ref()
            .map(|v| vec![v.clone()])
            .unwrap_or_default();
        let why = plan_step_label(action);
        let step = plan_step_from_agent_action(action, step_id.clone(), depends_on, why);
        if !matches!(action, AgentAction::Think { .. }) {
            previous_actionable_step_id = Some(step_id);
        }
        steps.push(step);
    }
    PlanResult {
        goal: goal.to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: super::planning_output_contract::parse_planner_output_contract(
            raw_plan_text,
        ),
        steps,
        planner_notes: planner_notes.trim().to_string(),
        plan_kind,
        raw_plan_text: raw_plan_text.to_string(),
    }
}
