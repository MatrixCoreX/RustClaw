//! Phase 7 §7.5 Step 2.a：fixture-replay 端到端 harness 骨架。
//!
//! 与 [`crate::providers::fixture_replay`] 单测的区别：
//!   * 单测只覆盖 provider 内部（hash / 缓存 / miss / malformed line）。
//!   * 本模块覆盖 **整条 wiring 链**：
//!       1. `FIXTURE_LLM_ROOT` / `FIXTURE_LLM_CASE` env 切换；
//!       2. `RUSTCLAW_TEST_FREEZE_NOW` 让 [`crate::schedule_service`] 注入的
//!          normalizer prompt `__NOW__` 字段稳定；
//!       3. 通过 [`crate::providers::client::PROVIDER_IMPLS`] 这条生产分发表
//!          找到 `fixture_replay` provider 并真的调起来；
//!       4. RAII guard 保证：测试 panic / 提前 return 都能把上面三条 env 还原，
//!          下一条测试不会"幽灵命中"上一条 case。
//!
//! Step 2.b 会在本模块继续追加每 case 一个 `#[tokio::test]`，组装最小 AppState
//! 调 `process_ask_task` 跑通 8 条 fixture。

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
        build_fixture_replay_runtime, clear_cache_for_test, fnv1a_64_hex, RecordedCall,
        FIXTURE_CALLS_FILENAME,
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
}
