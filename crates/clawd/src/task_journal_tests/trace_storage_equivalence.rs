use super::*;
use crate::task_journal::{
    compact_result_trace_value, result_trace_json_with_storage_limit, stable_trace_hash,
    trace_json_bytes, trace_json_hash, TraceStorageStats, MAX_RESULT_TRACE_ARRAY_ITEMS,
    MAX_RESULT_TRACE_BYTES, MAX_RESULT_TRACE_COMPACT_ARRAY_ITEMS,
    MAX_RESULT_TRACE_COMPACT_STRING_CHARS, MAX_RESULT_TRACE_STRING_CHARS,
};

#[path = "trace_storage_benchmark.rs"]
mod benchmark;
#[path = "trace_storage_reference_tests.rs"]
mod reference;

fn value_fixture(seed: usize, depth: usize) -> Value {
    match if depth == 0 { seed % 5 } else { seed % 7 } {
        0 => Value::Null,
        1 => json!(seed % 2 == 0),
        2 => json!((seed as i64).wrapping_mul(-7001)),
        3 => json!(seed as f64 / 17.0),
        4 => json!(format!("{seed}:\"\\\n\t\0中文🙂").repeat(seed % 17)),
        5 => Value::Array(
            (0..seed % 7)
                .map(|n| value_fixture(seed / 2 + n + 1, depth - 1))
                .collect(),
        ),
        _ => Value::Object(
            (0..seed % 5)
                .map(|n| {
                    (
                        format!("字段_{n}"),
                        value_fixture(seed / 3 + n + 1, depth - 1),
                    )
                })
                .collect(),
        ),
    }
}

fn artifact_fixture(kind: usize, index: usize) -> Value {
    let artifact = json!({"path":format!("/workspace/asset-{index}.txt"),"filename":format!("asset-{index}.txt"),"sha256":"fixture-digest"});
    let mut value = match kind % 10 {
        0 => json!({"status":"ok","data":{"value":index}}),
        1 => json!({"status":"ok","artifacts":[artifact]}),
        2 => json!({"status":"ok","data":{"extra":{"artifacts":[artifact]}}}),
        3 => json!({"status":"ok","extra":{"artifacts":[artifact]}}),
        4 => json!({"status":"ok","data":{"artifacts":[artifact]}}),
        5 => json!({"status":"error","artifacts":[artifact]}),
        6 => json!({"status":"ok","delivery":{"deliver_to_user":false},"artifacts":[artifact]}),
        7 => {
            json!({"status":"ok","artifacts":[{"path":"/workspace/private.txt","visibility":"internal_processing"}]})
        }
        8 => json!({"status":"ok","artifacts":[{"filename":"proof.txt","visibility":"evidence"}]}),
        _ => json!({"artifacts":[{"filename":"visible.txt"}]}),
    };
    value["fixture_index"] = json!(index);
    value
}

fn assert_projection_equivalent(original: Value) {
    let serialized = serde_json::to_vec(&original).unwrap();
    let expected = reference::result_trace_json_with_storage_limit(original.clone());
    let actual = result_trace_json_with_storage_limit(original);
    assert_eq!(actual, expected);
    let bytes = serde_json::to_vec(&actual).unwrap().len();
    assert!(bytes <= MAX_RESULT_TRACE_BYTES);
    assert_eq!(actual["trace_storage"]["stored_bytes"], json!(bytes));
    assert_eq!(
        actual["trace_storage"]["original_bytes"],
        json!(serialized.len())
    );
    assert_eq!(
        actual["trace_storage"]["original_hash"],
        reference::stable_trace_hash(std::str::from_utf8(&serialized).unwrap())
    );
}

#[test]
fn serialized_length_and_hash_match_reference_for_json_values() {
    for seed in 0..512 {
        let value = value_fixture(seed, 4);
        assert_eq!(
            trace_json_bytes(&value),
            serde_json::to_vec(&value).unwrap().len()
        );
        assert_eq!(trace_json_hash(&value), reference::trace_json_hash(&value));
    }
    for value in [
        json!(u64::MAX),
        json!(i64::MIN),
        json!(-0.0),
        json!(1.25e99),
        json!({"control":"\r\n\t\0","unicode":"é中文🙂"}),
    ] {
        assert_eq!(
            trace_json_bytes(&value),
            reference::trace_json_bytes(&value)
        );
        assert_eq!(trace_json_hash(&value), reference::trace_json_hash(&value));
    }
    assert_eq!(stable_trace_hash(""), "fnv64:cbf29ce484222325");
    assert_eq!(stable_trace_hash("hello"), "fnv64:a430d84680aabd0b");
}

#[test]
fn compaction_keeps_reference_artifact_order_and_statistics() {
    for count in [0, 1, 2, 7, 24, 25, 64, 129, 300] {
        for pattern in 0..12 {
            let items: Vec<_> = (0..count)
                .map(|n| artifact_fixture(if pattern < 10 { pattern } else { n + pattern }, n))
                .collect();
            for limit in [0, 1, 2, 4, 8, 24, 64, 128] {
                let mut actual = json!({"capability_results":items});
                let mut expected = actual.clone();
                let mut actual_stats = TraceStorageStats::default();
                let mut expected_stats = reference::TraceStorageStats::default();
                compact_result_trace_value(&mut actual, &mut actual_stats, limit, 768);
                reference::compact_result_trace_value(
                    &mut expected,
                    &mut expected_stats,
                    limit,
                    768,
                );
                assert_eq!(
                    actual, expected,
                    "count={count} pattern={pattern} limit={limit}"
                );
                assert_eq!(
                    (
                        actual_stats.truncated_arrays,
                        actual_stats.omitted_array_items,
                        actual_stats.truncated_strings
                    ),
                    (
                        expected_stats.truncated_arrays,
                        expected_stats.omitted_array_items,
                        expected_stats.truncated_strings
                    )
                );
            }
        }
    }
}

#[test]
fn complete_projection_matches_reference_across_byte_boundaries() {
    for size in [
        0,
        30,
        768,
        MAX_RESULT_TRACE_BYTES - 600,
        MAX_RESULT_TRACE_BYTES - 1,
        MAX_RESULT_TRACE_BYTES,
        MAX_RESULT_TRACE_BYTES + 1,
        MAX_RESULT_TRACE_BYTES * 2,
    ] {
        assert_projection_equivalent(
            json!({"step_results":[{"step_id":"step_1","data":value_fixture(6,3)}],"capability_results":[artifact_fixture(1,0)],"payload":"x".repeat(size)}),
        );
    }
}

#[test]
fn nested_unicode_locators_and_existing_metadata_match_reference() {
    for seed in 0..32 {
        assert_projection_equivalent(json!({
            "step_results":(0..seed).map(|n| json!({"path":format!("/workspace/{}/file-{n}", "路径".repeat(seed + 1)), "data":value_fixture(n + seed,4)})).collect::<Vec<_>>(),
            "capability_results":(0..64).map(|n| artifact_fixture(n + seed,n)).collect::<Vec<_>>(),
            "trace_storage":{"payload":"previous metadata","stored_bytes":u64::MAX},
            "payload":"\"\\\n中文🙂".repeat(seed * 500),
        }));
    }
}

#[test]
fn wide_objects_keep_identical_minimal_fallback_and_evidence_hashes() {
    let fields: serde_json::Map<String, Value> = (0..1800)
        .map(|n| (format!("field_{n}_{}", "k".repeat(100)), json!(n)))
        .collect();
    let original =
        json!({"step_results":[{"data":fields}],"capability_results":[artifact_fixture(2,1)]});
    let expected_stream = reference::trace_json_hash(&original["step_results"]);
    let stored = result_trace_json_with_storage_limit(original.clone());
    assert_eq!(stored["trace_storage"]["projection"], "minimal");
    assert_eq!(
        stored["trace_storage"]["evidence_streams"]["step_results"],
        expected_stream
    );
    assert_projection_equivalent(original);
}
