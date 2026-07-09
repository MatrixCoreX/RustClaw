use serde_json::Value;

use crate::pipeline_types::OutputContractRef;
use crate::{AgentAction, AppState, IntentOutputContract, PlanStep, RouteResult};

pub(crate) fn effective_service_probe_output_contract_for_plan_steps(
    state: &AppState,
    route: &RouteResult,
    steps: &[PlanStep],
) -> Option<IntentOutputContract> {
    if !route_can_upgrade_service_probe(route) || !service_probe_plan_steps_match(state, steps) {
        return None;
    }
    let mut output_contract = route.output_contract.clone();
    output_contract.apply_output_contract_ref(OutputContractRef::new(
        crate::OutputSemanticKind::ServiceStatus,
    ));
    output_contract.requires_content_evidence = true;
    output_contract.locator_kind = crate::OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    Some(output_contract)
}

fn route_can_upgrade_service_probe(route: &RouteResult) -> bool {
    route.is_execute_gate()
        && !route.needs_clarify
        && !route.output_contract.delivery_required
        && !route.wants_file_delivery
        && route.output_contract.requires_content_evidence
        && matches!(
            route.effective_output_contract_semantic_kind(),
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::CommandOutputSummary
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
                | crate::OutputSemanticKind::ServiceStatus
                | crate::OutputSemanticKind::PackageManagerDetection
                | crate::OutputSemanticKind::DockerPs
                | crate::OutputSemanticKind::DockerImages
        )
}

fn service_probe_plan_steps_match(state: &AppState, steps: &[PlanStep]) -> bool {
    let actions = steps
        .iter()
        .filter_map(|step| plan_step_action(state, step))
        .collect::<Vec<_>>();
    service_probe_actions_match(state, &actions)
}

fn service_probe_actions_match(state: &AppState, actions: &[AgentAction]) -> bool {
    let mut saw_probe = false;
    let mut saw_package_probe = false;
    let mut saw_docker_probe = false;

    for action in actions {
        let Some((skill, args)) = action_ref(action) else {
            continue;
        };
        let normalized_skill = state.resolve_canonical_skill_name(skill);
        let Some(action_name) = structured_action_name(args) else {
            return false;
        };
        match normalized_skill.as_str() {
            "package_manager" if action_name == "detect" => {
                saw_probe = true;
                saw_package_probe = true;
            }
            "docker_basic" if docker_probe_action_allowed(action_name) => {
                saw_probe = true;
                saw_docker_probe = true;
            }
            _ => return false,
        }
    }

    saw_probe && (saw_package_probe || saw_docker_probe)
}

fn docker_probe_action_allowed(action: &str) -> bool {
    matches!(action, "version" | "ps" | "images" | "inspect")
}

fn action_ref(action: &AgentAction) -> Option<(&str, &Value)> {
    match action {
        AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
        AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => None,
    }
}

fn plan_step_action(state: &AppState, step: &PlanStep) -> Option<AgentAction> {
    match step.action_type.as_str() {
        "call_skill" => Some(AgentAction::CallSkill {
            skill: step.skill.clone(),
            args: step.args.clone(),
        }),
        "call_tool" => Some(AgentAction::CallTool {
            tool: step.skill.clone(),
            args: step.args.clone(),
        }),
        "call_capability" => crate::capability_resolver::resolve_capability_action_for_state(
            state,
            &step.skill,
            step.args.clone(),
        )
        .or_else(|| {
            Some(AgentAction::CallCapability {
                capability: step.skill.clone(),
                args: step.args.clone(),
            })
        }),
        _ => None,
    }
}

fn structured_action_name(args: &Value) -> Option<&str> {
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
}
