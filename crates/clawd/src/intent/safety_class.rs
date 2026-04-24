use crate::{
    execution_recipe::{ExecutionRecipeProfile, ExecutionRecipeSpec, ExecutionRecipeTargetScope},
    RiskCeiling, RouteGateKind, RouteResult, SelfExtensionMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SafetyClassDecision {
    pub(crate) risk_ceiling: RiskCeiling,
    pub(crate) reason: &'static str,
}

pub(crate) fn classify_route_risk_ceiling(
    route_result: &RouteResult,
    execution_recipe_hint: Option<&ExecutionRecipeSpec>,
) -> SafetyClassDecision {
    if !matches!(
        route_result.output_contract.self_extension.mode,
        SelfExtensionMode::None
    ) || route_result.output_contract.self_extension.execute_now
    {
        return SafetyClassDecision {
            risk_ceiling: RiskCeiling::High,
            reason: "self_extension_requested",
        };
    }

    if route_result.needs_clarify || !route_result.ask_mode.is_execute_gate() {
        return SafetyClassDecision {
            risk_ceiling: RiskCeiling::Low,
            reason: "non_mutating_route",
        };
    }

    if let Some(spec) =
        execution_recipe_hint.filter(|spec| !matches!(spec.profile, ExecutionRecipeProfile::None))
    {
        let risk_ceiling = match spec.target_scope {
            ExecutionRecipeTargetScope::System
            | ExecutionRecipeTargetScope::ExternalWorkspace
            | ExecutionRecipeTargetScope::Greenfield => RiskCeiling::High,
            ExecutionRecipeTargetScope::CurrentRepo => match spec.profile {
                ExecutionRecipeProfile::ConfigChange
                | ExecutionRecipeProfile::CodeChange
                | ExecutionRecipeProfile::SkillAuthoring
                | ExecutionRecipeProfile::OpsService => RiskCeiling::Medium,
                ExecutionRecipeProfile::None => RiskCeiling::Unknown,
            },
            ExecutionRecipeTargetScope::Unknown => RiskCeiling::Medium,
        };
        return SafetyClassDecision {
            risk_ceiling,
            reason: "execution_recipe_scope_profile",
        };
    }

    if route_result.wants_file_delivery
        || route_result.output_contract.requires_content_evidence
        || matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return SafetyClassDecision {
            risk_ceiling: RiskCeiling::Low,
            reason: "read_or_delivery_route",
        };
    }

    if matches!(route_result.ask_mode.gate_kind(), RouteGateKind::Execute) {
        return SafetyClassDecision {
            risk_ceiling: RiskCeiling::Medium,
            reason: "action_route_without_recipe",
        };
    }

    SafetyClassDecision {
        risk_ceiling: RiskCeiling::Unknown,
        reason: "insufficient_signal",
    }
}

pub(crate) fn apply_route_risk_ceiling(
    route_result: &mut RouteResult,
    execution_recipe_hint: Option<&ExecutionRecipeSpec>,
) -> SafetyClassDecision {
    let decision = classify_route_risk_ceiling(route_result, execution_recipe_hint);
    route_result.risk_ceiling = decision.risk_ceiling;
    decision
}

#[cfg(test)]
mod tests {
    use super::classify_route_risk_ceiling;
    use crate::{
        execution_recipe::{
            ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeSpec,
            ExecutionRecipeTargetScope,
        },
        AskMode, IntentOutputContract, RiskCeiling, RouteResult, RoutedMode, ScheduleKind,
        SelfExtensionContract, SelfExtensionMode, SelfExtensionTrigger,
    };

    fn base_route(mode: RoutedMode) -> RouteResult {
        RouteResult {
            routed_mode: mode,
            ask_mode: AskMode::from_routed_mode(mode),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract::default(),
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    #[test]
    fn chat_route_is_low_risk() {
        let route = base_route(RoutedMode::Chat);
        let out = classify_route_risk_ceiling(&route, None);
        assert_eq!(out.risk_ceiling, RiskCeiling::Low);
    }

    #[test]
    fn current_repo_code_change_is_medium_risk() {
        let route = base_route(RoutedMode::Act);
        let recipe = ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            profile: ExecutionRecipeProfile::CodeChange,
            target_scope: ExecutionRecipeTargetScope::CurrentRepo,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
        };
        let out = classify_route_risk_ceiling(&route, Some(&recipe));
        assert_eq!(out.risk_ceiling, RiskCeiling::Medium);
    }

    #[test]
    fn system_ops_is_high_risk() {
        let route = base_route(RoutedMode::Act);
        let recipe = ExecutionRecipeSpec {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            profile: ExecutionRecipeProfile::OpsService,
            target_scope: ExecutionRecipeTargetScope::System,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
        };
        let out = classify_route_risk_ceiling(&route, Some(&recipe));
        assert_eq!(out.risk_ceiling, RiskCeiling::High);
    }

    #[test]
    fn self_extension_is_high_risk() {
        let mut route = base_route(RoutedMode::Act);
        route.output_contract.self_extension = SelfExtensionContract {
            mode: SelfExtensionMode::PermanentExtension,
            trigger: SelfExtensionTrigger::ExplicitUserRequest,
            execute_now: false,
        };
        let out = classify_route_risk_ceiling(&route, None);
        assert_eq!(out.risk_ceiling, RiskCeiling::High);
    }

    #[test]
    fn read_only_locator_route_is_low_risk() {
        let mut route = base_route(RoutedMode::Act);
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
        let out = classify_route_risk_ceiling(&route, None);
        assert_eq!(out.risk_ceiling, RiskCeiling::Low);
    }

    #[test]
    fn resume_execution_shortcut_is_still_execute_risk() {
        let mut route = base_route(RoutedMode::Chat);
        route.ask_mode = AskMode::from_legacy(RoutedMode::Chat, false, true);
        let out = classify_route_risk_ceiling(&route, None);
        assert_eq!(out.risk_ceiling, RiskCeiling::Medium);
        assert_eq!(out.reason, "action_route_without_recipe");
    }
}
