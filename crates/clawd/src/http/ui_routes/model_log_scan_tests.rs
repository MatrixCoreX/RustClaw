use super::*;

fn meta() -> UsageTaskMeta {
    UsageTaskMeta {
        channel: "ui".into(),
        kind: "ask".into(),
        task_status: "succeeded".into(),
        user_key: Some("owner-key".into()),
        external_user_id: None,
        external_chat_id: None,
        telegram_bot_name: None,
        request_text: Some("fixture request".into()),
    }
}

fn entry(value: Value) -> UsageLogEntry {
    serde_json::from_value(value).unwrap()
}

#[test]
fn usage_metadata_does_not_decode_or_retain_request_and_response_bodies() {
    let value = json!({"task_id":"task-a","ts":3,"status":"ok","model":"fixture-model",
        "usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16},
        "request_payload":{"model_log_encoding":"gzip-base64-v1","data":"corrupt","uncompressed_bytes":16,"sha256":"bad"},
        "prompt":{"wrong_type_for_full_entry":true}, "raw_response":"not metadata",
        "response":"not metadata", "clean_response":"not metadata"});
    assert!(serde_json::from_value::<TaskDebugEntry>(value.clone()).is_err());
    let mut summary = UsageTaskAccumulator::default();
    summary.add(entry(value));
    let summary = summary.into_summary("task-a".into(), meta());
    assert_eq!(summary.llm_call_count, 1);
    assert_eq!(summary.total_tokens, Some(16));
    assert_eq!(summary.model.as_deref(), Some("fixture-model"));
    assert!(!format!("{summary:?}").contains("not metadata"));
}

#[test]
fn streaming_summary_preserves_timestamp_ties_fallback_tokens_and_latest_status() {
    let values = [
        json!({"task_id":"task-a","ts":20,"status":"ok","model":"first","usage":{"input_tokens":5,"output_tokens":2}}),
        json!({"task_id":"task-a","ts":10,"status":"ok","model":"older","usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":9}}),
        json!({"task_id":"task-a","ts":20,"status":"error","model":"latest","prompt_file":"fixture.md","error":"fixture_error","usage":{"prompt_tokens":0,"input_tokens":50,"output_tokens":4}}),
    ];
    let mut accumulator = UsageTaskAccumulator::default();
    for value in &values {
        accumulator.add(entry(value.clone()));
    }
    let summary = accumulator.into_summary("task-a".into(), meta());
    assert_eq!(summary.ts, Some(20));
    assert_eq!(summary.model.as_deref(), Some("latest"));
    assert_eq!(summary.status.as_deref(), Some("error"));
    assert_eq!(summary.error.as_deref(), Some("fixture_error"));
    assert_eq!(summary.prompt_file.as_deref(), Some("fixture.md"));
    assert_eq!(summary.prompt_tokens, Some(8));
    assert_eq!(summary.completion_tokens, Some(7));
    assert_eq!(summary.total_tokens, Some(20));
    assert_eq!(summary.llm_call_count, 3);
    let decoded = values
        .into_iter()
        .map(|v| serde_json::from_value::<TaskDebugEntry>(v).unwrap())
        .collect::<Vec<_>>();
    let detailed = summarize_usage_task("task-a".into(), meta(), &decoded);
    assert_eq!(
        serde_json::to_value(summary).unwrap(),
        serde_json::to_value(detailed).unwrap()
    );
}

#[test]
fn missing_usage_empty_tasks_and_extreme_counters_are_safe() {
    let empty = UsageTaskAccumulator::default().into_summary("task-a".into(), meta());
    assert_eq!(empty.llm_call_count, 0);
    assert_eq!(empty.total_tokens, Some(0));
    assert!(empty.ts.is_none());
    let mut accumulator = UsageTaskAccumulator::default();
    accumulator.add(entry(json!({"ts":1,"status":"ok"})));
    for _ in 0..2 {
        accumulator.add(entry(
            json!({"ts":2,"status":"ok","usage":{"prompt_tokens":u64::MAX,"completion_tokens":1}}),
        ));
    }
    let summary = accumulator.into_summary("task-a".into(), meta());
    assert_eq!(summary.prompt_tokens, Some(u64::MAX));
    assert_eq!(summary.completion_tokens, Some(2));
    assert_eq!(summary.total_tokens, Some(u64::MAX));
    let mut stats = UsageHistoryStats {
        total_requests: usize::MAX,
        success_requests: usize::MAX,
        failed_requests: 0,
        prompt_tokens: u64::MAX,
        completion_tokens: 0,
        total_tokens: u64::MAX,
    };
    usage_stats_add(&mut stats, &summary);
    usage_stats_add(&mut stats, &summary);
    assert_eq!(stats.total_requests, usize::MAX);
    assert_eq!(stats.success_requests, usize::MAX);
    assert_eq!(stats.total_tokens, u64::MAX);
}

#[test]
fn task_selector_is_exact_and_accepts_json_escapes_without_decoding_payloads() {
    let ids = std::collections::HashSet::from(["task-a".into(), "child-a".into()]);
    for line in [
        r#"{"task_id":"task-a","request_payload":{"model_log_encoding":"unsupported"}}"#,
        r#"{"task_id":"ta\u0073k-a","response":"body"}"#,
        r#"{"task_id":"child-a"}"#,
    ] {
        assert!(line_matches_task_ids(line, &ids), "{line}");
    }
    for line in [
        r#"{"task_id":"other","prompt":"mentions task-a"}"#,
        r#"{"task_id":"task-a-extra"}"#,
        r#"{"task_id":null}"#,
        r#"{"call_id":"task-a"}"#,
        r#"{"task_id":"task-a","task_id":"other"}"#,
        "not JSON",
    ] {
        assert!(!line_matches_task_ids(line, &ids), "{line}");
    }
}

#[test]
fn metadata_projection_profile_keeps_summary_independent_of_large_body_size() {
    let payload =
        json!({"messages":[{"role":"system","content":"fixture context ".repeat(12000)}]});
    let raw = json!({"task_id":"task-a","ts":1,"status":"ok","usage":{"prompt_tokens":4},
        "request_payload":crate::providers::model_log_payload::encode(&payload)})
    .to_string();
    let count = 64;
    let start = std::time::Instant::now();
    for _ in 0..count {
        std::hint::black_box(serde_json::from_str::<TaskDebugEntry>(&raw).unwrap());
    }
    let full_us = start.elapsed().as_micros();
    let start = std::time::Instant::now();
    let mut summary = UsageTaskAccumulator::default();
    for _ in 0..count {
        summary.add(serde_json::from_str::<UsageLogEntry>(&raw).unwrap());
    }
    let metadata_us = start.elapsed().as_micros();
    assert_eq!(
        summary.into_summary("task-a".into(), meta()).llm_call_count,
        count
    );
    println!("MODEL_LOG_SCAN_PROFILE records={count} decoded_payload_bytes={} full_us={full_us} metadata_us={metadata_us}", payload.to_string().len());
}

struct LogDirectory(PathBuf);
impl LogDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("model-log-scan-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for LogDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn teaching_filter_preserves_matching_validation_offsets_children_and_archives() {
    let dir = LogDirectory::new();
    let payload = json!({"messages":"fixture ".repeat(1000)});
    let valid = crate::providers::model_log_payload::encode(&payload);
    let invalid = json!({"model_log_encoding":"gzip-base64-v1","data":"invalid","uncompressed_bytes":1,"sha256":"bad"});
    let archive = dir.0.join("model_io.log.2026-10-26");
    let current = dir.0.join("model_io.log");
    let records = [
        json!({"task_id":"other","request_payload":invalid.clone()}),
        json!({"task_id":"parent","call_id":"first","request_payload":valid.clone()}),
        json!({"task_id":"parent","call_id":"invalid","request_payload":invalid}),
        json!({"task_id":"child","call_id":"second","request_payload":valid.clone()}),
        json!({"task_id":"parent-extra","call_id":"not-a-child","request_payload":valid.clone()}),
        json!({"task_id":"parent","call_id":"third","request_payload":valid}),
    ];
    fs::write(
        &archive,
        records[..3]
            .iter()
            .map(|r| format!("{r}\n"))
            .collect::<String>(),
    )
    .unwrap();
    fs::write(
        &current,
        records[3..]
            .iter()
            .map(|r| format!("{r}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let ids = std::collections::HashSet::from(["parent".into(), "child".into()]);
    let page =
        read_task_debug_entries_page_from_paths(&ids, &[archive.clone(), current.clone()], 1, 1)
            .unwrap();
    assert_eq!(
        page.total, 3,
        "corrupt matching payload remains excluded from detail totals"
    );
    assert!(page.has_more);
    assert_eq!(page.entries[0].call_id.as_deref(), Some("second"));
    assert_eq!(page.entries[0].request_payload.as_ref(), Some(&payload));
    let rest = read_task_debug_entries_page_from_paths(&ids, &[archive, current], 10, 2).unwrap();
    assert_eq!(rest.total, 3);
    assert_eq!(rest.entries.len(), 1);
    assert_eq!(rest.entries[0].call_id.as_deref(), Some("third"));
    assert!(!rest.has_more);
}

#[tokio::test]
async fn usage_handler_keeps_owner_filters_paging_and_metadata_counts_without_payloads() {
    let dir = LogDirectory::new();
    fs::create_dir_all(dir.0.join("logs")).unwrap();
    let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.skill_rt.workspace_root = dir.0.clone();
    state.seed_test_auth_identity("usage-owner", "user");
    state.seed_test_auth_identity("usage-other", "user");
    let db = state.core.db.get().unwrap();
    let mut raw = String::new();
    for i in 0..14 {
        let task = format!("usage-task-{i:02}");
        let owner = if i < 12 { "usage-owner" } else { "usage-other" };
        db.execute("INSERT INTO tasks(task_id,user_id,chat_id,user_key,channel,kind,payload_json,status,created_at,updated_at) VALUES (?1,1,2,?2,'ui','ask','{}','succeeded','1','1')",rusqlite::params![task,owner]).unwrap();
        let row = json!({"task_id":task,"ts":i,"model":"fixture-model","status":"ok",
            "request_payload":{"model_log_encoding":"unsupported","data":"unavailable"},
            "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
        raw.push_str(&format!("{row}\n"));
    }
    drop(db);
    fs::write(dir.0.join("logs/model_io.log"), raw).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-agent-key",
        axum::http::HeaderValue::from_static("usage-owner"),
    );
    let query = |page, search| {
        serde_json::from_value(json!({"page":page,"page_size":10,"search":search})).unwrap()
    };
    let (status, Json(response)) =
        usage_records(State(state.clone()), headers.clone(), Query(query(2, ""))).await;
    assert_eq!(status, StatusCode::OK);
    let data = response.data.unwrap();
    assert_eq!(data["pagination"]["total_records"], 12);
    assert_eq!(data["records"].as_array().unwrap().len(), 2);
    assert_eq!(data["records"][0]["task_id"], "usage-task-01");
    assert_eq!(data["stats"]["total_tokens"], 60);
    assert!(!data.to_string().contains("usage-task-12"));
    let (_, Json(response)) = usage_records(
        State(state.clone()),
        headers,
        Query(query(1, "usage-task-05")),
    )
    .await;
    assert_eq!(response.data.unwrap()["pagination"]["total_records"], 1);
    let (status, _) = usage_records(State(state), HeaderMap::new(), Query(query(1, ""))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
