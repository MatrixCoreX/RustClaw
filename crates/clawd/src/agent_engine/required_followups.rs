use claw_core::capability_result::{CapabilityResultEnvelope, CapabilityResultStatus};
use serde_json::{json, Map, Value};

use crate::AgentAction;

const ALL_COMPONENTS: &str = "all_components";
const SELECTED_COMPONENTS: &str = "selected_components";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct RequiredFollowup {
    pub(super) component_kind: String,
    pub(super) capability: String,
    pub(super) args: Value,
    pub(super) source_capability: String,
    pub(super) completion_requirement: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptState {
    NotStarted,
    InFlight,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone)]
struct FollowupStep {
    component_kind: String,
    capability: String,
    input_field: String,
    input_value: Value,
    args: Map<String, Value>,
    fallback_capability: Option<String>,
    fallback_input_field: Option<String>,
    fallback_input_value: Option<Value>,
    completion_capabilities: Vec<String>,
    recommended_capability_pointer: Option<String>,
}

fn machine_ref(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 160
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
    .then(|| value.to_string())
}

fn followup_policy(result: &CapabilityResultEnvelope) -> Option<&Value> {
    result
        .data
        .pointer("/extra/content_bundle/followup_policy")
        .or_else(|| {
            result
                .data
                .pointer("/output/extra/content_bundle/followup_policy")
        })
}

fn completion_requirement(policy: &Value) -> Option<String> {
    policy
        .get("completion_requirement")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, ALL_COMPONENTS | SELECTED_COMPONENTS))
        .map(str::to_string)
}

fn parse_steps(policy: &Value) -> Vec<FollowupStep> {
    if completion_requirement(policy).is_none() {
        return Vec::new();
    }
    policy
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let object = value.as_object()?;
            let capability = machine_ref(object.get("capability")?.as_str()?)?;
            let input_field = machine_ref(object.get("input_field")?.as_str()?)?;
            let input_value = object.get("input_value")?.clone();
            let component_kind = object
                .get("component_kind")
                .and_then(Value::as_str)
                .and_then(machine_ref)
                .unwrap_or_else(|| capability.clone());
            let args = object
                .get("args")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let fallback_capability = object
                .get("fallback_capability")
                .and_then(Value::as_str)
                .and_then(machine_ref);
            let fallback_input_field = object
                .get("fallback_input_field")
                .and_then(Value::as_str)
                .and_then(machine_ref);
            let fallback_input_value = object.get("fallback_input_value").cloned();
            let completion_capabilities = object
                .get("completion_capabilities")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(machine_ref)
                .collect();
            let recommended_capability_pointer = object
                .get("recommended_capability_pointer")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|pointer| pointer.starts_with('/') && pointer.len() <= 256)
                .map(str::to_string);
            Some(FollowupStep {
                component_kind,
                capability,
                input_field,
                input_value,
                args,
                fallback_capability,
                fallback_input_field,
                fallback_input_value,
                completion_capabilities,
                recommended_capability_pointer,
            })
        })
        .collect()
}

fn activation_required(policy: &Value) -> bool {
    policy.get("activation_requirement").and_then(Value::as_str) == Some("required")
}

fn attempt_state(results: &[CapabilityResultEnvelope], capability: &str) -> AttemptState {
    results
        .iter()
        .rev()
        .find(|result| result.capability == capability)
        .map_or(AttemptState::NotStarted, |result| match result.status {
            CapabilityResultStatus::Ok => AttemptState::Succeeded,
            CapabilityResultStatus::Waiting | CapabilityResultStatus::NeedsUser => {
                AttemptState::InFlight
            }
            CapabilityResultStatus::Error => AttemptState::Failed,
        })
}

fn latest_result<'a>(
    results: &'a [CapabilityResultEnvelope],
    capability: &str,
) -> Option<&'a CapabilityResultEnvelope> {
    results
        .iter()
        .rev()
        .find(|result| result.capability == capability)
}

fn args_for_capability(step: &FollowupStep, capability: &str) -> Option<Value> {
    let (input_field, input_value) = if step.fallback_capability.as_deref() == Some(capability) {
        (
            step.fallback_input_field.clone()?,
            step.fallback_input_value.clone()?,
        )
    } else {
        (step.input_field.clone(), step.input_value.clone())
    };
    let mut args = step.args.clone();
    args.insert(input_field, input_value);
    Some(Value::Object(args))
}

fn required_action(
    source_capability: &str,
    completion_requirement: &str,
    step: &FollowupStep,
    capability: String,
) -> Option<RequiredFollowup> {
    Some(RequiredFollowup {
        component_kind: step.component_kind.clone(),
        args: args_for_capability(step, &capability)?,
        capability,
        source_capability: source_capability.to_string(),
        completion_requirement: completion_requirement.to_string(),
    })
}

fn recommended_capability(
    step: &FollowupStep,
    preview: &CapabilityResultEnvelope,
) -> Option<String> {
    let pointer = step.recommended_capability_pointer.as_deref()?;
    preview
        .data
        .pointer(pointer)
        .or_else(|| preview.data.pointer(&format!("/output{pointer}")))
        .and_then(Value::as_str)
        .and_then(machine_ref)
        .filter(|capability| step.completion_capabilities.contains(capability))
}

fn action_for_step(
    source_capability: &str,
    completion_requirement: &str,
    step: &FollowupStep,
    later_results: &[CapabilityResultEnvelope],
) -> Option<RequiredFollowup> {
    if !step.completion_capabilities.is_empty() {
        if step.completion_capabilities.iter().any(|capability| {
            matches!(
                attempt_state(later_results, capability),
                AttemptState::Succeeded | AttemptState::InFlight
            )
        }) {
            return None;
        }

        return match attempt_state(later_results, &step.capability) {
            AttemptState::NotStarted => required_action(
                source_capability,
                completion_requirement,
                step,
                step.capability.clone(),
            ),
            AttemptState::InFlight => None,
            AttemptState::Failed => step
                .fallback_capability
                .clone()
                .filter(|fallback| {
                    attempt_state(later_results, fallback) == AttemptState::NotStarted
                })
                .and_then(|fallback| {
                    required_action(source_capability, completion_requirement, step, fallback)
                }),
            AttemptState::Succeeded => {
                let preview = latest_result(later_results, &step.capability)?;
                let recommended = recommended_capability(step, preview)
                    .or_else(|| step.fallback_capability.clone())?;
                match attempt_state(later_results, &recommended) {
                    AttemptState::NotStarted => required_action(
                        source_capability,
                        completion_requirement,
                        step,
                        recommended,
                    ),
                    AttemptState::Failed => step
                        .fallback_capability
                        .clone()
                        .filter(|fallback| {
                            fallback != &recommended
                                && attempt_state(later_results, fallback)
                                    == AttemptState::NotStarted
                        })
                        .and_then(|fallback| {
                            required_action(
                                source_capability,
                                completion_requirement,
                                step,
                                fallback,
                            )
                        }),
                    AttemptState::InFlight | AttemptState::Succeeded => None,
                }
            }
        };
    }

    let primary = attempt_state(later_results, &step.capability);
    if matches!(primary, AttemptState::Succeeded | AttemptState::InFlight) {
        return None;
    }

    let (capability, input_field, input_value) = if primary == AttemptState::Failed {
        let fallback = step.fallback_capability.as_deref()?;
        match attempt_state(later_results, fallback) {
            AttemptState::Succeeded | AttemptState::InFlight | AttemptState::Failed => return None,
            AttemptState::NotStarted => (
                fallback.to_string(),
                step.fallback_input_field.clone()?,
                step.fallback_input_value.clone()?,
            ),
        }
    } else {
        (
            step.capability.clone(),
            step.input_field.clone(),
            step.input_value.clone(),
        )
    };

    let mut args = step.args.clone();
    args.insert(input_field, input_value);
    Some(RequiredFollowup {
        component_kind: step.component_kind.clone(),
        capability,
        args: Value::Object(args),
        source_capability: source_capability.to_string(),
        completion_requirement: completion_requirement.to_string(),
    })
}

/// Return the next required continuation after a structured component bundle
/// has started. A plain media download remains a
/// plain download; an explicitly required selected component is also enforced.
/// Once an all-components conversion starts, every declared component must
/// either succeed or reach an explicit terminal failure.
pub(super) fn next_required_followup(
    results: &[CapabilityResultEnvelope],
) -> Option<RequiredFollowup> {
    for (source_index, source) in results.iter().enumerate().rev() {
        if source.status != CapabilityResultStatus::Ok {
            continue;
        }
        let Some(policy) = followup_policy(source) else {
            continue;
        };
        let Some(requirement) = completion_requirement(policy) else {
            continue;
        };
        let steps = parse_steps(policy);
        if steps.is_empty() || (requirement == ALL_COMPONENTS && steps.len() < 2) {
            continue;
        }
        let later_results = &results[source_index + 1..];
        let activated = activation_required(policy)
            || steps.iter().any(|step| {
                attempt_state(later_results, &step.capability) != AttemptState::NotStarted
                    || step.fallback_capability.as_deref().is_some_and(|fallback| {
                        attempt_state(later_results, fallback) != AttemptState::NotStarted
                    })
                    || step.completion_capabilities.iter().any(|capability| {
                        attempt_state(later_results, capability) != AttemptState::NotStarted
                    })
            });
        if !activated {
            continue;
        }
        if let Some(action) = steps
            .iter()
            .find_map(|step| action_for_step(&source.capability, &requirement, step, later_results))
        {
            return Some(action);
        }
    }
    None
}

fn action_calls_capability(action: &AgentAction, capability: &str) -> bool {
    matches!(
        action,
        AgentAction::CallCapability {
            capability: requested,
            ..
        } if requested == capability
    )
}

/// Schedule the missing machine-declared continuation before any unrelated
/// planner action. This is driven only by a validated capability result
/// contract, never by user-visible prose or a skill-name branch in the main
/// request flow.
pub(super) fn enforce_required_followup(
    actions: &[AgentAction],
    results: &[CapabilityResultEnvelope],
) -> Option<(Vec<AgentAction>, Value)> {
    let required = next_required_followup(results)?;
    if actions
        .iter()
        .any(|action| action_calls_capability(action, &required.capability))
    {
        return None;
    }
    let enforced = vec![AgentAction::CallCapability {
        capability: required.capability.clone(),
        args: required.args.clone(),
    }];
    Some((
        enforced,
        json!({
            "schema_version": 1,
            "observation_kind": "required_followup_enforced",
            "owner_layer": "execution_scheduler",
            "state": "continue",
            "complete": false,
            "completion_requirement": required.completion_requirement,
            "source_capability": required.source_capability,
            "component_kind": required.component_kind,
            "required_capability": required.capability,
            "reason_code": "required_component_not_completed",
            "recovery_action": "execute_machine_declared_followup",
        }),
    ))
}

#[cfg(test)]
#[path = "required_followups_tests.rs"]
mod tests;
