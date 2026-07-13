use super::*;

pub(super) fn structured_respond_terminal_intent_from_pre_loop_clarify_candidate(
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> Option<StructuredRespondTerminalIntent> {
    if !crate::agent_engine::planning_followup::loop_state_has_pre_loop_locator_clarify_candidate(
        loop_state,
    ) || !super::actions_allow_structured_respond_terminal_intent(actions)
    {
        return None;
    }
    let content = actions.iter().find_map(|action| match action {
        AgentAction::Respond { content } => Some(content.trim()),
        _ => None,
    })?;
    if content.is_empty() {
        return None;
    }
    Some(StructuredRespondTerminalIntent {
        terminal_intent: "clarify".to_string(),
        content: Some(content.to_string()),
        clarify_reason_code: Some("pre_loop_boundary_clarify_candidate".to_string()),
        missing_slot: Some("locator".to_string()),
        message_key: Some("clawd.clarify.locator_required".to_string()),
        field_path: Some(
            "agent_loop.boundary_observations.pre_loop_clarify_candidates".to_string(),
        ),
        locator_kind: Some("path".to_string()),
    })
}
