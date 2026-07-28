use claw_core::model_turn::ModelToolDefinition;
use serde_json::json;

pub(super) fn native_capability_loader_tool_definition(
    group_names: &[String],
) -> ModelToolDefinition {
    ModelToolDefinition {
        name: super::super::capability_discovery::RUNTIME_CAPABILITY_LOADER_TOOL.to_string(),
        description: "runtime_capability_catalog_v4; effect=observe; operations=search|expand|load_groups; authorization=canonical_registry; next_action=replan".to_string(),
        input_schema: json!({
            "type": "object",
            "oneOf": [
                {
                    "type": "object",
                    "required": ["op", "query"],
                    "properties": {
                        "op": {"type": "string", "enum": ["search"]},
                        "query": {"type": "string", "minLength": 1}
                    },
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "required": ["op", "capability_refs"],
                    "properties": {
                        "op": {"type": "string", "enum": ["expand"]},
                        "capability_refs": {
                            "type": "array",
                            "minItems": 1,
                            "items": {"type": "string", "minLength": 1}
                        }
                    },
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "required": ["groups"],
                    "properties": {
                        "op": {"type": "string", "enum": ["load_groups"]},
                        "groups": {
                            "type": "array",
                            "minItems": 1,
                            "items": {
                                "type": "string",
                                "enum": group_names
                            }
                        }
                    },
                    "additionalProperties": false
                }
            ]
        }),
        strict: true,
    }
}
