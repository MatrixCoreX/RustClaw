use super::*;

pub(super) fn stable_trace_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64:{hash:016x}")
}

#[derive(Debug, Default)]
pub(super) struct TraceStorageStats {
    pub(super) truncated_arrays: usize,
    pub(super) omitted_array_items: usize,
    pub(super) truncated_strings: usize,
}

pub(super) fn trace_json_bytes(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

pub(super) fn trace_json_hash(value: &Value) -> String {
    serde_json::to_string(value)
        .map(|text| stable_trace_hash(&text))
        .unwrap_or_else(|_| stable_trace_hash("<unserializable-trace>"))
}

pub(super) fn compact_result_trace_value(
    value: &mut Value,
    stats: &mut TraceStorageStats,
    max_array_items: usize,
    max_string_chars: usize,
) {
    match value {
        Value::String(text) => {
            if text.chars().count() > max_string_chars {
                let mut truncated = crate::utf8_safe_prefix(text, max_string_chars).to_string();
                truncated.push_str("...(truncated)");
                *text = truncated;
                stats.truncated_strings += 1;
            }
        }
        Value::Array(items) => {
            if items.len() > max_array_items {
                stats.truncated_arrays += 1;
                stats.omitted_array_items += items.len() - max_array_items;
                items.truncate(max_array_items);
            }
            for item in items {
                compact_result_trace_value(item, stats, max_array_items, max_string_chars);
            }
        }
        Value::Object(map) => {
            for child in map.values_mut() {
                compact_result_trace_value(child, stats, max_array_items, max_string_chars);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

pub(super) fn result_trace_storage_meta(
    original_bytes: usize,
    stored_bytes: usize,
    original_hash: String,
    stats: &TraceStorageStats,
    truncated: bool,
) -> Value {
    json!({
        "schema_version": 1,
        "max_bytes": MAX_RESULT_TRACE_BYTES,
        "truncated": truncated,
        "original_bytes": original_bytes,
        "stored_bytes": stored_bytes,
        "original_hash": original_hash,
        "truncated_arrays": stats.truncated_arrays,
        "omitted_array_items": stats.omitted_array_items,
        "truncated_strings": stats.truncated_strings,
    })
}

pub(super) fn insert_result_trace_storage_meta(trace: &mut Value, meta: Value) {
    if let Some(obj) = trace.as_object_mut() {
        obj.insert("trace_storage".to_string(), meta);
    }
}

pub(super) fn result_trace_json_with_storage_limit(mut trace: Value) -> Value {
    let original_bytes = trace_json_bytes(&trace);
    let original_hash = trace_json_hash(&trace);
    if original_bytes <= MAX_RESULT_TRACE_BYTES {
        let stats = TraceStorageStats::default();
        let meta =
            result_trace_storage_meta(original_bytes, original_bytes, original_hash, &stats, false);
        insert_result_trace_storage_meta(&mut trace, meta);
        return trace;
    }

    let mut stats = TraceStorageStats::default();
    compact_result_trace_value(
        &mut trace,
        &mut stats,
        MAX_RESULT_TRACE_ARRAY_ITEMS,
        MAX_RESULT_TRACE_STRING_CHARS,
    );
    if trace_json_bytes(&trace) > MAX_RESULT_TRACE_BYTES {
        compact_result_trace_value(
            &mut trace,
            &mut stats,
            MAX_RESULT_TRACE_COMPACT_ARRAY_ITEMS,
            MAX_RESULT_TRACE_COMPACT_STRING_CHARS,
        );
    }
    let stored_bytes = trace_json_bytes(&trace);
    let meta = result_trace_storage_meta(original_bytes, stored_bytes, original_hash, &stats, true);
    insert_result_trace_storage_meta(&mut trace, meta);
    trace
}
