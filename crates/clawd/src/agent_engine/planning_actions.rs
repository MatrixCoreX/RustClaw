use super::plan_step_label;
use crate::{plan_step_from_agent_action, AgentAction, AppState, PlanKind, PlanResult};

pub(super) fn build_plan_result_with_notes(
    state: Option<&AppState>,
    goal: &str,
    raw_plan_text: &str,
    plan_kind: PlanKind,
    actions: &[AgentAction],
    planner_notes: &str,
) -> PlanResult {
    let dependencies = super::action_batch_contract::planner_action_dependencies(state, actions);
    let mut steps = Vec::new();
    for (idx, action) in actions.iter().enumerate() {
        let step_id = format!("step_{}", idx + 1);
        let why = plan_step_label(action);
        let step = plan_step_from_agent_action(
            action,
            step_id,
            dependencies.get(idx).cloned().unwrap_or_default(),
            why,
        );
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
