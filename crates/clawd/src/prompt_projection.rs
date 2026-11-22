//! Lossless model-only encoding of host diagnostic records. Never changes stored evidence.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const RECORD_ENCODING: &str = "ordered_object_runs_v1";
const REPORT_ENCODING_KEY: &str = "host_record_arrays_encoding";

/// Consecutive equal key sets share a header; absent fields and explicit null stay distinct.
pub(crate) fn compact_records(value: &Value) -> Option<Value> {
    let records = value.as_array().filter(|v| (4..=256).contains(&v.len()))?;
    let mut runs = Vec::new();
    let mut columns: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    for record in records {
        let object = record.as_object()?;
        if object.len() > 32 {
            return None;
        }
        let keys = object.keys().cloned().collect::<Vec<_>>();
        if columns.as_ref().is_some_and(|previous| previous != &keys) {
            runs.push(
                json!({"columns": columns.take().unwrap(), "rows": std::mem::take(&mut rows)}),
            );
        }
        columns = Some(keys);
        rows.push(Value::Array(object.values().cloned().collect()));
    }
    runs.push(json!({"columns": columns?, "rows": rows}));
    let compact = json!({"encoding": RECORD_ENCODING, "runs": runs});
    (compact.to_string().len() + 128 < value.to_string().len()).then_some(compact)
}

pub(crate) fn budget_report(report: &Value) -> Value {
    if !report.is_object() || report.get(REPORT_ENCODING_KEY).is_some() {
        return report.clone();
    }
    let mut compact = report.clone();
    let mut changed = false;
    for path in [
        "/included_refs",
        "/excluded_refs",
        "/context_input_inventory/inputs",
    ] {
        if let Some(records) = compact.pointer_mut(path) {
            if let Some(projected) = compact_records(records) {
                *records = projected;
                changed = true;
            }
        }
    }
    if changed {
        compact[REPORT_ENCODING_KEY] = json!(RECORD_ENCODING);
    }
    compact
}

/// Recognize only the host's canonical summary; arbitrary or older summaries stay verbatim.
pub(crate) fn context_summary(summary: &str) -> Option<Value> {
    if !summary.starts_with("execution_view=") {
        return None;
    }
    let (prefix, suffix) = summary.rsplit_once(" context_budget_report=")?;
    let report: Value = serde_json::from_str(suffix).ok()?;
    if report.get("schema_version").and_then(Value::as_u64) != Some(1)
        || !report
            .get("context_input_inventory")
            .is_some_and(Value::is_object)
        || report.to_string() != suffix
    {
        return None;
    }
    let projected = budget_report(&report);
    if projected == report {
        return None;
    }
    Some(json!({"context_summary_encoding":"host_summary_report_v1",
        "context_summary_prefix":prefix,"context_budget_report":projected,
        "original_sha256":format!("sha256:{:x}",Sha256::digest(summary.as_bytes()))}))
}

#[cfg(test)]
#[path = "prompt_projection_tests.rs"]
pub(crate) mod tests;
