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

const TRACE_STORAGE_TRUNCATION_MARKER: &str = "...(truncated)";
const MAX_RESULT_TRACE_LOCATOR_CHARS: usize = 4096;

fn is_preserved_trace_locator_key(key: &str) -> bool {
    matches!(
        key,
        "path"
            | "output_path"
            | "output_directory"
            | "resolved_path"
            | "sha256"
            | "filename"
            | "artifact_ref"
            | "id"
            | "artifact_id"
            | "mime_type"
            | "media_type"
            | "download_url"
            | "uri"
    )
}

pub(super) fn compact_result_trace_value(
    value: &mut Value,
    stats: &mut TraceStorageStats,
    max_array_items: usize,
    max_string_chars: usize,
) {
    compact_result_trace_value_at(value, stats, max_array_items, max_string_chars, false);
}

fn compact_result_trace_value_at(
    value: &mut Value,
    stats: &mut TraceStorageStats,
    max_array_items: usize,
    max_string_chars: usize,
    preserve_locator: bool,
) {
    match value {
        Value::String(text) => {
            let limit = if preserve_locator {
                MAX_RESULT_TRACE_LOCATOR_CHARS
            } else {
                max_string_chars
            };
            if text.chars().count() > limit {
                let mut truncated = crate::utf8_safe_prefix(text, limit).to_string();
                truncated.push_str(TRACE_STORAGE_TRUNCATION_MARKER);
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
                compact_result_trace_value_at(
                    item,
                    stats,
                    max_array_items,
                    max_string_chars,
                    false,
                );
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                compact_result_trace_value_at(
                    child,
                    stats,
                    max_array_items,
                    max_string_chars,
                    is_preserved_trace_locator_key(key),
                );
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

fn refresh_stored_bytes(trace: &mut Value) -> usize {
    // The counter is inside the serialized payload; converge after its digit
    // width changes so the advertised bound includes the metadata itself.
    loop {
        let bytes = trace_json_bytes(trace);
        if trace["trace_storage"]["stored_bytes"].as_u64() == Some(bytes as u64) {
            return bytes;
        }
        trace["trace_storage"]["stored_bytes"] = json!(bytes);
    }
}

pub(super) fn result_trace_json_with_storage_limit(mut trace: Value) -> Value {
    let original_bytes = trace_json_bytes(&trace);
    let original_hash = trace_json_hash(&trace);
    let evidence_streams = json!({
        "step_results": trace_json_hash(&trace["step_results"]),
        "capability_results": trace_json_hash(&trace["capability_results"]),
    });
    let mut stats = TraceStorageStats::default();
    let meta = result_trace_storage_meta(original_bytes, 0, original_hash.clone(), &stats, false);
    insert_result_trace_storage_meta(&mut trace, meta);
    if refresh_stored_bytes(&mut trace) <= MAX_RESULT_TRACE_BYTES {
        return trace;
    }

    for (items, chars) in [
        (MAX_RESULT_TRACE_ARRAY_ITEMS, MAX_RESULT_TRACE_STRING_CHARS),
        (
            MAX_RESULT_TRACE_COMPACT_ARRAY_ITEMS,
            MAX_RESULT_TRACE_COMPACT_STRING_CHARS,
        ),
        (4, 120),
        (2, 60),
        (1, 30),
    ] {
        // Never compact the evidence hashes or the storage counters.
        trace
            .as_object_mut()
            .expect("task trace object")
            .remove("trace_storage");
        compact_result_trace_value(&mut trace, &mut stats, items, chars);
        let mut meta =
            result_trace_storage_meta(original_bytes, 0, original_hash.clone(), &stats, true);
        meta["evidence_streams"] = evidence_streams.clone();
        insert_result_trace_storage_meta(&mut trace, meta);
        if refresh_stored_bytes(&mut trace) <= MAX_RESULT_TRACE_BYTES {
            return trace;
        }
    }

    // Object width/key lengths are not bounded by array/string compaction.
    // Fall back explicitly to references, not an apparently complete trace.
    // The independent task summary and full journal log remain unchanged.
    let mut meta = result_trace_storage_meta(original_bytes, 0, original_hash, &stats, true);
    meta["evidence_streams"] = evidence_streams;
    meta["projection"] = json!("minimal");
    trace = json!({"step_results":[], "capability_results":[], "trace_storage":meta});
    refresh_stored_bytes(&mut trace);
    trace
}
