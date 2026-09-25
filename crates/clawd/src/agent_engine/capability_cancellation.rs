use claw_core::skill_registry::{
    CapabilityExecutionMode, PlannerCapabilityEffect, PlannerCapabilityMapping, SkillKind,
};
use serde_json::{json, Value};

use super::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilityCancellationClass {
    ReadOnly,
    CooperativeMutation,
    ReconciliationRequiredMutation,
    SupervisedExternalJob,
}

impl CapabilityCancellationClass {
    fn as_token(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::CooperativeMutation => "cooperative_mutation",
            Self::ReconciliationRequiredMutation => "reconciliation_required_mutation",
            Self::SupervisedExternalJob => "supervised_external_job",
        }
    }

    fn external_effect_state(self) -> &'static str {
        match self {
            Self::ReadOnly => "not_applicable",
            Self::CooperativeMutation => "cleanup_requested",
            Self::ReconciliationRequiredMutation => "outcome_unknown",
            Self::SupervisedExternalJob => "supervisor_reconciliation_required",
        }
    }

    fn settlement_state(self) -> &'static str {
        match self {
            Self::ReadOnly => "settled",
            Self::CooperativeMutation => "cleanup_pending",
            Self::ReconciliationRequiredMutation => "reconciliation_required",
            Self::SupervisedExternalJob => "supervisor_pending",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapabilityCancellationContract {
    pub(crate) class: CapabilityCancellationClass,
    adapter_kind: &'static str,
    effect: &'static str,
}

impl CapabilityCancellationContract {
    pub(crate) fn projection(&self, capability: &str, action: Option<&str>) -> Value {
        json!({
            "schema_version": 1,
            "owner_layer": "capability_runtime",
            "observation_kind": "capability_cancellation",
            "status": "cancel_observed",
            "capability": capability,
            "action": action,
            "adapter_kind": self.adapter_kind,
            "effect": self.effect,
            "cancellation_class": self.class.as_token(),
            "local_execution_state": "stopped",
            "external_effect_state": self.class.external_effect_state(),
            "settlement_state": self.class.settlement_state(),
            "late_result_policy": "record_without_resuming_old_plan",
        })
    }
}

fn action_token(args: &Value) -> Option<&str> {
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn classify_mapping(
    mapping: Option<&PlannerCapabilityMapping>,
    kind: SkillKind,
    handles_task_cancellation: bool,
    fallback_mutates: bool,
    adapter_kind: &'static str,
) -> CapabilityCancellationContract {
    let effect = mapping.and_then(|mapping| mapping.effect);
    let async_execution = mapping.is_some_and(|mapping| {
        mapping.async_adapter_kind.is_some()
            || matches!(
                mapping.execution_mode,
                Some(
                    CapabilityExecutionMode::AsyncPreferred
                        | CapabilityExecutionMode::AsyncRequired
                )
            )
    });
    let effect_token = effect
        .map(PlannerCapabilityEffect::as_token)
        .unwrap_or_else(|| {
            if fallback_mutates {
                "mutate"
            } else {
                "observe"
            }
        });
    let mutates = matches!(
        effect,
        Some(PlannerCapabilityEffect::Mutate | PlannerCapabilityEffect::External)
    ) || (effect.is_none() && fallback_mutates);
    let class = if async_execution {
        CapabilityCancellationClass::SupervisedExternalJob
    } else if !mutates {
        CapabilityCancellationClass::ReadOnly
    } else if kind == SkillKind::Builtin && handles_task_cancellation {
        CapabilityCancellationClass::CooperativeMutation
    } else {
        CapabilityCancellationClass::ReconciliationRequiredMutation
    };
    CapabilityCancellationContract {
        class,
        adapter_kind,
        effect: effect_token,
    }
}

pub(crate) fn cancellation_contract_for_execution(
    state: &AppState,
    capability: &str,
    args: &Value,
    fallback_mutates: bool,
) -> CapabilityCancellationContract {
    if let Some(descriptor) = state.mcp_tool(capability) {
        let class = if descriptor.policy.effect == "observe" {
            CapabilityCancellationClass::ReadOnly
        } else {
            CapabilityCancellationClass::ReconciliationRequiredMutation
        };
        return CapabilityCancellationContract {
            class,
            adapter_kind: "mcp",
            effect: if descriptor.policy.effect == "observe" {
                "observe"
            } else {
                "mutate"
            },
        };
    }

    let manifest = state.skill_manifest(capability);
    let mapping = manifest.as_ref().and_then(|manifest| {
        claw_core::skill_registry::select_planner_capability_mapping(
            &manifest.planner_capabilities,
            action_token(args),
        )
    });
    let kind = manifest
        .as_ref()
        .map(|manifest| manifest.kind)
        .unwrap_or(SkillKind::External);
    classify_mapping(
        mapping,
        kind,
        manifest
            .as_ref()
            .is_some_and(|manifest| manifest.handles_task_cancellation),
        fallback_mutates,
        match kind {
            SkillKind::Builtin => "builtin",
            SkillKind::Runner => "process_runner",
            SkillKind::External => "external_process",
        },
    )
}

#[cfg(test)]
#[path = "capability_cancellation_tests.rs"]
mod tests;
