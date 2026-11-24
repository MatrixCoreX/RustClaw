use super::*;

fn restore(mut value: Value) -> Value {
    if let Some(pointer) = value
        .pointer("/data/output_reference")
        .and_then(Value::as_str)
    {
        let output = value.pointer(pointer).unwrap().clone();
        value["data"]
            .as_object_mut()
            .unwrap()
            .remove("output_reference");
        value["data"]["output"] = output;
    }
    if value.pointer("/data/output/extra_reference").is_some() {
        value["data"]["output"]
            .as_object_mut()
            .unwrap()
            .remove("extra_reference");
        value["data"]["output"]["extra"] = value["data"]["extra"].clone();
    }
    if let Some(references) = value
        .pointer("/data/output/field_references")
        .and_then(Value::as_object)
        .cloned()
    {
        value["data"]["output"]
            .as_object_mut()
            .unwrap()
            .remove("field_references");
        for (key, pointer) in references {
            value["data"]["output"][key] =
                value.pointer(pointer.as_str().unwrap()).unwrap().clone();
        }
    }
    value
}

#[test]
fn whole_equal_outputs_roundtrip_objects_arrays_and_utf8_without_mutating_evidence() {
    for body in [
        json!({"excerpt":"α\n\0".repeat(300),"count":17}),
        json!(["row".repeat(400), null, false]),
        json!("value\n".repeat(400)),
    ] {
        let original = json!({"schema_version":1,"data":{"extra":body,"output":body,"distinct":"keep"},"error":null});
        let (projected, referenced) = project_capability_evidence(original.clone());
        assert!(referenced);
        assert_eq!(projected["data"]["output_reference"], "/data/extra");
        assert_eq!(restore(projected.clone()), original);
        assert_eq!(
            project_capability_evidence(projected.clone()),
            (projected, false)
        );
    }
}

#[test]
fn partial_equal_fields_keep_distinct_metadata_and_escape_pointer_tokens() {
    let shared = "value".repeat(300);
    let original = json!({"data":{"extra":{"a/b~c":shared,"count":1},"output":{"a/b~c":shared,"count":2,"distinct":"keep"}}});
    let (projected, referenced) = project_capability_evidence(original.clone());
    assert!(referenced);
    assert_eq!(
        projected["data"]["output"]["field_references"]["a/b~c"],
        "/data/extra/a~1b~0c"
    );
    assert_eq!(projected["data"]["output"]["count"], 2);
    assert_eq!(restore(projected), original);
}

#[test]
fn nested_extra_reference_roundtrips_without_discarding_other_output() {
    let extra = json!({"body":"x".repeat(800),"status":"ok"});
    let original = json!({"data":{"extra":extra,"output":{"extra":extra,"result":3}}});
    let (projected, referenced) = project_capability_evidence(original.clone());
    assert!(referenced);
    assert_eq!(restore(projected), original);
}

#[test]
fn reserved_producer_keys_and_unequal_or_small_values_stay_verbatim() {
    let body = json!({"excerpt":"x".repeat(1000)});
    for original in [
        json!({"data":{"extra":body,"output":body,"output_reference":null}}),
        json!({"data":{"extra":body,"output":{"excerpt":"x".repeat(1000),"field_references":"user-data"}}}),
        json!({"data":{"extra":body,"output":{"extra":body,"extra_reference":"user-data"}}}),
        json!({"data":{"extra":{"a":1},"output":{"a":1}}}),
        json!({"data":{"extra":{"a":"x".repeat(1000)},"output":{"a":"y".repeat(1000)}}}),
        json!({"data":{"extra":null,"output":null}}),
        json!({"data":"opaque"}),
    ] {
        assert_eq!(
            project_capability_evidence(original.clone()),
            (original, false)
        );
    }
}

#[test]
fn bounded_equal_previews_cannot_hide_unequal_original_values() {
    let preview =
        json!({"data":{"extra":{"body":"x".repeat(800)},"output":{"body":"x".repeat(800)}}});
    let source = json!({"data":{"extra":{"body":"left"},"output":{"body":"right"}}});
    assert_eq!(
        project_capability_evidence_from_source(preview.clone(), &source),
        (preview, false)
    );
}

#[test]
fn absent_bounded_targets_never_become_references() {
    let body = "x".repeat(800);
    let source = json!({"data":{"extra":{"body":body},"output":{"body":body,"unique":1}}});
    let bounded = json!({"data":{"extra":{},"output":{"body":body,"unique":1}}});
    assert_eq!(
        project_capability_evidence_from_source(bounded.clone(), &source),
        (bounded, false)
    );
}

#[test]
fn medium_repeated_values_are_referenced_only_when_the_complete_map_saves_space() {
    let body = "path value".repeat(16);
    let original = json!({"data":{"extra":{"a":body,"b":body,"c":body},"output":{"a":body,"b":body,"c":body,"unique":1}}});
    let (projected, referenced) = project_capability_evidence(original.clone());
    assert!(referenced);
    assert!(projected.to_string().len() + 128 < original.to_string().len());
    assert_eq!(restore(projected), original);
    let small =
        json!({"data":{"extra":{"a":"x".repeat(100)},"output":{"a":"x".repeat(100),"unique":1}}});
    assert_eq!(project_capability_evidence(small.clone()), (small, false));
}
