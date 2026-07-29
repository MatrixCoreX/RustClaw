use serde_json::{json, Map, Value};

use super::LoopState;
use crate::{AppState, PlanResult, PlanStep};

const SAFE_SCHEMA_CONSTRAINT_KEYS: &[&str] = &[
    "type",
    "enum",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "pattern",
];

fn executable_for_step<'a>(
    step: &'a PlanStep,
    verify_result: &'a crate::verifier::VerifyResult,
) -> (&'a str, Option<&'a str>) {
    let resolution = verify_result
        .capability_resolutions
        .iter()
        .find(|resolution| resolution.plan_step_id == step.step_id);
    let executable = resolution
        .and_then(|resolution| resolution.record.resolved_ref.as_deref())
        .and_then(|resolved| resolved.split_once(':').map(|(_, value)| value))
        .unwrap_or(step.skill.as_str());
    let capability = resolution
        .and_then(|resolution| resolution.record.canonical_capability_ref.as_deref())
        .or_else(|| (step.action_type == "call_capability").then_some(step.skill.as_str()));
    (executable, capability)
}

fn executable_input_schema(state: &AppState, executable: &str) -> Option<Value> {
    state
        .mcp_tool(executable)
        .map(|tool| tool.input_schema)
        .or_else(|| {
            state
                .skill_manifest(executable)
                .and_then(|manifest| manifest.input_schema)
        })
}

fn schema_at_field<'a>(schema: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = schema;
    for raw_segment in field.split('.') {
        let segment = raw_segment.split('[').next().unwrap_or(raw_segment);
        if !segment.is_empty() {
            current = current.get("properties")?.get(segment)?;
        }
        let array_depth = raw_segment.matches('[').count();
        for _ in 0..array_depth {
            current = current.get("items")?;
        }
    }
    Some(current)
}

fn safe_schema_projection(schema: &Value) -> Value {
    let mut projected = Map::new();
    for key in SAFE_SCHEMA_CONSTRAINT_KEYS {
        if let Some(value) = schema.get(*key) {
            projected.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(projected)
}

fn field_value_kind(step: &PlanStep, field: &str) -> Option<&'static str> {
    let mut current = &step.args;
    for raw_segment in field.split('.') {
        let segment = raw_segment.split('[').next().unwrap_or(raw_segment);
        if !segment.is_empty() {
            current = current.get(segment)?;
        }
    }
    Some(match current {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    })
}

fn constraint_observations(state: &AppState, step: &PlanStep, executable: &str) -> Vec<Value> {
    let schema = executable_input_schema(state, executable);
    let projection = |field: &str| {
        schema
            .as_ref()
            .and_then(|schema| schema_at_field(schema, field))
            .map(safe_schema_projection)
            .unwrap_or_else(|| json!({}))
    };
    let mut observations = Vec::new();
    for violation in
        crate::schema_contract::executable_enum_violations(state, executable, &step.args)
    {
        observations.push(json!({
            "field": violation.field,
            "constraint": "enum",
            "supplied_value_kind": field_value_kind(step, &violation.field),
            "schema": projection(&violation.field),
        }));
    }
    for violation in
        crate::schema_contract::executable_type_constraint_violations(state, executable, &step.args)
    {
        observations.push(json!({
            "field": violation.field,
            "constraint": "type",
            "expected_type": violation.expected,
            "supplied_value_kind": field_value_kind(step, &violation.field),
            "schema": projection(&violation.field),
        }));
    }
    for violation in crate::schema_contract::executable_value_constraint_violations(
        state, executable, &step.args,
    ) {
        observations.push(json!({
            "field": violation.field,
            "constraint": violation.constraint,
            "supplied_value_kind": field_value_kind(step, &violation.field),
            "schema": projection(&violation.field),
        }));
    }
    for violation in crate::schema_contract::executable_unknown_argument_violations(
        state, executable, &step.args,
    ) {
        observations.push(json!({
            "field": violation.field,
            "constraint": "declared_property",
            "supplied_value_kind": field_value_kind(step, &violation.field),
        }));
    }
    for violation in crate::schema_contract::executable_nested_required_constraint_violations(
        state, executable, &step.args,
    ) {
        observations.push(json!({
            "field": violation.field,
            "constraint": "schema_required",
            "schema": projection(&violation.field),
        }));
    }
    if let Some(args) = step.args.as_object() {
        for field in args.keys() {
            if observations
                .iter()
                .any(|observation| observation.get("field").and_then(Value::as_str) == Some(field))
            {
                continue;
            }
            let schema = projection(field);
            if schema.as_object().is_some_and(|schema| !schema.is_empty()) {
                observations.push(json!({
                    "field": field,
                    "constraint": "declared_schema",
                    "supplied_value_kind": field_value_kind(step, field),
                    "schema": schema,
                }));
            }
        }
    }
    observations.sort_by(|left, right| {
        left.get("field")
            .and_then(Value::as_str)
            .cmp(&right.get("field").and_then(Value::as_str))
            .then_with(|| {
                left.get("constraint")
                    .and_then(Value::as_str)
                    .cmp(&right.get("constraint").and_then(Value::as_str))
            })
    });
    observations
}

pub(super) fn record_plan_verifier_rejection_observation(
    state: &AppState,
    loop_state: &mut LoopState,
    plan_result: &PlanResult,
    verify_result: &crate::verifier::VerifyResult,
) {
    if verify_result.mode != crate::verifier::VerifyMode::Enforce || verify_result.approved {
        return;
    }
    let issues = verify_result
        .issues
        .iter()
        .filter(|issue| crate::verifier::issue_blocks_in_enforce(issue.kind))
        .filter_map(|issue| {
            let step = plan_result
                .steps
                .iter()
                .find(|step| step.step_id == issue.step_id)?;
            let (executable, capability) = executable_for_step(step, verify_result);
            Some(json!({
                "step_id": issue.step_id,
                "status": "error",
                "error_code": issue.kind.status_code(),
                "message_key": issue.kind.message_key(),
                "retryable": false,
                "planner_repairable": super::loop_control::plan_verifier_rejection_is_repairable(verify_result),
                "failure_attribution": issue.kind.failure_attribution().as_str(),
                "requested_capability": capability,
                "resolved_executable": executable,
                "missing_fields": issue.missing_fields,
                "argument_constraints": constraint_observations(state, step, executable),
            }))
        })
        .collect::<Vec<_>>();
    if issues.is_empty() {
        return;
    }
    let observation = json!({
        "schema_version": 1,
        "observation_kind": "plan_verifier_rejection",
        "owner_layer": "plan_verifier",
        "status": "error",
        "issues": issues,
    });
    if !loop_state
        .task_observations
        .iter()
        .any(|existing| existing == &observation)
    {
        loop_state.task_observations.push(observation);
    }
}

#[cfg(test)]
#[path = "plan_verifier_observation_tests.rs"]
mod tests;
