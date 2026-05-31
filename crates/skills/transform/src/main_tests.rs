use super::*;

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
