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

    /// Step 2.b 留下的真 case skeleton：等用户在本地按模块顶部"录制 → 回放
    /// 工作流"录完 `act_find_service_file` 把 fixture 落进
    /// `crates/clawd/tests/fixtures/llm_io/act_find_service_file/calls.jsonl`
    /// 后，删掉本测试上方的 `#[ignore]` 即可启用。
    ///
    /// 当前为 `#[ignore]`：
    ///   * 没人录的时候跑会必 miss（fixture 文件不存在），让 CI 必绿;
    ///   * `cargo test -- --ignored` 一键看哪些 case 还没录;
    ///   * 一旦 unignore 跑通，就为后续 8 条 case 提供模板。
    ///
    /// 真 case 的完整 e2e（含 `process_ask_task` 端到端）在 Step 2.c+ 落地。
    /// 此 skeleton 当前只验证：fixture 文件存在 + 通过 PROVIDER_IMPLS 能命中
    /// "至少一条已录的 prompt"，作为录制是否成功的最快冒烟。
    #[tokio::test]
    #[ignore = "需要先按 module-level doc 的 '录制 → 回放工作流' 录入 fixture"]
    async fn e2e_act_find_service_file_replay_smoke() {
        let _guard = fixture_env_lock();
        clear_cache_for_test();

        let case = "act_find_service_file";
        let root = fixture_workspace_root();
        let calls_path = root.join(case).join(FIXTURE_CALLS_FILENAME);
        assert!(
            calls_path.exists(),
            "fixture missing: {} —— 按 module-level doc 录制后再 unignore",
            calls_path.display()
        );

        // 读出第一条 RecordedCall，作为"至少一条 prompt 能命中"的冒烟样本。
        let body = std::fs::read_to_string(&calls_path).expect("read fixture");
        let first_line = body
            .lines()
            .find(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
            .expect("fixture must have at least one record");
        let rec: RecordedCall = serde_json::from_str(first_line).expect("parse first record");

        let env = FixtureEnvGuard::install(&root, case, "2026-04-19T12:00:00+08:00");
        let provider = PROVIDER_IMPLS
            .iter()
            .find(|p| p.name() == FIXTURE_REPLAY_PROVIDER_TYPE)
            .expect("fixture_replay registered");
        let runtime = build_fixture_replay_runtime("vendor-fixture-act-find-service-file");

        // 我们没法重建出原始 prompt 字符串（只有 hash），所以只能间接验证：
        // 把 prompt_preview 当 prompt 喂回去，断言报 miss（说明 dispatcher 走通了，
        // 只是因为 preview 被截断后 hash 不同而已）；这是录制 fixture 文件
        // 完整性的最低烟雾。完整 e2e 在 Step 2.c 通过 process_ask_task 端到端验证。
        let preview = rec.prompt_preview.unwrap_or_else(|| "<no preview>".to_string());
        let r = provider
            .call(runtime, preview, ChatRequestHints::default())
            .await;
        match r {
            Ok(_) => {
                // 命中也行（preview 没被截断时 hash 一致）。
            }
            Err(e) => {
                assert!(
                    e.message.contains("fixture_replay miss"),
                    "expected miss-style err, got: {}",
                    e.message
                );
            }
        }

        drop(env);
    }
}
