use super::*;

#[test]
fn json_text_preserves_literal_types_and_large_integers() {
    let source = r#"[{"count":4,"code":"04","enabled":true,"absent":null,"id":9007199254740993,"nested":{"amount":2.5,"label":"2.5"}}]"#;
    let expected: Value = serde_json::from_str(source).unwrap();
    let out = handle_transform(&json!({"args": {"json_text": source}})).unwrap();
    assert_eq!(out["output"], expected);
    assert_eq!(out["output"][0]["id"].as_u64(), Some(9007199254740993));
}

#[test]
fn json_text_filter_sort_projection_matches_structured_input() {
    let data = json!([{"label":"amber","score":4},{"label":"jade","score":11},
                     {"label":"pearl","score":1}]);
    let ops = json!([{"op":"filter","field":"score","cmp":"gte","value":4},
                     {"op":"sort","by":"score","order":"desc"},
                     {"op":"project","fields":["label","score"]}]);
    let encoded =
        handle_transform(&json!({"args": {"json_text":data.to_string(),"ops":ops}})).unwrap();
    let direct = handle_transform(&json!({"args": {"data":data,"ops":ops}})).unwrap();
    assert_eq!(encoded, direct);
    assert_eq!(
        encoded["output"],
        json!([{"label":"jade","score":11},{"label":"amber","score":4}])
    );
}

#[test]
fn json_text_supports_empty_scalar_arrays_and_real_item_fields() {
    for source in [
        "[]",
        r#"[true,2,"02",null]"#,
        r#"{"item":[1,2],"code":"02"}"#,
    ] {
        let out = handle_transform(&json!({"json_text":source})).unwrap();
        assert_eq!(
            out["output"],
            serde_json::from_str::<Value>(source).unwrap()
        );
    }
}

#[test]
fn json_text_rejects_ambiguous_sources_even_when_null() {
    for key in ["data", "records", "csv_text", "csv", "text", "input"] {
        let mut args = json!({"json_text":"[]"});
        args[key] = Value::Null;
        assert!(handle_transform(&args).is_err(), "ambiguous input {key}");
    }
}

#[test]
fn json_text_rejects_invalid_json_wrong_types_and_scalar_roots() {
    for value in [
        json!(""),
        json!("[1,]"),
        json!("[1] trailing"),
        json!("true"),
        json!("null"),
        json!("42"),
        json!("\"text\""),
        json!(null),
        json!([]),
    ] {
        assert!(handle_transform(&json!({"json_text":value})).is_err());
    }
}

#[test]
fn transform_error_extra_has_canonical_protocol_fields() {
    let extra = transform_response_extra(
        &json!({"status":"error","error_code":"TRANSFORM_FAILED","result":[]}),
    );
    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], "transform");
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "TRANSFORM_FAILED");
    assert_eq!(extra["message_key"], "skill.transform.transform_failed");
    assert_eq!(extra["retryable"], false);
    assert!(extra.get("error_kind").is_none());
    assert!(extra.get("code").is_none());
}

#[test]
fn filter_sort_projection_preserves_original_numeric_types() {
    let out = handle_transform(&json!({"args": {
        "data": [{"label": "amber", "score": 2}, {"label": "jade", "score": 9},
                 {"label": "pearl", "score": -1}],
        "ops": [{"op": "filter", "field": "score", "cmp": "gte", "value": 2},
                {"op": "sort", "by": ["score"], "order": "desc"},
                {"op": "project", "fields": ["label", "score"]}]
    }}))
    .expect("numeric transformation");
    assert_eq!(
        out["output"],
        json!([{"label": "jade", "score": 9}, {"label": "amber", "score": 2}])
    );
}

#[test]
fn projection_preserves_numeric_strings_booleans_null_and_nested_values() {
    let record = json!({"count": 2, "code": "02", "enabled": true, "absent": null,
                       "nested": {"amount": 2.5, "label": "2.5"}});
    let out = handle_transform(&json!({"args": {
        "data": [record.clone()],
        "ops": [{"op": "project", "fields": ["count", "code", "enabled", "absent", "nested"]}]
    }}))
    .expect("type-preserving projection");
    assert_eq!(out["output"], json!([record]));
}

#[test]
fn scalar_array_dedup_preserves_order_and_value_shapes() {
    let out = handle_transform(&json!({
        "args": {
            "data": ["a", "b", "a", "c", "b", 1, true, null, 1],
            "ops": [{"op": "dedup"}],
            "result_shape": "array"
        }
    }))
    .expect("deduplicate complete JSON values");
    assert_eq!(out["output"], json!(["a", "b", "c", 1, true, null]));
    assert_eq!(out["stats"]["input_count"], 9);
    assert_eq!(out["stats"]["output_count"], 6);
}

#[test]
fn malformed_projection_is_not_silently_reinterpreted() {
    assert!(handle_transform(&json!({
        "args": {
            "data": [{"v": "a"}],
            "ops": [{"op": "project", "fields": {"item": "v"}}]
        }
    }))
    .is_err());
}

#[test]
fn response_extra_preserves_transform_payload() {
    let payload = json!({
        "status": "ok",
        "result": [{"name": "alpha"}],
        "stats": {"input_count": 1, "output_count": 1}
    });

    let extra = transform_response_extra(&payload);

    assert_eq!(
        extra.pointer("/result/0/name").and_then(Value::as_str),
        Some("alpha")
    );
    assert_eq!(
        extra.pointer("/stats/output_count").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        extra.get("action").and_then(Value::as_str),
        Some("transform_data")
    );
    assert_eq!(
        extra.get("source_skill").and_then(Value::as_str),
        Some("transform")
    );
}

#[test]
fn csv_text_can_render_markdown_table() {
    let out = handle_transform(&json!({
        "args": {
            "action": "transform_data",
            "csv_text": "name,score\nalpha,7\nbeta,9",
            "output_format": "md_table"
        }
    }))
    .expect("csv transform");

    let formatted = out
        .get("formatted")
        .and_then(Value::as_str)
        .expect("formatted table");
    assert!(formatted.contains("| name | score |"));
    assert!(formatted.contains("| alpha | 7 |"));
}

#[test]
fn csv_text_accepts_escaped_newline_sequences() {
    let out = handle_transform(&json!({
        "args": {
            "action": "transform_data",
            "csv_text": "name,score\\nalpha,7\\nbeta,9",
            "output_format": "md_table"
        }
    }))
    .expect("escaped csv transform");

    let formatted = out
        .get("formatted")
        .and_then(Value::as_str)
        .expect("formatted table");
    assert!(formatted.contains("| beta | 9 |"));
}

#[test]
fn single_object_rename_outputs_single_object_by_default() {
    let out = handle_transform(&json!({
        "args": {
            "action": "transform_data",
            "data": {"old_name": "alpha", "count": 2},
            "ops": [{"op": "rename", "from": "old_name", "to": "new_name"}]
        }
    }))
    .expect("object rename");

    let output = out.get("output").expect("output");
    assert_eq!(
        output.get("new_name").and_then(Value::as_str),
        Some("alpha")
    );
    assert_eq!(output.get("count").and_then(Value::as_i64), Some(2));
    assert!(output.get("old_name").is_none());
}

#[test]
fn aggregate_can_request_scalar_output() {
    let out = handle_transform(&json!({
        "args": {
            "action": "transform_data",
            "data": [{"value": 4}, {"value": 6}, {"value": 5}],
            "ops": [{"op": "aggregate", "aggregations": [{"op": "sum", "field": "value", "name": "total"}]}],
            "result_shape": "scalar"
        }
    }))
    .expect("aggregate scalar");

    assert_eq!(out.get("output").and_then(Value::as_i64), Some(15));
}
