use super::*;
use crate::task_journal::{
    result_trace_json_with_storage_limit, trace_json_bytes, trace_json_hash, MAX_RESULT_TRACE_BYTES,
};

fn assert_bound(original: Value) -> Value {
    let expected_hash = trace_json_hash(&original);
    let expected_streams = json!({
        "step_results": trace_json_hash(&original["step_results"]),
        "capability_results": trace_json_hash(&original["capability_results"]),
    });
    let stored = result_trace_json_with_storage_limit(original);
    let bytes = trace_json_bytes(&stored);
    assert!(bytes <= MAX_RESULT_TRACE_BYTES, "stored {bytes} bytes");
    assert_eq!(stored["trace_storage"]["stored_bytes"], json!(bytes));
    assert_eq!(stored["trace_storage"]["original_hash"], expected_hash);
    if stored["trace_storage"]["truncated"] == true {
        assert_eq!(
            stored["trace_storage"]["evidence_streams"],
            expected_streams
        );
    }
    stored
}

#[test]
fn metadata_is_included_at_the_exact_byte_boundary() {
    let seed = json!({"step_results":[],"capability_results":[],"payload":""});
    let overhead = trace_json_bytes(&seed);
    for delta in [0, 1, 128, 512] {
        let mut original = seed.clone();
        original["payload"] = json!("x".repeat(MAX_RESULT_TRACE_BYTES - overhead - delta));
        let stored = assert_bound(original);
        if delta == 0 {
            assert_eq!(stored["trace_storage"]["truncated"], true);
        }
    }
}

#[test]
fn broad_nested_steps_and_escaped_strings_stay_bounded() {
    let fields: serde_json::Map<String, Value> = (0..50)
        .map(|n| (format!("field_{n}"), json!("\"\\\n".repeat(300))))
        .collect();
    let original = json!({
        "step_results": vec![json!({"output":fields.clone()}); 16],
        "capability_results": vec![json!({"data":fields}); 16],
        "evidence_policy":{"contract_match":"generic_default"},
    });
    let stored = assert_bound(original);
    assert_eq!(
        stored["evidence_policy"]["contract_match"],
        "generic_default"
    );
    assert_eq!(stored["trace_storage"]["truncated"], true);
}

#[test]
fn unbounded_object_keys_use_an_explicit_minimal_projection() {
    let fields: serde_json::Map<String, Value> = (0..3000)
        .map(|n| (format!("field_{n}_{}", "k".repeat(100)), json!(n)))
        .collect();
    let stored = assert_bound(json!({
        "step_results":[{"output":fields}], "capability_results":[],
    }));
    assert_eq!(stored["trace_storage"]["projection"], "minimal");
    assert_eq!(stored["step_results"], json!([]));
    assert_eq!(stored["capability_results"], json!([]));
}

#[test]
fn compact_preserves_artifact_locator_fields_past_string_budget() {
    let long_path = format!(
        "/home/guagua/rustclaw/.agent-runtime/artifacts/skill-invocations/{}/image_vision/{}/image_text_ai.txt",
        "1c5349fb-69b7-4013-892b-3f67a62b3f8a",
        "594655cf-7fca-4159-966d-8d68c3de9734",
    );
    assert!(long_path.chars().count() > 120);
    let digest = "6b013e4bab138d2c508a84a6fbcb4aae95ecd17eb250c73ae547e2999be04934";
    let original = json!({
        "step_results": [],
        "capability_results": [{
            "status": "ok",
            "artifacts": [{
                "path": long_path,
                "sha256": digest,
                "filename": "image_text_ai.txt",
                "preview": "x".repeat(80_000),
            }]
        }],
        "payload": "y".repeat(80_000),
    });
    let stored = assert_bound(original);
    assert_eq!(stored["trace_storage"]["truncated"], true);
    let artifact = &stored["capability_results"][0]["artifacts"][0];
    assert_eq!(
        artifact["path"].as_str().unwrap(),
        "/home/guagua/rustclaw/.agent-runtime/artifacts/skill-invocations/1c5349fb-69b7-4013-892b-3f67a62b3f8a/image_vision/594655cf-7fca-4159-966d-8d68c3de9734/image_text_ai.txt"
    );
    assert_eq!(artifact["sha256"], digest);
    assert_eq!(artifact["filename"], "image_text_ai.txt");
    assert!(artifact["preview"]
        .as_str()
        .unwrap()
        .contains("...(truncated)"));
}

#[test]
fn small_traces_keep_their_fields_and_report_exact_stored_bytes() {
    let original = json!({"step_results":[],"capability_results":[],"state":"ok"});
    let stored = assert_bound(original.clone());
    for (key, value) in original.as_object().unwrap() {
        assert_eq!(&stored[key], value);
    }
    assert_eq!(stored["trace_storage"]["truncated"], false);
}
