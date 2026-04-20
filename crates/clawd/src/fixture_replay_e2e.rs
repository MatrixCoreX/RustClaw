//! Phase 7 §7.5 Step 2.a/2.b：fixture-replay 端到端 harness。
//!
//! 与 [`crate::providers::fixture_replay`] 单测的区别：
//!   * 单测只覆盖 provider 内部（hash / 缓存 / miss / malformed line）以及
//!     `convert_model_io_log_to_fixture` 转换器逻辑。
//!   * 本模块覆盖 **整条 wiring 链**：
//!       1. `FIXTURE_LLM_ROOT` / `FIXTURE_LLM_CASE` env 切换；
//!       2. `RUSTCLAW_TEST_FREEZE_NOW` 让 [`crate::schedule_service`] 注入的
//!          normalizer prompt `__NOW__` 字段稳定；
//!       3. 通过 [`crate::providers::client::PROVIDER_IMPLS`] 这条生产分发表
//!          找到 `fixture_replay` provider 并真的调起来；
//!       4. RAII guard 保证：测试 panic / 提前 return 都能把上面三条 env 还原，
//!          下一条测试不会"幽灵命中"上一条 case；
//!       5. **"日志 → fixture → 回放"** 闭环：把一条合成的 `model_io.log` 喂给
//!          `convert_model_io_log_to_fixture`，写出 `calls.jsonl`，再通过生产
//!          dispatch table 调起来命中。这是整层除了"真 LLM 录制"以外的全部环节
//!          的 self-check —— 任何一环坏了，下面的真 case e2e 都没必要跑。
//!
//! ## 录制 → 回放 工作流
//!
//! Step 2.b 落地后，新 case 上线流程如下（以 `act_find_service_file` 为例）：
//!
//! 1. **真录一次**：在本地配好真 LLM key，把 `[routing] debug_log_prompt = true`
//!    打开，跑一次目标 case（例如通过 telegram / `scripts/nl_tests/run_manual_test.sh`
//!    触发 ask 请求 "rustclaw 有 service 文件吗"）。
//! 2. **抓日志**：定位到 workspace 下的 `logs/model_io.log`，把对应任务的所有
//!    verbose 行 grep 出来（按 `task_id` 过滤），存成临时文件
//!    `/tmp/<case>.model_io.log`。
//! 3. **转换**：在测试代码或一次性脚本里调
//!    [`crate::providers::fixture_replay::convert_model_io_log_to_fixture`]，把
//!    返回的 `Vec<RecordedCall>` 序列化成 JSONL，写到
//!    `crates/clawd/tests/fixtures/llm_io/<case>/calls.jsonl`。
//! 4. **commit fixture**：fixture 文件进 git，与生产代码一起评审 / 回滚。
//! 5. **unignore harness**：把对应 case 的 `#[ignore]` 标记去掉，跑
//!    `cargo test fixture_replay_e2e::tests::e2e_<case>` 验证回放命中。
//!
//! ⚠️ **录制时必须 `RUSTCLAW_TEST_FREEZE_NOW` 已 set 且 `freeze_now` 实际值与
//! 回放时完全一致**，否则 normalizer prompt 里的 `__NOW__` 会让 hash 漂；具体
//! 见 [`crate::schedule_service::TEST_FREEZE_NOW_ENV`] doc。

use std::path::PathBuf;

use crate::providers::fixture_replay::{
    FIXTURE_LLM_CASE_ENV, FIXTURE_LLM_ROOT_ENV, FIXTURE_REPLAY_PROVIDER_TYPE,
};
use crate::schedule_service::TEST_FREEZE_NOW_ENV;

/// fixture 仓内根目录：`crates/clawd/tests/fixtures/llm_io/`。
///
/// 用 `CARGO_MANIFEST_DIR` 解析，与 `cargo test` 的 cwd / IDE 无关。Step 2.b
/// 录制完每条 case 后，`<root>/<case>/calls.jsonl` 进 git。
pub(crate) fn fixture_workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("llm_io")
}

/// 进程级 env guard：构造时 set 三条 env，drop 时清掉。
///
/// `Drop` impl 即使被 panic unwind 触发也会执行（panic = stack unwind 路径），
/// 所以 self-check / e2e 测试不会因为一个失败 case 把 env 残留给下一条。
///
/// **不是线程安全**——所有用到本 guard 的测试必须共享 [`fixture_env_lock`] 序列化。
pub(crate) struct FixtureEnvGuard;

impl FixtureEnvGuard {
    /// `freeze_now` 接受 [`TEST_FREEZE_NOW_ENV`] 已支持的两种格式（RFC-3339
    /// 或 `%Y-%m-%d %H:%M:%S %:z`）。每个 case 录制时记录的"now"必须与 replay
    /// 时这里 set 的值完全一致，否则 normalizer prompt hash 会漂。
    pub(crate) fn install(root: &std::path::Path, case: &str, freeze_now: &str) -> Self {
        std::env::set_var(FIXTURE_LLM_ROOT_ENV, root);
        std::env::set_var(FIXTURE_LLM_CASE_ENV, case);
        std::env::set_var(TEST_FREEZE_NOW_ENV, freeze_now);
        Self
    }
}

impl Drop for FixtureEnvGuard {
    fn drop(&mut self) {
        std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
        std::env::remove_var(FIXTURE_LLM_CASE_ENV);
        std::env::remove_var(TEST_FREEZE_NOW_ENV);
    }
}

/// 全局串行锁：所有 fixture e2e 测试共享，保证三条 env 不会在并行测试间撕裂。
///
/// 锁中毒（上一条测试 panic）时返回 inner —— 我们只关心 env 互斥，不关心数据。
pub(crate) fn fixture_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// §7.5 Step 4.a：一个已录 case 的内存视图 —— 路径 + 全部 [`RecordedCall`]。
///
/// `case`：fixture root 下的子目录名。约定子目录名以**字母数字**开头：`_`
/// 起头的目录视为"内部 / 临时"（self-check / regen smoke 等），扫描器主动跳过。
pub(crate) struct LoadedCase {
    pub case: String,
    pub calls_path: std::path::PathBuf,
    pub records: Vec<crate::providers::fixture_replay::RecordedCall>,
}

/// §7.5 Step 4.a：扫描 fixture root 下所有"真 case"目录（跳过 `_*`），把每个
/// case 的 `calls.jsonl` 整体读进内存。用在批量 smoke harness 与未来
/// process_ask_task 端到端 harness 里，避免散落的 fs 路径拼接。
///
/// 失败语义：fixture root 不存在 → 返回空 Vec（用户还没录任何 case 是合法
/// 状态）；某个 case 的 calls.jsonl 解析挂了 → `Err`（fixture 数据坏不能装作没事）。
pub(crate) fn load_recorded_cases() -> Result<Vec<LoadedCase>, String> {
    use crate::providers::fixture_replay::{RecordedCall, FIXTURE_CALLS_FILENAME};
    let root = fixture_workspace_root();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&root)
        .map_err(|e| format!("read fixture root {} failed: {e}", root.display()))?;
    let mut sorted_dirs: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    sorted_dirs.sort();
    for case_dir in sorted_dirs {
        let case_name = match case_dir.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if case_name.starts_with('_') || case_name.starts_with('.') {
            continue;
        }
        let calls_path = case_dir.join(FIXTURE_CALLS_FILENAME);
        if !calls_path.exists() {
            // case 目录里没有 calls.jsonl —— 视为"目录占位但还没录"，跳过而不报错。
            continue;
        }
        let body = std::fs::read_to_string(&calls_path)
            .map_err(|e| format!("read {} failed: {e}", calls_path.display()))?;
        let mut records = Vec::new();
        for (line_idx, line) in body.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let rec: RecordedCall = serde_json::from_str(trimmed).map_err(|e| {
                format!(
                    "{}:{} parse RecordedCall failed: {e}",
                    calls_path.display(),
                    line_idx + 1
                )
            })?;
            records.push(rec);
        }
        out.push(LoadedCase {
            case: case_name,
            calls_path,
            records,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::client::{ChatRequestHints, PROVIDER_IMPLS};
    use crate::providers::fixture_replay::{
        build_fixture_replay_runtime, clear_cache_for_test, convert_model_io_log_to_fixture,
        fnv1a_64_hex, regen_fixture_from_log, RecordedCall, FIXTURE_CALLS_FILENAME,
    };

    /// Step 2.a 必须验证：
    ///   1. `fixture_workspace_root()` 解出来的路径形如 `.../crates/clawd/tests/fixtures/llm_io`；
    ///   2. `FixtureEnvGuard` 构造后三条 env 都 set 了；
    ///   3. `PROVIDER_IMPLS` 这张生产分发表里能找到 `fixture_replay`，并且对一条
    ///      录制好的 prompt 命中后返回 `clean_response`；
    ///   4. guard drop 后三条 env 都被清干净，下一条测试不会幽灵命中。
    ///
    /// 这是 Step 2.b 真接 `process_ask_task` 之前的"管线 self-check"——任何一环
    /// 寄了，下面就不必再调 process_ask_task。
    #[tokio::test]
    async fn step2a_self_check_e2e_wiring_through_provider_impls() {
        let _guard = fixture_env_lock();
        clear_cache_for_test();

        let root = fixture_workspace_root().join("__self_check");
        let case = "wiring";
        let case_dir = root.join(case);
        std::fs::create_dir_all(&case_dir).expect("mkdir self_check case");

        let prompt = "self-check ping";
        let hash = fnv1a_64_hex(prompt);
        let rec = RecordedCall {
            prompt_hash: hash.clone(),
            prompt_source: Some("self_check".to_string()),
            prompt_preview: Some(prompt.to_string()),
            clean_response: "self-check pong".to_string(),
            raw_response: None,
            usage: None,
        };
        std::fs::write(
            case_dir.join(FIXTURE_CALLS_FILENAME),
            serde_json::to_string(&rec).unwrap(),
        )
        .expect("write self_check fixture");

        let env = FixtureEnvGuard::install(&root, case, "2026-04-19T12:00:00+08:00");

        assert_eq!(
            std::env::var(FIXTURE_LLM_ROOT_ENV).unwrap(),
            root.display().to_string()
        );
        assert_eq!(std::env::var(FIXTURE_LLM_CASE_ENV).unwrap(), case);
        assert_eq!(
            std::env::var(TEST_FREEZE_NOW_ENV).unwrap(),
            "2026-04-19T12:00:00+08:00"
        );

        let provider = PROVIDER_IMPLS
            .iter()
            .find(|p| p.name() == FIXTURE_REPLAY_PROVIDER_TYPE)
            .expect("fixture_replay must be registered in PROVIDER_IMPLS");

        let runtime = build_fixture_replay_runtime("vendor-fixture-self-check");
        let resp = provider
            .call(runtime, prompt.to_string(), ChatRequestHints::default())
            .await
            .expect("self_check fixture must hit");
        assert_eq!(resp.text, "self-check pong");
        assert_eq!(resp.request_payload["prompt_hash"], hash);

        drop(env);

        assert!(
            std::env::var(FIXTURE_LLM_ROOT_ENV).is_err(),
            "guard drop must clear FIXTURE_LLM_ROOT_ENV"
        );
        assert!(
            std::env::var(FIXTURE_LLM_CASE_ENV).is_err(),
            "guard drop must clear FIXTURE_LLM_CASE_ENV"
        );
        assert!(
            std::env::var(TEST_FREEZE_NOW_ENV).is_err(),
            "guard drop must clear TEST_FREEZE_NOW_ENV"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 严格冒烟：fixture root 必须落在 `crates/clawd/tests/fixtures/llm_io`，
    /// 防止有人误把 fixture 散到别的 crate 或 git ignore 目录。
    #[test]
    fn fixture_workspace_root_points_to_in_repo_path() {
        let root = fixture_workspace_root();
        let s = root.display().to_string();
        assert!(
            s.ends_with("crates/clawd/tests/fixtures/llm_io"),
            "unexpected fixture root: {s}"
        );
    }

    /// Step 2.b 闭环：模拟一次"真录制"产物 —— 一段合成的 `model_io.log` JSONL，
    /// 喂给 [`convert_model_io_log_to_fixture`] 转出 `Vec<RecordedCall>`，序列化
    /// 到磁盘当作 `calls.jsonl`，再走生产 [`PROVIDER_IMPLS`] dispatch table 调
    /// 起来命中。
    ///
    /// 这条测试的语义是：**只要 P7.5 真录制能写出符合 §7.5 Step 2.b 后日志格式
    /// （含 `prompt_hash` 字段）的 model_io.log，整个 P7.5 链条就能跑通**。剩下的
    /// 唯一变量就是真 LLM 是否产生了我们预期的 prompt 序列 —— 这只能靠手工录制
    /// 验证，但不会再因为 hash / 截断 / 缓存 / dispatch 这些**基础设施**问题挂掉。
    #[tokio::test]
    async fn step2b_self_check_full_loop_log_to_convert_to_replay() {
        let _guard = fixture_env_lock();
        clear_cache_for_test();

        let prompt_a = "normalizer prompt for ping";
        let prompt_b = "chat response prompt for ping";
        let hash_a = fnv1a_64_hex(prompt_a);
        let hash_b = fnv1a_64_hex(prompt_b);

        // 1) 合成一次"真录制"的 model_io.log：含 verbose ok / 一条 slim / 一条
        // verbose error，验证 convert_* 真的只挑 verbose+ok 出来。
        let make_verbose = |hash: &str, prompt: &str, source: &str, clean: &str| {
            serde_json::json!({
                "ts": 1_700_000_000u64,
                "mode": "verbose",
                "task_id": "task-loop",
                "user_id": "u1",
                "chat_id": "c1",
                "vendor": "openai",
                "provider": "vendor-openai",
                "provider_type": "openai",
                "model": "gpt-test",
                "model_kind": "chat",
                "status": "ok",
                "prompt_source": source,
                "prompt_hash": hash,
                "prompt": prompt,
                "request_payload": {"foo": 1},
                "response": clean,
                "raw_response": clean,
                "clean_response": clean,
                "usage": {"prompt_tokens": 12, "completion_tokens": 3},
                "sanitized": false,
                "error": null,
            })
            .to_string()
        };
        let slim_line = serde_json::json!({
            "ts": 1u64, "mode": "slim", "task_id": "task-loop",
            "status": "ok", "prompt_source": "noise", "prompt_chars": 10u64,
        })
        .to_string();
        let mut errored = serde_json::from_str::<serde_json::Value>(&make_verbose(
            "deadbeefcafe0000",
            "ignored",
            "noise",
            "ignored-clean",
        ))
        .unwrap();
        errored["status"] = serde_json::json!("error");

        let log = format!(
            "{}\n{}\n{}\n{}\n",
            make_verbose(&hash_a, prompt_a, "intent_normalizer", "{\"intent\":\"ask\"}"),
            slim_line,
            errored,
            make_verbose(&hash_b, prompt_b, "chat_response", "Hello, world!"),
        );

        // 2) Convert 出 RecordedCall；只应留 2 条。
        let recs = convert_model_io_log_to_fixture(&log).expect("convert ok");
        assert_eq!(recs.len(), 2, "verbose+ok 行有 2 条，其它必须被过滤");
        let hashes: std::collections::HashSet<_> = recs.iter().map(|r| r.prompt_hash.clone()).collect();
        assert!(hashes.contains(&hash_a));
        assert!(hashes.contains(&hash_b));

        // 3) 把 recs 序列化成 calls.jsonl，落到一个临时 case 目录里。
        let root = fixture_workspace_root().join("__self_check_loop");
        let case = "loop";
        let case_dir = root.join(case);
        std::fs::create_dir_all(&case_dir).expect("mkdir loop case");
        let body = recs
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(case_dir.join(FIXTURE_CALLS_FILENAME), body)
            .expect("write loop calls.jsonl");

        // 4) 通过生产 dispatch table + env guard 把两条 prompt 都"重放"出来。
        let env = FixtureEnvGuard::install(&root, case, "2026-04-19T12:00:00+08:00");
        let provider = PROVIDER_IMPLS
            .iter()
            .find(|p| p.name() == FIXTURE_REPLAY_PROVIDER_TYPE)
            .expect("fixture_replay registered");
        let runtime = build_fixture_replay_runtime("vendor-fixture-loop");

        let resp_a = provider
            .call(runtime.clone(), prompt_a.to_string(), ChatRequestHints::default())
            .await
            .expect("loop hit a");
        assert_eq!(resp_a.text, "{\"intent\":\"ask\"}");
        let resp_b = provider
            .call(runtime, prompt_b.to_string(), ChatRequestHints::default())
            .await
            .expect("loop hit b");
        assert_eq!(resp_b.text, "Hello, world!");

        drop(env);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// §7.5 Step 3 工具入口：把 `regen_fixture_from_log` 包成 `cargo test` 可
    /// 直接调起来的形态，让 `scripts/regen_fixture.sh` 不必自己解析 model_io.log。
    ///
    /// 由 4 个 env 驱动（其它 env 一律不读，避免与其它 fixture 测试互污染）：
    ///   * `RUSTCLAW_REGEN_FIXTURE_CASE`（必填）—— case 名（fixture root 下的子目录）；
    ///   * `RUSTCLAW_REGEN_FIXTURE_LOG`（必填）—— 待解析的 model_io.log 路径；
    ///   * `RUSTCLAW_REGEN_FIXTURE_FORCE=1`—— 允许覆盖已存在的 calls.jsonl；
    ///   * `RUSTCLAW_REGEN_FIXTURE_DRY=1`—— 只解析、不落盘。
    ///
    /// `#[ignore]` 表示默认 `cargo test` 不跑（避免 CI 误触发文件 I/O）。
    /// 调用形态（由 `scripts/regen_fixture.sh` 拼出来）：
    ///
    /// ```bash
    /// RUSTCLAW_REGEN_FIXTURE_CASE=act_find_service_file \
    /// RUSTCLAW_REGEN_FIXTURE_LOG=/tmp/log.jsonl \
    /// cargo test -p clawd --bin clawd \
    ///   fixture_replay_e2e::tests::regen_fixture_tool \
    ///   -- --ignored --nocapture
    /// ```
    ///
    /// 任何错误都 `panic!`，让 `cargo test` 把消息显示出来；成功时通过
    /// `eprintln!`（`--nocapture` 才能看到）打印 [`crate::providers::fixture_replay::RegenSummary`]
    /// 摘要。
    #[test]
    #[ignore = "tool entry; only invoked by scripts/regen_fixture.sh with env vars"]
    fn regen_fixture_tool() {
        const CASE_ENV: &str = "RUSTCLAW_REGEN_FIXTURE_CASE";
        const LOG_ENV: &str = "RUSTCLAW_REGEN_FIXTURE_LOG";
        const FORCE_ENV: &str = "RUSTCLAW_REGEN_FIXTURE_FORCE";
        const DRY_ENV: &str = "RUSTCLAW_REGEN_FIXTURE_DRY";

        let case = std::env::var(CASE_ENV)
            .unwrap_or_else(|_| panic!("{CASE_ENV} env required (case name under fixture root)"));
        let log_path = std::env::var(LOG_ENV).unwrap_or_else(|_| {
            panic!("{LOG_ENV} env required (path to model_io.log to convert)")
        });
        let truthy = |v: String| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        };
        let force = std::env::var(FORCE_ENV).map(truthy).unwrap_or(false);
        let dry_run = std::env::var(DRY_ENV).map(truthy).unwrap_or(false);

        let log_text = std::fs::read_to_string(&log_path).unwrap_or_else(|e| {
            panic!("read log file {log_path:?} failed: {e}")
        });
        let root = fixture_workspace_root();
        let summary = regen_fixture_from_log(&log_text, &case, &root, dry_run, force)
            .unwrap_or_else(|e| panic!("regen_fixture_from_log failed: {e}"));

        eprintln!(
            "regen_fixture_tool ok: case={} written={} dry_run={} overwrote={} dest={}",
            case,
            summary.written_records,
            summary.dry_run,
            summary.overwrote_existing,
            summary.dest_path.display()
        );
    }

    /// §7.5 Step 4.a：扫所有已录 case 做 schema + best-effort 命中验证。
    ///
    /// **不带 `#[ignore]`，CI 一直跑** —— 没录任何 case 时会通过并打印
    /// "no recorded cases yet"。一旦你 commit 了任何 `<case>/calls.jsonl`，
    /// 本测试自动覆盖它，无需为每条 case 单独写 `#[test]`。
    ///
    /// 每个 case 的检查：
    ///   1. **Schema 完整性**：每条 record 能反序列化成 [`RecordedCall`]，
    ///      `prompt_hash` 长度 16 hex 字符（FNV-1a），`clean_response` 非空。
    ///   2. **Hash 唯一性**：同一 `calls.jsonl` 内 `prompt_hash` 不重复
    ///      （重复说明 [`convert_model_io_log_to_fixture`] 的 dedup 出了问题，
    ///      或者录制阶段 prompt 漂移）。
    ///   3. **Best-effort 命中**：把每条 `prompt_preview` 当 prompt 喂回 fixture
    ///      provider —— preview 没被截断（§7.5 抬到 128K 后正常 prompt 都不会
    ///      截）时 hash 一致，必命中；preview 被截断时报 miss，统计为
    ///      `preview_drift`，**不**算 fail。这是"fixture 内容能被 PROVIDER_IMPLS
    ///      正确分发"的最低烟雾，端到端命中率由 Step 4.b 的 process_ask_task
    ///      harness 覆盖。
    ///   4. **总耗时**：所有 case 加起来 < 1 秒（schema 检查纯 in-memory；
    ///      网络 / DB 都不该在路径上）。
    ///
    /// 输出 dashboard（`--nocapture` 可见）：
    /// ```text
    /// fixture-replay smoke dashboard:
    ///   act_find_service_file       :  7 records, 7 hit, 0 drift, 0.4 ms
    ///   chat_simple_hello           :  3 records, 2 hit, 1 drift, 0.2 ms
    /// total: 2 cases, 10 records, 9 hit, 1 drift, 0.6 ms
    /// ```
    #[tokio::test]
    async fn batch_replay_smoke_all_recorded_cases() {
        let _guard = fixture_env_lock();
        clear_cache_for_test();

        let cases = load_recorded_cases().expect("load recorded cases");
        if cases.is_empty() {
            eprintln!(
                "fixture-replay smoke: no recorded cases yet under {} — \
                 see scripts/regen_fixture.sh to record one.",
                fixture_workspace_root().display()
            );
            return;
        }

        let provider = PROVIDER_IMPLS
            .iter()
            .find(|p| p.name() == FIXTURE_REPLAY_PROVIDER_TYPE)
            .expect("fixture_replay registered");

        let mut total_records = 0usize;
        let mut total_hit = 0usize;
        let mut total_drift = 0usize;
        let total_start = std::time::Instant::now();
        let mut per_case_lines = Vec::new();

        for loaded in &cases {
            let case = &loaded.case;
            assert!(
                !loaded.records.is_empty(),
                "case {case} has empty calls.jsonl ({} records, expected >= 1) — \
                 if this is a placeholder, name the directory `_<case>` to skip the scan",
                loaded.records.len()
            );

            // 1) Schema 完整性
            for (idx, rec) in loaded.records.iter().enumerate() {
                let path = loaded.calls_path.display();
                assert_eq!(
                    rec.prompt_hash.len(),
                    16,
                    "{path} record #{idx}: prompt_hash must be FNV-1a 64-bit hex \
                     (16 chars), got len={}",
                    rec.prompt_hash.len()
                );
                assert!(
                    rec.prompt_hash.chars().all(|c| c.is_ascii_hexdigit()),
                    "{path} record #{idx}: prompt_hash must be lowercase hex, got {:?}",
                    rec.prompt_hash
                );
                assert!(
                    !rec.clean_response.is_empty(),
                    "{path} record #{idx}: clean_response must not be empty (status=ok \
                     records always carry response text)"
                );
            }

            // 2) Hash 唯一性
            let mut seen = std::collections::HashSet::new();
            for rec in &loaded.records {
                assert!(
                    seen.insert(rec.prompt_hash.clone()),
                    "{}: prompt_hash {} appears more than once — convert_* dedup \
                     should have collapsed these; possible re-record drift",
                    loaded.calls_path.display(),
                    rec.prompt_hash,
                );
            }

            // 3) Best-effort 命中：env 切到本 case，逐条用 preview 喂回去。
            let case_start = std::time::Instant::now();
            let env = FixtureEnvGuard::install(
                &fixture_workspace_root(),
                case,
                "2026-04-19T12:00:00+08:00",
            );
            let runtime =
                build_fixture_replay_runtime(&format!("vendor-fixture-{case}"));
            let mut hit = 0usize;
            let mut drift = 0usize;
            for rec in &loaded.records {
                let preview = match rec.prompt_preview.clone() {
                    Some(p) if !p.is_empty() => p,
                    _ => {
                        // 没 preview / preview 空 —— 无法做 best-effort 命中，
                        // 直接计入 drift。
                        drift += 1;
                        continue;
                    }
                };
                match provider
                    .call(runtime.clone(), preview, ChatRequestHints::default())
                    .await
                {
                    Ok(resp) => {
                        assert_eq!(
                            resp.text, rec.clean_response,
                            "preview hit but returned different clean_response for \
                             case={case} hash={}",
                            rec.prompt_hash
                        );
                        hit += 1;
                    }
                    Err(e) => {
                        // 必须是 miss-style 错（dispatcher 通了，只是 hash 对不上），
                        // 不能是其它系统错（env 没 set / 文件不存在 / etc.）。
                        assert!(
                            e.message.contains("fixture_replay miss"),
                            "case={case} unexpected provider error: {}",
                            e.message
                        );
                        drift += 1;
                    }
                }
            }
            drop(env);

            let case_ms = case_start.elapsed().as_secs_f64() * 1000.0;
            total_records += loaded.records.len();
            total_hit += hit;
            total_drift += drift;
            per_case_lines.push(format!(
                "  {case:<32}: {n:>3} records, {h:>3} hit, {d:>3} drift, {ms:>5.1} ms",
                case = case,
                n = loaded.records.len(),
                h = hit,
                d = drift,
                ms = case_ms,
            ));
        }

        let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;
        eprintln!("fixture-replay smoke dashboard:");
        for line in &per_case_lines {
            eprintln!("{line}");
        }
        eprintln!(
            "total: {} cases, {} records, {} hit, {} drift, {:.1} ms",
            cases.len(),
            total_records,
            total_hit,
            total_drift,
            total_ms
        );

        // 4) 整批 schema + smoke < 1 秒。process_ask_task 端到端有自己的 5s 预算，
        // 这里只盯纯 in-memory 的部分。
        assert!(
            total_ms < 1_000.0,
            "batch_replay_smoke total {:.1} ms exceeds 1s budget — fixture growth \
             or non-trivial work crept into the smoke path",
            total_ms,
        );
    }

    /// §7.5 Step 4.b.1：验证 [`AppState::test_default_with_fixture_provider`]
    /// 装出来的 state，**通过生产路径** [`AppState::task_llm_providers`] 取 provider
    /// 列表 → 经 [`PROVIDER_IMPLS`] 这条生产分发表 → 命中 fixture，全链路通。
    ///
    /// 区别于 [`step2a_self_check_e2e_wiring_through_provider_impls`]：那条测试
    /// 直接 `build_fixture_replay_runtime("vendor-fixture-self-check")` 自己
    /// 攒 runtime 调起来，**不**经 `AppState`；本测试走 `AppState` 真路径，
    /// 保证未来 `process_ask_task` harness 拿到的 provider 列表里**真的有**
    /// fixture replay。
    ///
    /// 任何一环坏（helper 没装 provider / agent_runtime 把 provider 吞了 /
    /// task_llm_providers 走错分支），本测试会挂；这是 Step 4.b 真 e2e harness
    /// 的入口前置 self-check。
    #[tokio::test]
    async fn step4b1_self_check_appstate_with_fixture_provider_routes_through_task() {
        let _guard = fixture_env_lock();
        clear_cache_for_test();

        let root = fixture_workspace_root().join("__appstate_wiring");
        let case = "task_routing";
        let case_dir = root.join(case);
        std::fs::create_dir_all(&case_dir).expect("mkdir appstate wiring case");

        let prompt = "appstate-wired ping";
        let hash = fnv1a_64_hex(prompt);
        let rec = RecordedCall {
            prompt_hash: hash.clone(),
            prompt_source: Some("appstate_wiring".to_string()),
            prompt_preview: Some(prompt.to_string()),
            clean_response: "appstate-wired pong".to_string(),
            raw_response: None,
            usage: None,
        };
        std::fs::write(
            case_dir.join(FIXTURE_CALLS_FILENAME),
            serde_json::to_string(&rec).unwrap(),
        )
        .expect("write appstate wiring fixture");

        let env = FixtureEnvGuard::install(&root, case, "2026-04-19T12:00:00+08:00");

        let state = crate::AppState::test_default_with_fixture_provider();
        assert_eq!(
            state.core.active_provider_type.as_deref(),
            Some(FIXTURE_REPLAY_PROVIDER_TYPE),
            "AppState helper must set active_provider_type so call sites that branch \
             on provider_type pick up fixture_replay"
        );

        // 走 ClaimedTask → task_llm_providers 真路径，保证生产分支里
        // 没有偷偷 fallback 到一条空 provider 列表。
        let task = crate::ClaimedTask {
            task_id: "task-step4b1".to_string(),
            user_id: 1,
            chat_id: 1,
            user_key: None,
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: serde_json::json!({}).to_string(),
        };
        let providers = state.task_llm_providers(&task);
        assert_eq!(
            providers.len(),
            1,
            "expected exactly 1 fixture provider, got {}",
            providers.len()
        );
        assert_eq!(providers[0].config.provider_type, FIXTURE_REPLAY_PROVIDER_TYPE);

        let dispatch = PROVIDER_IMPLS
            .iter()
            .find(|p| p.name() == providers[0].config.provider_type)
            .expect("PROVIDER_IMPLS must dispatch fixture_replay");
        let resp = dispatch
            .call(
                providers[0].clone(),
                prompt.to_string(),
                ChatRequestHints::default(),
            )
            .await
            .expect("AppState-routed fixture call must hit");
        assert_eq!(resp.text, "appstate-wired pong");
        assert_eq!(resp.request_payload["prompt_hash"], hash);

        drop(env);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// §7.5 Step 4.b.2.1：验证 [`crate::AppState::with_minimal_builtin_registry`]
    /// 链式 helper 装出来的 registry **能过 §P4.1 integrity 校验**，且 `core` 里
    /// `skill_views_snapshot` 真被替换（不是被静默丢弃）。
    ///
    /// 前置：`process_ask_task` 启动期会跑 `integrity_report().is_clean()`，缺
    /// 任何一条 [`claw_core::skill_registry::REQUIRED_BUILTIN_SKILLS`] 就 bail；
    /// 本测试是 e2e harness 启动这道门的 self-check 入口 —— 任何后续 builtin
    /// 增删漏在 helper 里都会让本测试先红，而不是污染 e2e 报错。
    #[tokio::test]
    async fn step4b2_1_self_check_minimal_builtin_registry_satisfies_integrity() {
        use claw_core::skill_registry::REQUIRED_BUILTIN_SKILLS;

        let state = crate::AppState::test_default_with_fixture_provider()
            .with_minimal_builtin_registry();

        let registry = state
            .get_skills_registry()
            .expect("with_minimal_builtin_registry must install Some(registry)");
        let report = registry.integrity_report();
        assert!(
            report.is_clean(),
            "minimal builtin registry must satisfy integrity check, got {:?}",
            report,
        );

        let installed: std::collections::HashSet<String> =
            registry.all_names().into_iter().collect();
        for required in REQUIRED_BUILTIN_SKILLS {
            assert!(
                installed.contains(*required),
                "minimal registry missing REQUIRED builtin {required:?}; \
                 if you added a new builtin to REQUIRED_BUILTIN_SKILLS, also \
                 extend with_minimal_builtin_registry to spit it into the toml"
            );
        }

        let skills_list = state.get_skills_list();
        assert_eq!(
            skills_list.len(),
            REQUIRED_BUILTIN_SKILLS.len(),
            "skills_list snapshot must equal enabled builtin set, got {:?}",
            skills_list,
        );
    }

    /// §7.5 Step 4.b.2.2：验证 [`crate::AppState::with_prompt_layers_installed`]
    /// 把 `workspace_root` 指到真仓库根后，**通过生产路径**
    /// [`crate::bootstrap::prompts::load_prompt_template_for_state_with_meta`]
    /// 加载 `prompts/intent_normalizer_prompt.md` 拿到的是磁盘 layered manifest
    /// 的拼接结果，而不是各 callsite 的 `include_str!` 兜底常量。
    ///
    /// 同时确认 `default_locator_search_dir` 没被这个 helper 改写 —— 这是 helper
    /// 的安全契约（见 helper doc）。
    #[tokio::test]
    async fn step4b2_2_self_check_prompt_layers_installed_loads_real_disk_prompt() {
        let state = crate::AppState::test_default_with_fixture_provider()
            .with_minimal_builtin_registry()
            .with_prompt_layers_installed();

        assert!(
            state
                .skill_rt
                .workspace_root
                .join("prompts/layers/manifest.toml")
                .is_file(),
            "with_prompt_layers_installed must point workspace_root at a tree \
             containing prompts/layers/manifest.toml; got {}",
            state.skill_rt.workspace_root.display(),
        );

        // 用一段刻意不像生产 prompt 的兜底字符串，便于通过"是否等于它"判断
        // 命中了磁盘还是 include_str fallback。
        const SENTINEL_FALLBACK: &str = "<EMBEDDED_FALLBACK_FOR_STEP4B22_SELF_CHECK>";
        let resolved = crate::bootstrap::prompts::load_prompt_template_for_state_with_meta(
            &state,
            "prompts/intent_normalizer_prompt.md",
            SENTINEL_FALLBACK,
        );
        assert_ne!(
            resolved.template, SENTINEL_FALLBACK,
            "with_prompt_layers_installed must resolve real disk prompt, not the \
             include_str fallback (source={})",
            resolved.source,
        );
        assert!(
            !resolved.template.trim().is_empty(),
            "resolved layered prompt body must be non-empty (source={})",
            resolved.source,
        );
        assert!(
            resolved.source.starts_with("layered:")
                || resolved.source.contains("intent_normalizer"),
            "source should reflect the layered manifest or the disk path, got {:?}",
            resolved.source,
        );

        assert_eq!(
            state.skill_rt.default_locator_search_dir,
            std::env::temp_dir(),
            "with_prompt_layers_installed must NOT change default_locator_search_dir; \
             only workspace_root should be promoted to the real repo root, otherwise \
             locator paths would scan the whole git tree"
        );
    }

    /// §7.5 Step 4.b.2.3：契约守底测试 ——【fixture-replay e2e 下 channel push
    /// **绝不应该**真发 HTTP】。
    ///
    /// 调研结论（详见 commit log）：
    ///   * `process_ask_task` 主流程**不**调用 `channel_send::*`；任何 finalize /
    ///     loop_reply 路径都不主动 push 通道，回复经 DB → 各通道 daemon
    ///     polling 反向出去。
    ///   * 唯一 channel push 调用点是 [`crate::worker::maybe_notify_schedule_result`]，
    ///     仅在 `payload.schedule_triggered == true` 时走 `send_task_channel_message`，
    ///     且对 send 失败 **fail-soft**（只 `warn!`，不 propagate）。
    ///   * `ChannelConfig::default()`（`AppState::test_default_with_fixture_provider`
    ///     用的就是它）所有 token 字段均为空串/`None`，每个 `channel_send::send_*`
    ///     入口第一步 `if token.is_empty() { return Err(...) }` short-circuit
    ///     返回，**不发任何 HTTP 请求**。
    ///
    /// 本测试把上面三条契约一次性钉死：
    ///   1. 默认 channels 所有可标识为"配好了"的字段都是 empty / None；
    ///   2. 直接调 `send_telegram_message(&state, 0, "...")` 必返回带
    ///      `telegram bot token is empty` 的 Err，证明 short-circuit 路径走通；
    ///   3. （隐式）—— 调用过程中**没**发 HTTP，因为 token 检查在任何 reqwest
    ///      调用之前。
    ///
    /// 守底意义：未来若有人手贱往 `ChannelConfig::default()` 里塞了真 token
    /// 默认值（例如挪 dev token 来调试 / fixture seed 时不慎复制粘贴），本测试
    /// 会立刻挂掉，避免 fixture-replay 测试集**默默地**对真生产 telegram /
    /// wechat / 飞书后端发 HTTP。
    #[tokio::test]
    async fn step4b2_3_self_check_default_channels_short_circuit_without_http() {
        let state = crate::AppState::test_default_with_fixture_provider();

        assert!(
            state.channels.telegram_bot_token.is_empty(),
            "default channels.telegram_bot_token must stay empty in tests, got {:?}",
            state.channels.telegram_bot_token,
        );
        assert!(
            state.channels.whatsapp_access_token.is_empty(),
            "default channels.whatsapp_access_token must stay empty in tests, got {:?}",
            state.channels.whatsapp_access_token,
        );
        assert!(
            !state.channels.whatsapp_cloud_enabled,
            "default channels.whatsapp_cloud_enabled must stay false"
        );
        assert!(
            !state.channels.whatsapp_web_enabled,
            "default channels.whatsapp_web_enabled must stay false"
        );
        assert!(
            state.channels.wechat_send_config.is_none(),
            "default channels.wechat_send_config must stay None in tests"
        );
        assert!(
            state.channels.feishu_send_config.is_none(),
            "default channels.feishu_send_config must stay None in tests"
        );
        assert!(
            state.channels.lark_send_config.is_none(),
            "default channels.lark_send_config must stay None in tests"
        );

        let err = crate::channel_send::send_telegram_message(&state, 0, "should-not-send")
            .await
            .expect_err(
                "with empty telegram_bot_token, send_telegram_message must short-circuit \
                 with Err(...) instead of issuing an HTTP call to api.telegram.org",
            );
        assert!(
            err.contains("telegram bot token is empty"),
            "Err message must come from the short-circuit branch, not from a network \
             error. Got: {err}",
        );
    }

    /// §7.5 Step 4.b.2（待 DB schema seed 到位）：真正驱动
    /// [`crate::worker::process_ask_task`] 的端到端 harness。当前只是占位 + 文档。
    ///
    /// 已落地子项：
    ///   * 4.b.1（本文件 `step4b1_self_check_appstate_with_fixture_provider_routes_through_task`）：
    ///     `AppState` 装得起 fixture provider，`task_llm_providers` 取得到，
    ///     `PROVIDER_IMPLS` 命中。
    ///   * 4.b.2.1（本文件 `step4b2_1_self_check_minimal_builtin_registry_satisfies_integrity`）：
    ///     [`AppState::with_minimal_builtin_registry`] 链式 helper 装出
    ///     integrity-clean 的 builtin 注册表。
    ///   * 4.b.2.2（本文件 `step4b2_2_self_check_prompt_layers_installed_loads_real_disk_prompt`）：
    ///     [`AppState::with_prompt_layers_installed`] 链式 helper 把
    ///     `workspace_root` 指到真仓库根，让 `load_prompt_template_for_state*`
    ///     命中 `prompts/layers/manifest.toml` 拼层 → fnv1a 输入与录制时一致。
    ///   * 4.b.2.3（本文件 `step4b2_3_self_check_default_channels_short_circuit_without_http`）：
    ///     调研发现 `process_ask_task` 主流程不调 channel push，
    ///     `maybe_notify_schedule_result` 是 fail-soft，且 `ChannelConfig::default()`
    ///     全空字段让 `channel_send::send_*` 立即 short-circuit 不发 HTTP ——
    ///     无需 helper，只补一份契约守底测试。
    ///
    /// 仍待补的剩余工程：
    ///   1. **DB schema seed**：`db_init::test_pool` 已建表，但 `tasks` 行需要
    ///      先 insert 一条对应 `task.task_id` 的记录，否则 finalize 的 audit
    ///      写入会 FK 失败；
    ///   2. **每个 case 目录下 commit `expected.json`**：含 `user_text` /
    ///      `expected_final_answer_contains` / `expected_llm_call_count` /
    ///      `expected_prompt_sources` / `expected_verifier_verdict` /
    ///      `expected_fallback_source` 字段；
    ///   3. 删掉本测试的 `#[ignore]`。
    #[tokio::test]
    #[ignore = "Step 4.b.2 占位：4.b.1 / 4.b.2.1 / 4.b.2.2 / 4.b.2.3 已落地，等 1-3 项就绪再启用"]
    async fn e2e_per_case_replay_with_process_ask_task() {
        // 见上方 doc。
    }
}
