use std::collections::BTreeSet;

use claw_core::capability_result::{CapabilityResultEnvelope, CapabilityResultStatus};
use claw_core::model_turn::ModelTurnRequest;

use super::super::LoopState;
use crate::capability_map::PlannerNativeCapabilityGroup;

pub(super) struct RequiredCompanionRepairContext {
    pub(super) capabilities: Vec<String>,
    pub(super) available_tool_names: Vec<String>,
    pub(super) suggested_group_names: Vec<String>,
}

pub(super) fn required_companion_repair_context(
    error_code: &str,
    request: &ModelTurnRequest,
    all_groups: &[PlannerNativeCapabilityGroup],
    loadable_group_names: &[String],
    loop_state: Option<&LoopState>,
) -> RequiredCompanionRepairContext {
    let capabilities = if error_code == "native_plan_required_companion_capability_missing" {
        loop_state
            .map(missing_required_companion_capabilities)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let directly_available = capabilities
        .iter()
        .filter(|capability| {
            all_groups.iter().any(|group| {
                group.capability_names.contains(capability)
                    && companion_tool_names(group, capability)
                        .iter()
                        .any(|name| request.tools.iter().any(|tool| tool.name.eq(name)))
            })
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let available_tool_names = request
        .tools
        .iter()
        .filter(|tool| {
            directly_available.iter().any(|capability| {
                all_groups.iter().any(|group| {
                    group.capability_names.contains(capability)
                        && companion_tool_names(group, capability).contains(&tool.name)
                })
            })
        })
        .map(|tool| tool.name.clone())
        .collect();
    let suggested_group_names = all_groups
        .iter()
        .filter(|group| {
            loadable_group_names.contains(&group.skill_name)
                && group.capability_names.iter().any(|capability| {
                    capabilities.contains(capability) && !directly_available.contains(capability)
                })
        })
        .map(|group| group.skill_name.clone())
        .collect();
    RequiredCompanionRepairContext {
        capabilities,
        available_tool_names,
        suggested_group_names,
    }
}

fn companion_tool_names(group: &PlannerNativeCapabilityGroup, capability: &str) -> [String; 2] {
    [
        super::native_capability_tools::native_group_leaf_tool_name(group, capability),
        super::native_capability_tools::native_capability_leaf_tool_name(capability),
    ]
}

pub(super) fn missing_required_companion_capabilities(loop_state: &LoopState) -> Vec<String> {
    let successful_capabilities = loop_state
        .capability_results
        .iter()
        .filter(|result| result.status == CapabilityResultStatus::Ok)
        .map(|result| result.capability.as_str())
        .collect::<BTreeSet<_>>();
    let mut missing = BTreeSet::new();
    for observation in &loop_state.task_observations {
        if observation
            .get("observation_kind")
            .and_then(serde_json::Value::as_str)
            != Some("capability_resolution")
            || observation
                .get("outcome")
                .and_then(serde_json::Value::as_str)
                != Some("resolved")
        {
            continue;
        }
        let primary_completed = observation
            .get("requested_capability")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|primary| successful_capabilities.contains(primary));
        if !primary_completed {
            continue;
        }
        let Some(companions) = observation
            .get("required_companions")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for companion in companions.iter().filter_map(serde_json::Value::as_str) {
            if !loop_state
                .capability_results
                .iter()
                .any(|result| companion_observation_is_settled(result, companion))
            {
                missing.insert(companion.to_string());
            }
        }
    }
    missing.into_iter().collect()
}

fn companion_observation_is_settled(result: &CapabilityResultEnvelope, companion: &str) -> bool {
    if result.capability != companion {
        return false;
    }
    match result.status {
        CapabilityResultStatus::Ok => true,
        CapabilityResultStatus::Error => {
            !result.retry.as_ref().is_some_and(|retry| retry.retryable)
        }
        CapabilityResultStatus::Waiting | CapabilityResultStatus::NeedsUser => false,
    }
}

#[cfg(test)]
#[path = "required_companion_capabilities_tests.rs"]
mod tests;
