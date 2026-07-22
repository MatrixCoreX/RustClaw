use super::*;

#[test]
fn raw_response_sanitizer_preserves_visible_fields_without_hidden_reasoning() {
    let raw = json!({
        "choices": [{
            "message": {
                "content": "<think>private reasoning</think>{\"pass\":true}",
                "reasoning_content": "private field"
            }
        }],
        "usage": {"total_tokens": 10}
    })
    .to_string();

    let (safe, changed) = sanitize_provider_raw_response(&raw);
    let value: Value = serde_json::from_str(&safe).expect("safe JSON");

    assert!(changed);
    assert_eq!(
        value.pointer("/choices/0/message/content"),
        Some(&Value::String("{\"pass\":true}".to_string()))
    );
    assert!(value
        .pointer("/choices/0/message/reasoning_content")
        .is_none());
    assert_eq!(value.pointer("/usage/total_tokens"), Some(&json!(10)));
}

#[test]
fn raw_response_sanitizer_handles_json_lines() {
    let raw = [
        json!({"choices": [{"delta": {"content": "<think>hidden</think>visible"}}]}).to_string(),
        json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}).to_string(),
    ]
    .join("\n");

    let (safe, changed) = sanitize_provider_raw_response(&raw);

    assert!(changed);
    assert!(!safe.contains("hidden"));
    assert!(safe.contains("visible"));
    assert!(safe.contains("finish_reason"));
}

#[test]
fn model_io_rotation_does_not_drop_concurrent_appends() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-model-io-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let path = root.join("model_io.log");
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        for logical_call_index in 1..=200_u64 {
            let line = json!({
                "ts": now_ts_u64(),
                "task_id": "concurrent-task",
                "logical_call_index": logical_call_index,
            })
            .to_string();
            append_model_io_line(&writer_path, &line).expect("append model io line");
            std::thread::yield_now();
        }
    });

    for _ in 0..40 {
        rotate_model_io_log_daily(&path, MODEL_IO_LOG_KEEP_DAYS).expect("rotate model io log");
        std::thread::yield_now();
    }
    writer.join().expect("join model io writer");
    rotate_model_io_log_daily(&path, MODEL_IO_LOG_KEEP_DAYS).expect("final rotation");

    let raw = std::fs::read_to_string(&path).expect("read model io log");
    let mut indexes = std::collections::BTreeSet::new();
    for line in raw.lines() {
        let row: Value = serde_json::from_str(line).expect("valid model io row");
        indexes.insert(
            row.get("logical_call_index")
                .and_then(Value::as_u64)
                .expect("logical call index"),
        );
    }
    assert_eq!(indexes.len(), 200);
    assert_eq!(indexes.first().copied(), Some(1));
    assert_eq!(indexes.last().copied(), Some(200));
    std::fs::remove_dir_all(root).expect("remove temp root");
}
