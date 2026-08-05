use std::collections::{BTreeMap, BTreeSet};

use claw_core::model_turn::ModelToolDefinition;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{MAX_NATIVE_TOOL_NAME_BYTES, NATIVE_CALL_CAPABILITY_TOOL};

pub(super) fn native_capability_tool_definition(
    tool_name: &str,
    description: &str,
    capability_names: &[String],
    capability_argument_schemas: &BTreeMap<String, Value>,
) -> ModelToolDefinition {
    if tool_name != NATIVE_CALL_CAPABILITY_TOOL {
        if let [capability] = capability_names {
            if let Some(input_schema) = capability_argument_schemas.get(capability) {
                return ModelToolDefinition {
                    name: tool_name.to_string(),
                    description: format!(
                        "{description}; schema:direct_runtime_capability_arguments_v1; capability={capability}"
                    ),
                    input_schema: input_schema.clone(),
                    strict: true,
                };
            }
        }
    }
    let input_schema = if !capability_names.is_empty()
        && capability_names
            .iter()
            .all(|name| capability_argument_schemas.contains_key(name))
    {
        let variants = capability_names
            .iter()
            .map(|capability| {
                json!({
                    "type": "object",
                    "required": ["capability", "args"],
                    "properties": {
                        "capability": {"type": "string", "enum": [capability]},
                        "args": &capability_argument_schemas[capability]
                    },
                    "additionalProperties": false
                })
            })
            .collect::<Vec<_>>();
        json!({
            "type": "object",
            "description": "schema:discriminated_runtime_capability_call_v1",
            "oneOf": variants
        })
    } else {
        json!({
            "type": "object",
            "required": ["capability", "args"],
            "properties": {
                "capability": {
                    "type": "string",
                    "description": "runtime_callable_capability_catalog_v1.token",
                    "enum": capability_names
                },
                "args": {
                    "type": "object",
                    "description": "schema:structured_capability_arguments"
                }
            },
            "additionalProperties": false
        })
    };
    ModelToolDefinition {
        name: tool_name.to_string(),
        description: description.to_string(),
        input_schema,
        strict: true,
    }
}

pub(super) fn native_capability_leaf_tool_name(capability: &str) -> String {
    let readable = capability
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let direct_name = format!("call_{readable}");
    if direct_name.len() <= MAX_NATIVE_TOOL_NAME_BYTES {
        return direct_name;
    }
    let digest = Sha256::digest(capability.as_bytes());
    let suffix = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let suffix = format!("__{suffix}");
    let mut prefix = direct_name;
    prefix.truncate(MAX_NATIVE_TOOL_NAME_BYTES.saturating_sub(suffix.len()));
    format!("{prefix}{suffix}")
}

pub(super) fn native_group_leaf_tool_name(
    group: &crate::capability_map::PlannerNativeCapabilityGroup,
    capability: &str,
) -> String {
    if group.capability_names.len() == 1 {
        group.tool_name.clone()
    } else {
        native_capability_leaf_tool_name(capability)
    }
}

pub(super) fn native_group_tool_definitions(
    group: &crate::capability_map::PlannerNativeCapabilityGroup,
) -> Vec<ModelToolDefinition> {
    group
        .capability_names
        .iter()
        .map(|capability| {
            let description = if group.capability_names.len() == 1 {
                group.description.clone()
            } else {
                group
                    .capability_descriptions
                    .get(capability)
                    .cloned()
                    .unwrap_or_else(|| {
                        format!(
                            "runtime_capability_leaf_v1; source_group={}; capability={capability}; dispatch=resolver_verifier",
                            group.skill_name
                        )
                    })
            };
            native_capability_tool_definition(
                &native_group_leaf_tool_name(group, capability),
                &description,
                std::slice::from_ref(capability),
                &group.capability_argument_schemas,
            )
        })
        .collect()
}

pub(super) fn native_capability_tool_map(
    groups: &[crate::capability_map::PlannerNativeCapabilityGroup],
) -> BTreeMap<String, BTreeSet<String>> {
    groups
        .iter()
        .flat_map(|group| {
            group.capability_names.iter().map(|capability| {
                (
                    native_group_leaf_tool_name(group, capability),
                    BTreeSet::from([capability.clone()]),
                )
            })
        })
        .collect()
}
