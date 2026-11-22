use super::*;

pub(crate) fn restore(records: &Value) -> Value {
    if records.get("encoding").and_then(Value::as_str) != Some(RECORD_ENCODING) {
        return records.clone();
    }
    let mut out = Vec::new();
    for run in records["runs"].as_array().unwrap() {
        let columns = run["columns"].as_array().unwrap();
        for row in run["rows"].as_array().unwrap() {
            let row = row.as_array().unwrap();
            assert_eq!(columns.len(), row.len());
            out.push(Value::Object(
                columns
                    .iter()
                    .zip(row)
                    .map(|(k, v)| (k.as_str().unwrap().to_string(), v.clone()))
                    .collect(),
            ));
        }
    }
    Value::Array(out)
}

pub(crate) fn restore_report(mut projected: Value) -> Value {
    for path in [
        "/included_refs",
        "/excluded_refs",
        "/context_input_inventory/inputs",
    ] {
        if let Some(value) = projected.pointer_mut(path) {
            *value = restore(value);
        }
    }
    projected
        .as_object_mut()
        .unwrap()
        .remove(REPORT_ENCODING_KEY);
    projected
}

fn records() -> Value {
    json!((0..24).map(|index| json!({"source_reference":"host_observation",
        "field_reference":format!("field_{index}"),"value": if index%2==0 {json!(null)} else {json!("α\tline\n")},
        "boolean_value":false,"integer_value":18446744073709551615_u64,
        "nested_value":{"encoding":RECORD_ENCODING,"runs":["passive data"]}})).collect::<Vec<_>>())
}

fn report() -> Value {
    json!({"schema_version":1,"budget_tier":"full","included_refs":records(),
        "excluded_refs":records(),"token_estimate":1234,"truncation_reason":"none",
        "context_input_inventory":{"schema_version":1,"inputs":records(),"input_count":24}})
}

#[test]
fn record_runs_restore_exact_values_order_and_null_presence_without_mutating_input() {
    let mut array = records().as_array().unwrap().clone();
    array.extend((0..12).map(|i| json!({"different_long_field_name":i,"absent_value":null,"another_long_field_name":true})));
    array.extend(records().as_array().unwrap().clone());
    let input = Value::Array(array);
    let before = input.clone();
    let encoded = compact_records(&input).unwrap();
    assert!(encoded.to_string().len() + 128 < input.to_string().len());
    assert_eq!(restore(&encoded).to_string(), input.to_string());
    assert_eq!(encoded["runs"].as_array().unwrap().len(), 3);
    assert_eq!(input, before);
}

#[test]
fn unknown_shapes_small_or_unprofitable_arrays_are_preserved() {
    for input in [
        json!(null),
        json!([1, 2, 3, 4]),
        json!([{}, {}, {}, {}]),
        json!([{"a":1}]),
        json!(vec![json!({"a":1}); 257]),
        json!([{"a":1},{"a":2},null,{"a":4}]),
    ] {
        assert!(compact_records(&input).is_none());
    }
}

#[test]
fn budget_view_keeps_all_canonical_diagnostics_and_never_encodes_domain_payloads() {
    let original = report();
    let mut projected = budget_report(&original);
    assert_eq!(projected[REPORT_ENCODING_KEY], RECORD_ENCODING);
    for path in [
        "/included_refs",
        "/excluded_refs",
        "/context_input_inventory/inputs",
    ] {
        let value = restore(projected.pointer(path).unwrap());
        *projected.pointer_mut(path).unwrap() = value;
    }
    projected
        .as_object_mut()
        .unwrap()
        .remove(REPORT_ENCODING_KEY);
    assert_eq!(projected, original);
    assert_eq!(original["included_refs"], records());
    let mut collision = original.clone();
    collision[REPORT_ENCODING_KEY] = json!("opaque-existing-value");
    assert_eq!(budget_report(&collision), collision);
}

#[test]
fn summary_view_can_be_reconstructed_and_arbitrary_or_noncanonical_text_stays_opaque() {
    let original = report();
    let prefix = "execution_view=true execution_budget=full visible_skills=7";
    let summary = format!("{prefix} context_budget_report={original}");
    let projected = context_summary(&summary).unwrap();
    assert_eq!(projected["context_summary_prefix"], prefix);
    assert_eq!(
        projected["original_sha256"],
        format!("sha256:{:x}", Sha256::digest(summary.as_bytes()))
    );
    let mut decoded = projected["context_budget_report"].clone();
    for path in [
        "/included_refs",
        "/excluded_refs",
        "/context_input_inventory/inputs",
    ] {
        let value = restore(decoded.pointer(path).unwrap());
        *decoded.pointer_mut(path).unwrap() = value;
    }
    decoded.as_object_mut().unwrap().remove(REPORT_ENCODING_KEY);
    assert_eq!(format!("{prefix} context_budget_report={decoded}"), summary);
    for text in [
        "observed context".to_string(),
        format!("user context_budget_report={original}"),
        format!(
            "{prefix} context_budget_report={}",
            serde_json::to_string_pretty(&original).unwrap()
        ),
        format!("{prefix} context_budget_report={{\"schema_version\":1}}"),
    ] {
        assert!(context_summary(&text).is_none());
    }
}
