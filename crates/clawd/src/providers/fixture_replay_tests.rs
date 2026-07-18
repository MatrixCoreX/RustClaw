use super::*;
use std::path::Path;

/// 进程内 env 串扰隔离锁：本模块所有用 set_var 的测试串行化。
fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::fixture_replay_e2e::fixture_env_lock()
}

/// 简易 RAII tempdir（避开新增 tempfile dev-dep）：drop 时递归删除。
/// uuid v4 命名足够避开并行测试的目录撞名。
struct ScopedTempDir(std::path::PathBuf);
impl ScopedTempDir {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rustclaw_fixture_replay_{}_{}",
            label,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for ScopedTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn make_runtime() -> Arc<LlmProviderRuntime> {
    build_fixture_replay_runtime("vendor-fixture")
}

fn write_fixture(root: &Path, case: &str, lines: &[RecordedCall]) {
    let dir = root.join(case);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(FIXTURE_CALLS_FILENAME);
    let body = lines
        .iter()
        .map(|c| serde_json::to_string(c).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, body).unwrap();
}

#[test]
fn fnv1a_64_hex_is_stable_and_distinguishes_inputs() {
    let a = fnv1a_64_hex("hello");
    let b = fnv1a_64_hex("hello");
    let c = fnv1a_64_hex("hellp");
    assert_eq!(a.len(), 16);
    assert_eq!(a, b, "same input must hash same");
    assert_ne!(a, c, "different input must hash different");
    assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert_eq!(fnv1a_64_hex(""), fnv1a_64_hex(""));
}

#[tokio::test]
async fn replay_returns_recorded_response_on_hit() {
    let _g = env_guard();
    clear_cache_for_test();
    let tmp = ScopedTempDir::new("hit");
    let prompt = "what is 2 + 2";
    let hash = fnv1a_64_hex(prompt);
    let rec = RecordedCall {
        prompt_hash: hash.clone(),
        prompt_source: Some("chat_response".to_string()),
        prompt_preview: Some(prompt.to_string()),
        clean_response: "4".to_string(),
        raw_response: Some("\"4\"".to_string()),
        usage: None,
    };
    write_fixture(tmp.path(), "case_basic", &[rec]);

    std::env::set_var(FIXTURE_LLM_ROOT_ENV, tmp.path());
    std::env::set_var(FIXTURE_LLM_CASE_ENV, "case_basic");

    let runtime = make_runtime();
    let resp = FixtureReplayProvider
        .call(runtime, prompt.to_string(), ChatRequestHints::default())
        .await
        .expect("hit");
    assert_eq!(resp.text, "4");
    assert_eq!(resp.raw_response, "\"4\"");
    assert_eq!(resp.request_payload["fixture_replay"], true);
    assert_eq!(resp.request_payload["prompt_hash"], hash);

    std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
    std::env::remove_var(FIXTURE_LLM_CASE_ENV);
}

#[tokio::test]
async fn replay_fails_loud_on_miss_with_regen_hint() {
    let _g = env_guard();
    clear_cache_for_test();
    let tmp = ScopedTempDir::new("miss");
    write_fixture(tmp.path(), "case_miss", &[]);

    std::env::set_var(FIXTURE_LLM_ROOT_ENV, tmp.path());
    std::env::set_var(FIXTURE_LLM_CASE_ENV, "case_miss");

    let runtime = make_runtime();
    let err = FixtureReplayProvider
        .call(runtime, "anything".to_string(), ChatRequestHints::default())
        .await
        .expect_err("miss");
    assert!(!err.retryable, "miss must be non-retryable to fail loud");
    assert!(err.message.contains("fixture_replay miss"));
    assert!(err.message.contains("prompt_hash="));
    assert!(err.message.contains("RUSTCLAW_REGEN_FIXTURE=case_miss"));

    std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
    std::env::remove_var(FIXTURE_LLM_CASE_ENV);
}

#[tokio::test]
async fn replay_sequence_fallback_is_opt_in_and_ordered() {
    let _g = env_guard();
    clear_cache_for_test();
    let tmp = ScopedTempDir::new("sequence_fallback");
    let rec_a = RecordedCall {
        prompt_hash: fnv1a_64_hex("old prompt a"),
        prompt_source: Some("plan".to_string()),
        prompt_preview: Some("old prompt a".to_string()),
        clean_response: "response-a".to_string(),
        raw_response: None,
        usage: None,
    };
    let rec_b = RecordedCall {
        prompt_hash: fnv1a_64_hex("old prompt b"),
        prompt_source: Some("chat".to_string()),
        prompt_preview: Some("old prompt b".to_string()),
        clean_response: "response-b".to_string(),
        raw_response: None,
        usage: None,
    };
    write_fixture(tmp.path(), "case_sequence", &[rec_a, rec_b]);

    std::env::set_var(FIXTURE_LLM_ROOT_ENV, tmp.path());
    std::env::set_var(FIXTURE_LLM_CASE_ENV, "case_sequence");
    std::env::set_var(FIXTURE_LLM_SEQUENCE_FALLBACK_ENV, "1");

    let runtime = make_runtime();
    let first = FixtureReplayProvider
        .call(
            runtime.clone(),
            "new prompt a".to_string(),
            ChatRequestHints::default(),
        )
        .await
        .expect("first sequence fallback");
    let second = FixtureReplayProvider
        .call(
            runtime,
            "new prompt b".to_string(),
            ChatRequestHints::default(),
        )
        .await
        .expect("second sequence fallback");

    assert_eq!(first.text, "response-a");
    assert_eq!(second.text, "response-b");
    assert_eq!(
        first.request_payload["fixture_replay_sequence_fallback"],
        true
    );
    assert_eq!(second.request_payload["sequence_index"], 1);

    std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
    std::env::remove_var(FIXTURE_LLM_CASE_ENV);
    std::env::remove_var(FIXTURE_LLM_SEQUENCE_FALLBACK_ENV);
}

#[tokio::test]
async fn replay_fails_loud_when_env_not_set() {
    let _g = env_guard();
    clear_cache_for_test();
    std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
    std::env::remove_var(FIXTURE_LLM_CASE_ENV);

    let runtime = make_runtime();
    let err = FixtureReplayProvider
        .call(runtime, "x".to_string(), ChatRequestHints::default())
        .await
        .expect_err("env missing");
    assert!(err.message.contains("RUSTCLAW_FIXTURE_LLM_ROOT"));
}

// ---------- §7.5 Step 2.b: convert_model_io_log_to_fixture ----------

fn verbose_ok_line(prompt_hash: &str, prompt: &str, clean: &str, source: &str) -> String {
    serde_json::json!({
        "ts": 1_700_000_000u64,
        "mode": "verbose",
        "task_id": "task-1",
        "user_id": "u1",
        "chat_id": "c1",
        "vendor": "openai",
        "provider": "vendor-openai",
        "provider_type": "openai",
        "model": "gpt-test",
        "model_kind": "chat",
        "status": "ok",
        "prompt_source": source,
        "prompt_hash": prompt_hash,
        "prompt": prompt,
        "request_payload": {"foo": 1},
        "response": clean,
        "raw_response": clean,
        "clean_response": clean,
        "usage": {"prompt_tokens": 10, "completion_tokens": 4},
        "sanitized": false,
        "error": null,
    })
    .to_string()
}

#[test]
fn convert_empty_log_yields_empty_vec() {
    let recs = convert_model_io_log_to_fixture("").expect("empty ok");
    assert!(recs.is_empty());
    let recs = convert_model_io_log_to_fixture("   \n# comment\n\n").expect("comments ok");
    assert!(recs.is_empty());
}

#[test]
fn convert_picks_up_single_verbose_ok_line() {
    let prompt = "what time is it";
    let hash = fnv1a_64_hex(prompt);
    let line = verbose_ok_line(&hash, prompt, "12:00", "intent_normalizer");
    let recs = convert_model_io_log_to_fixture(&line).expect("convert ok");
    assert_eq!(recs.len(), 1);
    let r = &recs[0];
    assert_eq!(r.prompt_hash, hash);
    assert_eq!(r.clean_response, "12:00");
    assert_eq!(r.raw_response.as_deref(), Some("12:00"));
    assert_eq!(r.prompt_source.as_deref(), Some("intent_normalizer"));
    assert!(r.usage.is_some(), "usage must be parsed");
}

#[test]
fn convert_dedupes_same_hash_keeping_latest() {
    let prompt = "same prompt";
    let hash = fnv1a_64_hex(prompt);
    let body = format!(
        "{}\n{}\n",
        verbose_ok_line(&hash, prompt, "first", "chat_response"),
        verbose_ok_line(&hash, prompt, "second", "chat_response"),
    );
    let recs = convert_model_io_log_to_fixture(&body).expect("convert ok");
    assert_eq!(recs.len(), 1, "same hash must dedupe");
    assert_eq!(
        recs[0].clean_response, "second",
        "must keep the LATEST occurrence (closest to current behaviour)"
    );
}

#[test]
fn convert_skips_slim_and_failed_lines() {
    let slim = serde_json::json!({
        "ts": 1u64,
        "mode": "slim",
        "task_id": "t",
        "status": "ok",
        "prompt_source": "x",
        "prompt_chars": 10u64,
    })
    .to_string();
    let prompt = "p";
    let hash = fnv1a_64_hex(prompt);
    let mut errored =
        serde_json::from_str::<serde_json::Value>(&verbose_ok_line(&hash, prompt, "ignored", "x"))
            .unwrap();
    errored["status"] = serde_json::json!("error");
    let body = format!("{}\n{}\n", slim, errored);
    let recs = convert_model_io_log_to_fixture(&body).expect("convert ok");
    assert!(
        recs.is_empty(),
        "slim and non-ok status lines must be filtered out"
    );
}

#[test]
fn convert_fails_when_prompt_hash_missing() {
    // 模拟老版本 clawd 写出的日志（无 prompt_hash 字段）。
    let mut v = serde_json::from_str::<serde_json::Value>(&verbose_ok_line(
        "deadbeef0000beef",
        "any",
        "ok",
        "x",
    ))
    .unwrap();
    v.as_object_mut().unwrap().remove("prompt_hash");
    let line = v.to_string();
    let err = convert_model_io_log_to_fixture(&line).expect_err("must fail loud");
    assert!(err.contains("`prompt_hash`"), "err msg = {err}");
    assert!(
        err.contains("pre-§7.5"),
        "err msg should hint upgrade: {err}"
    );
}

// ---------- §7.5 Step 3: regen_fixture_from_log ----------

fn make_log_with_n_records(n: usize) -> String {
    (0..n)
        .map(|i| {
            let prompt = format!("prompt-{i}");
            let hash = fnv1a_64_hex(&prompt);
            verbose_ok_line(&hash, &prompt, &format!("clean-{i}"), "x")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn regen_writes_calls_jsonl_with_correct_records() {
    let tmp = ScopedTempDir::new("regen_basic");
    let log = make_log_with_n_records(3);
    let summary =
        regen_fixture_from_log(&log, "case_a", tmp.path(), false, false).expect("regen ok");
    assert_eq!(summary.written_records, 3);
    assert!(!summary.dry_run);
    assert!(!summary.overwrote_existing);
    assert_eq!(
        summary.dest_path,
        tmp.path().join("case_a").join(FIXTURE_CALLS_FILENAME)
    );
    let body = std::fs::read_to_string(&summary.dest_path).expect("read written file");
    let lines: Vec<_> = body.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 3, "must write exactly 3 JSONL lines");
    for line in &lines {
        let _: RecordedCall = serde_json::from_str(line).expect("each line parses back");
    }
}

#[test]
fn regen_dry_run_does_not_touch_disk() {
    let tmp = ScopedTempDir::new("regen_dry");
    let log = make_log_with_n_records(2);
    let summary =
        regen_fixture_from_log(&log, "case_dry", tmp.path(), true, false).expect("dry-run ok");
    assert_eq!(summary.written_records, 2);
    assert!(summary.dry_run);
    assert!(
        !summary.dest_path.exists(),
        "dry-run must not create dest file"
    );
    assert!(
        !summary.dest_path.parent().unwrap().exists(),
        "dry-run must not even mkdir the case dir"
    );
}

#[test]
fn regen_refuses_overwrite_without_force() {
    let tmp = ScopedTempDir::new("regen_no_force");
    let log = make_log_with_n_records(1);
    regen_fixture_from_log(&log, "case_x", tmp.path(), false, false).expect("first write ok");
    let err = regen_fixture_from_log(&log, "case_x", tmp.path(), false, false)
        .expect_err("must refuse overwrite without force");
    assert!(err.contains("already exists"), "err = {err}");
    assert!(
        err.contains("RUSTCLAW_REGEN_FIXTURE_FORCE"),
        "err must hint at the force env: {err}"
    );
}

#[test]
fn regen_with_force_overwrites_and_marks_summary() {
    let tmp = ScopedTempDir::new("regen_force");
    let log_a = make_log_with_n_records(2);
    let log_b = make_log_with_n_records(5);
    let s1 = regen_fixture_from_log(&log_a, "case_y", tmp.path(), false, false).expect("first");
    assert!(!s1.overwrote_existing);
    let s2 = regen_fixture_from_log(&log_b, "case_y", tmp.path(), false, true)
        .expect("force overwrite ok");
    assert_eq!(s2.written_records, 5);
    assert!(s2.overwrote_existing);
    let body = std::fs::read_to_string(&s2.dest_path).unwrap();
    assert_eq!(body.lines().filter(|l| !l.is_empty()).count(), 5);
}

#[test]
fn regen_fails_loud_on_empty_records() {
    let tmp = ScopedTempDir::new("regen_empty");
    let err = regen_fixture_from_log("", "case_empty", tmp.path(), false, false)
        .expect_err("empty must fail");
    assert!(err.contains("0 records"), "err = {err}");
    let only_slim = serde_json::json!({
        "ts": 1u64, "mode": "slim", "task_id": "t", "status": "ok",
        "prompt_source": "x", "prompt_chars": 5u64,
    })
    .to_string();
    let err = regen_fixture_from_log(&only_slim, "case_empty", tmp.path(), false, false)
        .expect_err("only slim must fail");
    assert!(err.contains("0 records"), "err = {err}");
}

#[test]
fn regen_rejects_unsafe_case_names() {
    let tmp = ScopedTempDir::new("regen_unsafe");
    let log = make_log_with_n_records(1);
    for bad in &["", "  ", "../escape", "a/b", "..", r"a\b"] {
        let err = regen_fixture_from_log(&log, bad, tmp.path(), false, false)
            .expect_err(&format!("case name {bad:?} must be rejected"));
        assert!(
            err.contains("case name") || err.contains("non-empty"),
            "case={bad:?} err={err}"
        );
    }
}

// ---------- back to convert_* edge cases ----------

/// §7.5 抬阈值后回归保护：模型 io 日志阈值不能再回到 16K。
/// 16K 会让 normalizer prompt（典型 15~30 KB）被截断，prompt_hash 虽然落了
/// 但 prompt_preview 看不全，长 response 也直接拒录。
#[test]
fn model_io_log_max_chars_must_stay_at_or_above_128k() {
    assert!(
        crate::MODEL_IO_LOG_MAX_CHARS >= 128_000,
        "MODEL_IO_LOG_MAX_CHARS regressed below 128K (now {}); \
         fixture recording for normal-sized cases will start hitting the truncated guard",
        crate::MODEL_IO_LOG_MAX_CHARS
    );
}

#[test]
fn convert_fails_when_response_was_truncated() {
    let prompt = "p";
    let hash = fnv1a_64_hex(prompt);
    let mut v =
        serde_json::from_str::<serde_json::Value>(&verbose_ok_line(&hash, prompt, "ok-base", "x"))
            .unwrap();
    v["clean_response"] = serde_json::json!("some-long-response...(truncated)");
    let err = convert_model_io_log_to_fixture(&v.to_string()).expect_err("must fail loud");
    assert!(err.contains("truncated"), "err = {err}");
}

#[tokio::test]
async fn replay_skips_malformed_lines_but_keeps_good_ones() {
    let _g = env_guard();
    clear_cache_for_test();
    let tmp = ScopedTempDir::new("partial");
    let case = "case_partial";
    let dir = tmp.path().join(case);
    std::fs::create_dir_all(&dir).unwrap();

    let good_prompt = "ping";
    let good_hash = fnv1a_64_hex(good_prompt);
    let good = serde_json::to_string(&RecordedCall {
        prompt_hash: good_hash.clone(),
        prompt_source: None,
        prompt_preview: None,
        clean_response: "pong".to_string(),
        raw_response: None,
        usage: None,
    })
    .unwrap();

    let body = format!(
        "# this is a comment line, ignored\n\
         {good}\n\
         this-is-not-json\n\
         \n"
    );
    std::fs::write(dir.join(FIXTURE_CALLS_FILENAME), body).unwrap();

    std::env::set_var(FIXTURE_LLM_ROOT_ENV, tmp.path());
    std::env::set_var(FIXTURE_LLM_CASE_ENV, case);

    let runtime = make_runtime();
    let resp = FixtureReplayProvider
        .call(
            runtime,
            good_prompt.to_string(),
            ChatRequestHints::default(),
        )
        .await
        .expect("good line should still load");
    assert_eq!(resp.text, "pong");
    assert_eq!(
        resp.raw_response, "pong",
        "raw fallback to clean when absent"
    );

    std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
    std::env::remove_var(FIXTURE_LLM_CASE_ENV);
}
