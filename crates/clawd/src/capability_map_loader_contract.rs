use claw_core::model_turn::ModelToolDefinition;
use serde_json::{json, Value};

pub(crate) const LOADER_TOOL: &str = "load_capability_groups";

fn load_groups_schema(group_names: &[String]) -> Value {
    json!({
        "type": "object",
        "required": ["groups"],
        "properties": {
            "op": {"type": "string", "enum": ["load_groups"]},
            "groups": {"type": "array", "minItems": 1,
                "items": {"type": "string", "enum": group_names}}
        },
        "additionalProperties": false
    })
}

pub(crate) fn tool_definition(group_names: &[String]) -> ModelToolDefinition {
    ModelToolDefinition {
        name: LOADER_TOOL.to_string(),
        description: "runtime_capability_catalog_v4; effect=observe; operations=search|expand|load_groups; group_tokens=exact_registry_skill_or_registry_category; authorization=canonical_registry; next_action=replan".to_string(),
        input_schema: json!({
            "type": "object",
            "oneOf": [
                {"type": "object", "required": ["op", "query"],
                    "properties": {"op": {"type": "string", "enum": ["search"]},
                        "query": {"type": "string", "minLength": 1}},
                    "additionalProperties": false},
                {"type": "object", "required": ["op", "capability_refs"],
                    "properties": {"op": {"type": "string", "enum": ["expand"]},
                        "capability_refs": {"type": "array", "minItems": 1,
                            "items": {"type": "string", "minLength": 1}}},
                    "additionalProperties": false},
                load_groups_schema(group_names)
            ]
        }),
        strict: true,
    }
}

pub(super) fn planner_hint(group_names: &[String]) -> String {
    format!(
        "capability_group_loader=call_tool:{LOADER_TOOL}; args_schema={}; selected_scopes_persist=true",
        load_groups_schema(group_names)
    )
}

#[cfg(test)]
#[path = "capability_map_loader_contract_tests.rs"]
mod tests;
