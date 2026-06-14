use crate::{
    execution_recipe::{ExecutionRecipeProfile, ExecutionRecipeSpec, ExecutionRecipeTargetScope},
    RiskCeiling, RouteResult, SelfExtensionMode,
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

    if route_result.needs_clarify || !route_result.is_execute_gate() {
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

    if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::GeneratedFileDelivery
            | crate::OutputSemanticKind::GeneratedFilePathReport
    ) && (route_result.output_contract.delivery_required
        || route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::GeneratedFilePathReport)
    {
        return SafetyClassDecision {
            risk_ceiling: RiskCeiling::High,
            reason: "generated_file_delivery_route",
        };
    }

    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ConfigMutation {
        return SafetyClassDecision {
            risk_ceiling: RiskCeiling::High,
            reason: "config_mutation_route",
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

    if route_result.is_execute_gate() {
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
#[path = "safety_class_tests.rs"]
mod tests;
