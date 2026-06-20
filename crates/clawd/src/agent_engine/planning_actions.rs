use super::plan_step_label;
use crate::{plan_step_from_agent_action, AgentAction, AppState, PlanKind, PlanResult};

pub(super) fn build_plan_result(
    goal: &str,
    raw_plan_text: &str,
    plan_kind: PlanKind,
    actions: &[AgentAction],
) -> PlanResult {
    build_plan_result_with_notes(goal, raw_plan_text, plan_kind, actions, "")
}

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
        steps,
        planner_notes: planner_notes.trim().to_string(),
        plan_kind,
        raw_plan_text: raw_plan_text.to_string(),
    }
}

pub(super) fn has_executable_observation_or_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. }
                | AgentAction::CallTool { .. }
                | AgentAction::CallCapability { .. }
        )
    })
}

pub(super) fn has_tool_or_skill_observation(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. }
                | AgentAction::CallTool { .. }
                | AgentAction::CallCapability { .. }
        )
    })
}

pub(super) fn planned_action_skill_name(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { skill, .. } => Some(skill.as_str()),
        AgentAction::CallTool { tool, .. } => Some(tool.as_str()),
        AgentAction::CallCapability { capability, .. } => Some(capability.as_str()),
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => None,
    }
}

pub(super) fn contains_unavailable_skill_action(state: &AppState, actions: &[AgentAction]) -> bool {
    let enabled_skills = state.get_skills_list();
    if enabled_skills.is_empty() {
        return false;
    }
    actions.iter().any(|action| {
        let Some(skill) = planned_action_skill_name(action) else {
            return false;
        };
        let canonical = state.resolve_canonical_skill_name(skill);
        canonical.trim().is_empty() || !enabled_skills.contains(&canonical)
    })
}
