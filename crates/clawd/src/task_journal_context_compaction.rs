use serde_json::{json, Value};

pub(super) fn transcript_compaction_records_json(task_observations: &[Value]) -> Option<Value> {
    let records = task_observations
        .iter()
        .filter(|observation| {
            observation.get("observation_kind").and_then(Value::as_str)
                == Some("context_compaction_record")
        })
        .filter_map(|observation| observation.get("record"))
        .filter(|record| record.is_object())
        .take(24)
        .cloned()
        .collect::<Vec<_>>();
    (!records.is_empty()).then(|| Value::Array(records))
}

pub(super) fn record_observation(record: Value) -> Value {
    json!({
        "schema_version": 1,
        "observation_kind": "context_compaction_record",
        "record": record,
    })
}
