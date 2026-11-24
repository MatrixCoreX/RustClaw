use super::project_schema_fields;
use serde_json::json;

fn writer_schema() -> serde_json::Value {
    json!({"type":"object","additionalProperties":false,"required":["content","path"],
        "properties":{"path":{"type":"string"},"content":{"type":"string","minLength":0,"description":"Preserve exact UTF-8 bytes."},
            "deliver_to_user":{"type":"boolean"},"replaces_artifact_refs":{"type":"array","items":{"type":"string"}}}})
}

fn frozen_write_scope() -> serde_json::Value {
    json!({"type":"object","additionalProperties":false,"required":["path","content"],
        "properties":{"path":{"type":"string","enum":["document/result.txt"]},
            "content":{"type":"string","enum":["accepted\n"]},
            "action":{"type":"string","enum":["write_text"]}}})
}

#[test]
fn frozen_closed_scope_hides_ungranted_optional_fields_without_changing_registry() {
    let original = writer_schema();
    let scope = frozen_write_scope();
    let mut projected = original.clone();
    project_schema_fields(&mut projected, &[&scope], 0);
    assert_eq!(
        projected["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["content", "path"]
    );
    assert_eq!(
        projected["properties"]["content"],
        original["properties"]["content"]
    );
    assert_eq!(projected["required"], original["required"]);
    assert!(original["properties"].get("deliver_to_user").is_some());
    assert!(projected["properties"].get("action").is_none());
    assert!(scope["properties"]["content"]["enum"][0]
        .as_str()
        .unwrap()
        .ends_with('\n'));
}

#[test]
fn rejected_extra_field_never_becomes_a_dispatchable_native_action() {
    use claw_core::model_turn::ModelToolCall;
    use std::collections::{BTreeMap, BTreeSet};
    let mut projected = writer_schema();
    project_schema_fields(&mut projected, &[&frozen_write_scope()], 0);
    let schemas = BTreeMap::from([("fixture.write".to_string(), projected)]);
    let groups = BTreeMap::from([(
        "call_fixture_write".to_string(),
        BTreeSet::from(["fixture.write".to_string()]),
    )]);
    let mut call = ModelToolCall {
        id: "scope-call".into(),
        name: "call_fixture_write".into(),
        arguments: json!({"path":"document/result.txt","content":"accepted\n","deliver_to_user":false}),
    };
    let rejected =
        super::super::action_from_native_tool_call_with_schemas(&call, &groups, &schemas, None);
    assert!(matches!(rejected, Err(ref error) if error == "native_plan_args_schema_invalid"));
    call.arguments
        .as_object_mut()
        .unwrap()
        .remove("deliver_to_user");
    assert!(matches!(
        super::super::action_from_native_tool_call_with_schemas(&call, &groups, &schemas, None),
        Ok(crate::AgentAction::CallCapability { .. })
    ));
}

#[test]
fn ordinary_writer_retains_its_declared_delivery_option() {
    let args = json!({"path":"document/result.txt","content":"accepted\n","deliver_to_user":false});
    assert!(
        super::super::native_capability_tools::native_argument_values_valid(
            &writer_schema(),
            &args
        )
    );
}

#[test]
fn closed_alternatives_keep_the_union_of_permitted_fields() {
    let mut schema = json!({"type":"object","properties":{"a":{},"b":{},"c":{}}});
    let left = json!({"type":"object","additionalProperties":false,"properties":{"a":{}}});
    let right = json!({"type":"object","additionalProperties":false,"properties":{"b":{}}});
    project_schema_fields(&mut schema, &[&left, &right], 0);
    assert_eq!(
        schema,
        json!({"type":"object","additionalProperties":false,"properties":{"a":{},"b":{}}})
    );
}

#[test]
fn an_open_alternative_does_not_hide_permitted_fields() {
    let original = writer_schema();
    let mut schema = original.clone();
    project_schema_fields(
        &mut schema,
        &[
            &frozen_write_scope(),
            &json!({"type":"object","additionalProperties":true}),
        ],
        0,
    );
    assert_eq!(schema, original);
}

#[test]
fn nested_closed_objects_and_array_items_are_projected() {
    let mut schema = json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{"keep":{"type":"string"},"extra":{"type":"boolean"}}}}}});
    let scope = json!({"type":"object","additionalProperties":false,"properties":{"items":{"type":"array","items":{"type":"object","additionalProperties":false,"properties":{"keep":{"type":"string","enum":["yes"]}}}}}});
    project_schema_fields(&mut schema, &[&scope], 0);
    assert_eq!(
        schema["properties"]["items"]["items"]["properties"],
        json!({"keep":{"type":"string"}})
    );
    assert_eq!(
        schema["properties"]["items"]["items"]["additionalProperties"],
        false
    );
}
