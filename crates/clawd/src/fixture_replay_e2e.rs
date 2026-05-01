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

use serde::{Deserialize, Serialize};

use crate::providers::fixture_replay::{
    FIXTURE_LLM_CASE_ENV, FIXTURE_LLM_ROOT_ENV, FIXTURE_REPLAY_PROVIDER_TYPE,
};
use crate::schedule_service::TEST_FREEZE_NOW_ENV;

/// §7.5 Step 4.b.2.5：每个 case 目录下 `expected.json` 的文件名常量。
///
/// 与 [`crate::providers::fixture_replay::FIXTURE_CALLS_FILENAME`]（`calls.jsonl`）
/// 配套：`calls.jsonl` 描述"喂给 LLM 的 prompt + LLM 应该回什么"，
/// `expected.json` 描述"端到端跑完后业务层面应该看到什么"。
pub(crate) const FIXTURE_EXPECTED_FILENAME: &str = "expected.json";

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

/// §7.5 Step 4.b.2.5：每个 case 的端到端期望声明。
///
/// **设计原则**：
///   1. **全字段可选**：让一条新 case 可以"先只断言一个 contains 子串"上岗，
///      日后再逐步加严。`load_expected_for_case` 文件不存在也返回 `Ok(None)`，
///      不强制每个 fixture 目录都有 `expected.json`（兼容仅含 `calls.jsonl`
///      的 smoke fixture）。
///   2. **严 schema**：开了 `deny_unknown_fields`，typo 立刻在 parse 阶段
///      报错，避免新加字段被悄悄拼错。
///   3. **冗余字段做 cross-check**：`case` 字段必须等于所在目录名，避免
///      文件被错挂到别的 case 下还能"看似正常"。
///   4. **无 LLM-specific 字段下沉到 fixture-replay 层**：harness 用
///      [`crate::providers::fixture_replay::RecordedCall`] + 本结构两份，
///      角色分离：calls = 输入端 contract（prompt → response），
///      expected = 输出端 contract（业务可观察值）。
///
/// **字段语义**：
///   * `case`：必须与目录名相同（典型：`act_find_service_file`）。
///   * `description`：人类可读注释，仅文档用，不参与断言。
///   * `user_text`：用户输入原文，作为 ask payload 的 `text` 字段；harness
///     会把它和 [`crate::repo::submit::insert_submitted_task`] 一致地包成
///     `{"text": user_text}` 写进 `tasks.payload_json`。
///   * `freeze_now`：`RUSTCLAW_TEST_FREEZE_NOW` 应当注入的 wallclock；必须
///     与录制 `calls.jsonl` 时一致，否则 normalizer prompt 里的 `__NOW__`
///     字段会让 fnv1a hash 漂、fixture miss。
///   * `user_id` / `chat_id`：seed 进 `tasks` 行，缺省 1 / 1 —— 与 telegram
///     allowlist 里的"自家人"约定俗成保持一致；`user_id < 0` 留给 webd 用户
///     体系（与 [`crate::repo::auth`] 的 `negative-id-for-webd` 约定一致）。
///
///   * `expected_final_answer_contains`：所有列出的子串都必须出现在 final
///     answer 文本里（顺序无关，大小写敏感）。最常用的轻量断言。
///   * `expected_final_answer_not_contains`：禁止出现的子串集合。用来卡住
///     "幻觉文案"或老版本的固定坏话术（如：旧回复链把"有没有"答成
///     "这是 systemd 文件…"段落式描述）。
///   * `expected_llm_call_count`：精确等于。仅当此 case 的 LLM 调用数稳定
///     时才设；不稳定时用 `expected_min_llm_call_count` /
///     `expected_max_llm_call_count` 给区间。
///   * `expected_prompt_sources`：**集合（无序）** 断言 —— 每个列出的
///     [`crate::llm_gateway::classify_prompt_source`] 标签必须在本次任务里
///     至少被调用过一次。来源是 `state.task_llm_by_prompt(task_id)` 的 key
///     集（HashMap，无调用顺序）。允许值：`normalizer` / `plan` /
///     `plan_repair` / `delivery_classifier` / `observed` / `clarify` /
///     `intent_meta` / `schedule` / `nl2cmd` / `self_extension` / `memory` /
///     `verifier` / `chat` / `semantic_judge` / `router_legacy` / `other`。
///     未来若需要"按顺序"断言，需在 `state.metrics` 加事件序列字段 —— 当前
///     不支持。
///   * `expected_fallback_source`：断言 finalizer 收尾使用的 fallback 标签，
///     源自 `task_journal.summary.finalizer_summary.fallback`。允许值见
///     [`crate::task_journal::TaskJournalFinalizerFallback::as_str`]
///     （`raw_text` / `no_answer_nonqualified` / `no_answer_parse_failed`）。
///     `None` / 缺字段 = 不断言。
///   * `expected_verifier_verdict`：**当前未落 journal**，[`OutputContractVerdict`]
///     只走 tracing event，没有结构化字段可断言。schema 保留字段名，但
///     [`diff_outcome_against_expected`] 会把它当成"未实现的预期断言"
///     `panic!`，避免 case 文件设了字段而被静默跳过。后续在 `task_journal`
///     里加 `output_contract_verdict: Option<...>` 之后再启用。
///
///     [`OutputContractVerdict`]: crate::output_contract_verifier::OutputContractVerdict
///   * `expected_final_status`：断言
///     [`crate::task_journal::TaskJournalFinalStatus`]（`success` /
///     `failure` / `clarify` / `resume_failure`）。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectedPriorTurn {
    pub user_text: String,
    pub assistant_text: String,
    pub updated_at: String,

    #[serde(
        default = "default_prior_turn_final_status",
        skip_serializing_if = "is_default_prior_turn_final_status"
    )]
    pub final_status: String,
}

fn default_prior_turn_final_status() -> String {
    "success".to_string()
}

fn is_default_prior_turn_final_status(v: &String) -> bool {
    v == "success"
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectedCase {
    pub case: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    pub user_text: String,

    pub freeze_now: String,

    #[serde(
        default = "default_user_id",
        skip_serializing_if = "is_default_user_id"
    )]
    pub user_id: i64,
    #[serde(
        default = "default_chat_id",
        skip_serializing_if = "is_default_chat_id"
    )]
    pub chat_id: i64,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prior_turns: Vec<ExpectedPriorTurn>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_final_answer_contains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_final_answer_not_contains: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_llm_call_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_min_llm_call_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_max_llm_call_count: Option<u32>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_prompt_sources: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_fallback_source: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_verifier_verdict: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_final_status: Option<String>,
}

fn default_user_id() -> i64 {
    1
}
fn default_chat_id() -> i64 {
    1
}
fn is_default_user_id(v: &i64) -> bool {
    *v == 1
}
fn is_default_chat_id(v: &i64) -> bool {
    *v == 1
}

impl ExpectedCase {
    /// 在 case 目录尝试加载 `expected.json`。
    ///
    /// 文件不存在 → `Ok(None)`（仅含 `calls.jsonl` 的 smoke fixture 合法）。
    /// 文件存在 → 解析 + 调 [`Self::validate_against_dir`] cross-check。
    /// 解析失败 / cross-check 失败 → `Err(具体原因 + 路径)`。
    pub(crate) fn load_for_case(case_dir: &std::path::Path) -> Result<Option<Self>, String> {
        let path = case_dir.join(FIXTURE_EXPECTED_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
        let body = std::fs::read_to_string(&path)
            .map_err(|e| format!("read {} failed: {e}", path.display()))?;
        let parsed: Self = serde_json::from_str(&body).map_err(|e| {
            format!(
                "{} parse expected.json failed: {e} \
                 (typo in field name? schema deny_unknown_fields)",
                path.display()
            )
        })?;
        parsed
            .validate_against_dir(case_dir)
            .map_err(|e| format!("{} validate expected.json failed: {e}", path.display()))?;
        Ok(Some(parsed))
    }

    /// 一致性 cross-check：
    ///   * `case` 字段必须等于目录名；
    ///   * `freeze_now` 非空；
    ///   * `user_text` 非空；
    ///   * 三条 LLM call count 约束相互不冲突（`exact` 与 `min/max` 不能同时给
    ///     而又互相违背）。
    fn validate_against_dir(&self, case_dir: &std::path::Path) -> Result<(), String> {
        let dir_name = case_dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("case dir name not utf8: {}", case_dir.display()))?;
        if self.case != dir_name {
            return Err(format!(
                "expected.case = {:?} but lives under directory {:?}",
                self.case, dir_name
            ));
        }
        if self.user_text.is_empty() {
            return Err("user_text must not be empty".to_string());
        }
        if self.freeze_now.is_empty() {
            return Err(
                "freeze_now must not be empty (must match the wallclock used during recording, \
                 e.g. \"2026-04-19T12:00:00+08:00\")"
                    .to_string(),
            );
        }
        for (idx, prior_turn) in self.prior_turns.iter().enumerate() {
            if prior_turn.user_text.is_empty() {
                return Err(format!("prior_turns[{idx}].user_text must not be empty"));
            }
            if prior_turn.assistant_text.is_empty() {
                return Err(format!(
                    "prior_turns[{idx}].assistant_text must not be empty"
                ));
            }
            if prior_turn.updated_at.is_empty() {
                return Err(format!("prior_turns[{idx}].updated_at must not be empty"));
            }
            if prior_turn.final_status.is_empty() {
                return Err(format!("prior_turns[{idx}].final_status must not be empty"));
            }
        }
        if let (Some(exact), Some(min)) = (
            self.expected_llm_call_count,
            self.expected_min_llm_call_count,
        ) {
            if exact < min {
                return Err(format!(
                    "expected_llm_call_count ({exact}) < expected_min_llm_call_count ({min})"
                ));
            }
        }
        if let (Some(exact), Some(max)) = (
            self.expected_llm_call_count,
            self.expected_max_llm_call_count,
        ) {
            if exact > max {
                return Err(format!(
                    "expected_llm_call_count ({exact}) > expected_max_llm_call_count ({max})"
                ));
            }
        }
        if let (Some(min), Some(max)) = (
            self.expected_min_llm_call_count,
            self.expected_max_llm_call_count,
        ) {
            if min > max {
                return Err(format!(
                    "expected_min_llm_call_count ({min}) > expected_max_llm_call_count ({max})"
                ));
            }
        }
        Ok(())
    }
}

/// §7.5 Step 4.b.2.6：`process_ask_task` 跑完后从 [`AppState`] + DB 抽取出来的
/// 业务可观察值快照。是 [`diff_outcome_against_expected`] 的输入，让"抽取"和
/// "断言"两步分离 —— 前者依赖 process_ask_task 真跑过，后者是纯函数可单测。
#[derive(Debug, Clone)]
pub(crate) struct ReplayOutcome {
    /// `tasks.task_id` —— 与 [`crate::ClaimedTask::task_id`] 同。
    pub task_id: String,
    /// `tasks.status`：`succeeded` / `failed` / `running` / `queued` / `canceled` / `timeout`。
    /// `process_ask_task` 收尾如果什么 update 都没跑（如 task 被预先 cancel），
    /// 这里仍是 `queued` —— 与生产 inspect 路径同语义。
    pub task_status: String,
    /// `tasks.error_text`：失败路径下 finalizer 写入。
    pub error_text: Option<String>,
    /// `tasks.result_json` 反序列化后的 `task_journal.summary.final_answer`
    /// （注意已被 `truncate_for_log` 截断，对短答案 contains 断言够用；
    /// 长答案断言请挑独特子串而非长片段）。
    pub final_answer_text: Option<String>,
    /// `task_journal.summary.final_status`：`success` / `failure` / `clarify` /
    /// `resume_failure`。task 还在 running 或 process_ask_task 早返时为 `None`。
    pub final_status: Option<String>,
    /// `task_journal.summary.finalizer_summary.fallback`：finalizer 走的 fallback 标签。
    pub fallback_source: Option<String>,
    /// `state.task_llm_call_count(task_id)`：本次任务实际发出的 LLM 逻辑调用数。
    pub llm_call_count: u64,
    /// `state.task_llm_by_prompt(task_id)` 的 key 集 —— 本次任务调用过的
    /// `classify_prompt_source` 标签集合，无序。
    pub prompt_sources_invoked: std::collections::BTreeSet<String>,
}

fn build_fixture_ask_payload_json(text: &str, call_id: &str) -> String {
    serde_json::json!({
        "text": text,
        "agent_mode": true,
        "channel": "ui",
        "agent_id": crate::DEFAULT_AGENT_ID,
        "call_id": call_id,
    })
    .to_string()
}

fn build_fixture_result_json(text: &str, final_status: &str) -> String {
    serde_json::json!({
        "text": text,
        "task_journal": {
            "summary": {
                "final_answer": text,
                "final_status": final_status,
            }
        }
    })
    .to_string()
}

/// §7.5 Step 4.b.2.6：从 `AppState` + DB 抽出 [`ReplayOutcome`]。
///
/// 优先读 `tasks.result_json.task_journal.summary.task_metrics`；如果当前任务还没
/// finalize 到 DB，再回退读 `AppState` 里的 live metrics 桶。读取 DB 失败 /
/// `result_json` 不是合法 JSON / status 列缺失都视为 `Err`，让 harness 立刻
/// panic 而不是给出半残数据。
pub(crate) fn extract_outcome_from_state(
    state: &crate::AppState,
    task_id: &str,
) -> Result<ReplayOutcome, String> {
    let conn = state
        .core
        .db
        .get()
        .map_err(|e| format!("acquire main-db conn for outcome read: {e}"))?;
    let row: (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT status, result_json, error_text FROM tasks WHERE task_id = ?1 LIMIT 1",
            rusqlite::params![task_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| {
            format!(
                "query tasks row for task_id={task_id}: {e} \
                 (did you forget to call seed_ask_task_row before process_ask_task?)"
            )
        })?;
    let (task_status, result_json_text, error_text) = row;

    let mut final_answer_text = None;
    let mut final_status = None;
    let mut fallback_source = None;
    let mut journal_llm_call_count = None;
    let mut journal_prompt_sources_invoked = None;
    if let Some(text) = &result_json_text {
        let parsed: serde_json::Value = serde_json::from_str(text).map_err(|e| {
            format!(
                "tasks.result_json for {task_id} is not valid JSON: {e}; raw = {}",
                crate::truncate_for_log(text)
            )
        })?;
        let summary = parsed.get("task_journal").and_then(|j| j.get("summary"));
        if let Some(summary) = summary {
            final_answer_text = summary
                .get("final_answer")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            final_status = summary
                .get("final_status")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            fallback_source = summary
                .get("finalizer_summary")
                .and_then(|f| f.get("fallback"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let task_metrics = summary.get("task_metrics");
            journal_llm_call_count = task_metrics
                .and_then(|m| m.get("llm_calls_per_task"))
                .and_then(|v| v.as_u64());
            journal_prompt_sources_invoked = task_metrics
                .and_then(|m| m.get("by_prompt"))
                .and_then(|v| v.as_object())
                .map(|map| {
                    map.keys()
                        .cloned()
                        .collect::<std::collections::BTreeSet<_>>()
                });
        }
    }

    let llm_call_count =
        journal_llm_call_count.unwrap_or_else(|| state.task_llm_call_count(task_id));
    let prompt_sources_invoked = journal_prompt_sources_invoked.unwrap_or_else(|| {
        state
            .task_llm_by_prompt(task_id)
            .into_keys()
            .collect::<std::collections::BTreeSet<_>>()
    });

    Ok(ReplayOutcome {
        task_id: task_id.to_string(),
        task_status,
        error_text,
        final_answer_text,
        final_status,
        fallback_source,
        llm_call_count,
        prompt_sources_invoked,
    })
}

/// §7.5 Step 4.b.2.6：纯函数比对器。返回 `Vec<String>` 失败说明（每条对应
/// 一个未达预期的断言）；空 Vec 表示所有断言通过。
///
/// **失败模式分组**（与 [`ExpectedCase`] 字段一一对应）：
///   * `expected_final_answer_contains` 子串缺失 → 一条失败/缺项；
///   * `expected_final_answer_not_contains` 命中 → 一条失败/出现项；
///   * `expected_llm_call_count` 不等 → 一条失败；
///   * `expected_min_llm_call_count` / `_max_` 越界 → 各一条；
///   * `expected_prompt_sources` 子集关系不满足 → 一条失败/缺项；
///   * `expected_fallback_source` 不等 → 一条失败；
///   * `expected_final_status` 不等 → 一条失败。
///
/// **特殊处理**：`expected_verifier_verdict` 设了非空值 → **panic**
/// （不是返回失败），见 [`ExpectedCase`] doc：当前 journal 没暴露这个字段，
/// 不能装作"已比对"。如果你确实需要，请先在 `task_journal` 里加结构化字段。
pub(crate) fn diff_outcome_against_expected(
    expected: &ExpectedCase,
    outcome: &ReplayOutcome,
) -> Vec<String> {
    let mut failures = Vec::new();

    if let Some(verdict) = expected.expected_verifier_verdict.as_deref() {
        if !verdict.is_empty() {
            panic!(
                "ExpectedCase {:?} sets expected_verifier_verdict = {:?}, but \
                 task_journal currently does not expose OutputContractVerdict as a \
                 structured field. Refusing to silently skip the assertion. \
                 Either remove the field from expected.json, or first plumb \
                 `output_contract_verdict` into TaskJournal.summary.",
                expected.case, verdict
            );
        }
    }

    let answer = outcome.final_answer_text.as_deref().unwrap_or_default();
    for needle in &expected.expected_final_answer_contains {
        if !answer.contains(needle) {
            failures.push(format!(
                "expected_final_answer_contains: missing substring {:?} in final_answer = {:?}",
                needle, answer
            ));
        }
    }
    for needle in &expected.expected_final_answer_not_contains {
        if answer.contains(needle) {
            failures.push(format!(
                "expected_final_answer_not_contains: forbidden substring {:?} present in \
                 final_answer = {:?}",
                needle, answer
            ));
        }
    }

    if let Some(exact) = expected.expected_llm_call_count {
        if outcome.llm_call_count != exact as u64 {
            failures.push(format!(
                "expected_llm_call_count: expected {exact}, got {}",
                outcome.llm_call_count
            ));
        }
    }
    if let Some(min) = expected.expected_min_llm_call_count {
        if outcome.llm_call_count < min as u64 {
            failures.push(format!(
                "expected_min_llm_call_count: expected >= {min}, got {}",
                outcome.llm_call_count
            ));
        }
    }
    if let Some(max) = expected.expected_max_llm_call_count {
        if outcome.llm_call_count > max as u64 {
            failures.push(format!(
                "expected_max_llm_call_count: expected <= {max}, got {}",
                outcome.llm_call_count
            ));
        }
    }

    for label in &expected.expected_prompt_sources {
        if !outcome.prompt_sources_invoked.contains(label) {
            failures.push(format!(
                "expected_prompt_sources: label {:?} was not invoked. Invoked set = {:?}",
                label, outcome.prompt_sources_invoked
            ));
        }
    }

    if let Some(want) = expected.expected_fallback_source.as_deref() {
        let got = outcome.fallback_source.as_deref().unwrap_or("<none>");
        if got != want {
            failures.push(format!(
                "expected_fallback_source: expected {:?}, got {:?}",
                want, got
            ));
        }
    }

    if let Some(want) = expected.expected_final_status.as_deref() {
        let got = outcome.final_status.as_deref().unwrap_or("<none>");
        if got != want {
            failures.push(format!(
                "expected_final_status: expected {:?}, got {:?}",
                want, got
            ));
        }
    }

    failures
}

/// §7.5 Step 4.b.2.6：跑一条 `<case>/calls.jsonl` + `<case>/expected.json`
/// 端到端 case，调 [`crate::worker::process_ask_task`] 后返回失败说明。
///
/// **协作约定（caller 必持）**：
///   1. 调用前必须握住 [`fixture_env_lock`]，因为 [`FixtureEnvGuard`] 改的是
///      进程级 env；
///   2. 调用前应当 [`crate::providers::fixture_replay::clear_cache_for_test`]，
///      否则上一条 case 残留的 hash → response map 会幽灵命中本 case；
///   3. 本函数会自己 install / drop `FixtureEnvGuard`。
///
/// **失败语义**：
///   * fixture 文件缺失（`calls.jsonl` 或 `expected.json`）→ `Err(说明)`；
///   * `process_ask_task` 自己返 `Err` → `Err(说明)`；
///   * 抽取 outcome 失败（DB 读失败 / result_json 非法）→ `Err(说明)`；
///   * 比对失败（断言不通过）→ `Ok(Vec<String>)`，每条对应一个未达预期断言；
///   * 全部通过 → `Ok(空 Vec)`。
pub(crate) async fn run_replay_case(case_name: &str) -> Result<Vec<String>, String> {
    let root = fixture_workspace_root();
    let case_dir = root.join(case_name);
    if !case_dir.is_dir() {
        return Err(format!(
            "fixture case dir not found: {} (did you record this case yet?)",
            case_dir.display()
        ));
    }
    let calls_path = case_dir.join(crate::providers::fixture_replay::FIXTURE_CALLS_FILENAME);
    if !calls_path.is_file() {
        return Err(format!(
            "fixture {} missing in {} (run scripts/regen_fixture.sh)",
            crate::providers::fixture_replay::FIXTURE_CALLS_FILENAME,
            case_dir.display()
        ));
    }
    let expected = ExpectedCase::load_for_case(&case_dir)
        .map_err(|e| format!("load expected.json failed: {e}"))?
        .ok_or_else(|| {
            format!(
                "case {case_name:?} has no expected.json — required for e2e harness; \
                 use only calls.jsonl if you want a smoke fixture instead"
            )
        })?;

    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry()
        .with_real_runtime_policy()
        .with_seeded_db_schema();

    {
        let user_key = format!("anon:{}:{}", expected.user_id, expected.chat_id);
        for (idx, prior_turn) in expected.prior_turns.iter().enumerate() {
            let prior_task_id = format!("fixture-history-{case_name}-{idx}");
            let payload_text =
                build_fixture_ask_payload_json(&prior_turn.user_text, &prior_task_id);
            let result_json =
                build_fixture_result_json(&prior_turn.assistant_text, &prior_turn.final_status);
            state
                .core
                .db
                .get()
                .map_err(|e| format!("acquire main-db conn for prior_turns[{idx}] seed: {e}"))?
                .execute(
                    "INSERT INTO tasks (task_id, user_id, chat_id, user_key, channel, kind, payload_json, status, result_json, error_text, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, 'ui', 'ask', ?5, 'succeeded', ?6, NULL, ?7, ?7)",
                    rusqlite::params![
                        prior_task_id,
                        expected.user_id,
                        expected.chat_id,
                        user_key,
                        payload_text,
                        result_json,
                        prior_turn.updated_at,
                    ],
                )
                .map_err(|e| format!("seed prior_turns[{idx}] failed: {e}"))?;

            crate::memory::service::insert_memory(
                &state,
                expected.user_id,
                expected.chat_id,
                Some(&user_key),
                "ui",
                None,
                crate::memory::MEMORY_ROLE_USER,
                &prior_turn.user_text,
                state.policy.memory.item_max_chars.max(256),
            )
            .map_err(|e| format!("seed prior_turns[{idx}] user memory failed: {e}"))?;

            let assistant_memory_text = if state.policy.memory.mark_llm_reply_in_short_term {
                format!(
                    "{}{}",
                    crate::memory::LLM_SHORT_TERM_MEMORY_PREFIX,
                    prior_turn.assistant_text
                )
            } else {
                prior_turn.assistant_text.clone()
            };
            crate::memory::service::insert_memory_with_kind(
                &state,
                expected.user_id,
                expected.chat_id,
                Some(&user_key),
                "ui",
                None,
                crate::memory::MEMORY_ROLE_ASSISTANT,
                &assistant_memory_text,
                state.policy.memory.item_max_chars.max(256),
                crate::memory::MemoryWriteKind::AssistantOutcome,
            )
            .map_err(|e| format!("seed prior_turns[{idx}] assistant memory failed: {e}"))?;
        }
    }

    let task_id = format!("fixture-replay-{}-{}", case_name, uuid::Uuid::new_v4());
    let payload_text = build_fixture_ask_payload_json(&expected.user_text, &task_id);
    state.seed_ask_task_row(&task_id, expected.user_id, expected.chat_id, &payload_text);

    let user_key = format!("anon:{}:{}", expected.user_id, expected.chat_id);
    let task = crate::ClaimedTask {
        task_id: task_id.clone(),
        user_id: expected.user_id,
        chat_id: expected.chat_id,
        user_key: Some(user_key),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: payload_text,
    };
    // claim 路径在生产里会把 status 从 'queued' 切到 'running'；fixture seed 后
    // 直接驱动 process_ask_task 不走 claim_next_task —— 手工把行切到 'running'。
    state
        .core
        .db
        .get()
        .map_err(|e| format!("acquire main-db conn for mark_running: {e}"))?
        .execute(
            "UPDATE tasks SET status = 'running', updated_at = ?2 WHERE task_id = ?1 AND status = 'queued'",
            rusqlite::params![task_id, crate::now_ts()],
        )
        .map_err(|e| format!("mark task running failed: {e}"))?;

    let _env = FixtureEnvGuard::install(&root, case_name, &expected.freeze_now);

    let mut payload_for_process = serde_json::from_str::<serde_json::Value>(&task.payload_json)
        .map_err(|e| format!("payload_json reparse: {e}"))?;
    crate::worker::process_ask_task(&state, &task, &mut payload_for_process)
        .await
        .map_err(|e| format!("process_ask_task returned Err: {e:?}"))?;

    let outcome = extract_outcome_from_state(&state, &task_id)?;
    Ok(diff_outcome_against_expected(&expected, &outcome))
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
            make_verbose(
                &hash_a,
                prompt_a,
                "intent_normalizer",
                "{\"intent\":\"ask\"}"
            ),
            slim_line,
            errored,
            make_verbose(&hash_b, prompt_b, "chat_response", "Hello, world!"),
        );

        // 2) Convert 出 RecordedCall；只应留 2 条。
        let recs = convert_model_io_log_to_fixture(&log).expect("convert ok");
        assert_eq!(recs.len(), 2, "verbose+ok 行有 2 条，其它必须被过滤");
        let hashes: std::collections::HashSet<_> =
            recs.iter().map(|r| r.prompt_hash.clone()).collect();
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
            .call(
                runtime.clone(),
                prompt_a.to_string(),
                ChatRequestHints::default(),
            )
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
        let log_path = std::env::var(LOG_ENV)
            .unwrap_or_else(|_| panic!("{LOG_ENV} env required (path to model_io.log to convert)"));
        let truthy = |v: String| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        };
        let force = std::env::var(FORCE_ENV).map(truthy).unwrap_or(false);
        let dry_run = std::env::var(DRY_ENV).map(truthy).unwrap_or(false);

        let log_text = std::fs::read_to_string(&log_path)
            .unwrap_or_else(|e| panic!("read log file {log_path:?} failed: {e}"));
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
            let runtime = build_fixture_replay_runtime(&format!("vendor-fixture-{case}"));
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
        assert_eq!(
            providers[0].config.provider_type,
            FIXTURE_REPLAY_PROVIDER_TYPE
        );

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

        let state =
            crate::AppState::test_default_with_fixture_provider().with_minimal_builtin_registry();

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

    #[test]
    fn step4b2_1b_real_runtime_policy_aligns_prompt_vendor_and_runtime_payloads() {
        let state = crate::AppState::test_default_with_fixture_provider()
            .with_prompt_layers_installed()
            .with_real_runtime_policy();

        assert_eq!(
            crate::active_prompt_vendor_name(&state),
            "minimax",
            "fixture replay should reuse the real selected vendor so layered prompt patches match recorded prompts"
        );
        assert!(
            !state.policy.persona_prompt_string().trim().is_empty(),
            "real runtime policy must load persona prompt text"
        );
        assert!(
            !state
                .policy
                .schedule
                .intent_rules_template_string()
                .trim()
                .is_empty(),
            "real runtime policy must load schedule rules template"
        );
        assert_eq!(
            state.skill_rt.default_locator_search_dir, state.skill_rt.workspace_root,
            "routing.default_locator_search_dir='.' should resolve to workspace root in test too"
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

    /// §7.5 Step 4.b.2.5 自检（schema + loader）：
    ///   * `_example/expected.json` 能 deserialize + cross-check 通过；
    ///   * 字段缺省 (`user_id` / `chat_id` 默认 1 / 1) 在 ExpectedCase 上正确；
    ///   * `case` 字段与目录名不符 → 报错；
    ///   * `freeze_now` 空 → 报错；
    ///   * `expected_llm_call_count` 与区间约束矛盾 → 报错；
    ///   * `deny_unknown_fields`：unknown key → 报错。
    ///
    /// 不调 `process_ask_task`，只验证 schema/loader 自身契约 —— 真 e2e 留给
    /// `e2e_per_case_replay_with_process_ask_task`。
    #[test]
    fn step4b2_5_self_check_expected_json_schema_and_loader() {
        let example_dir = fixture_workspace_root().join("_example");
        assert!(
            example_dir.is_dir(),
            "expected `_example/` dir to ship as documentation: {}",
            example_dir.display()
        );
        let parsed = ExpectedCase::load_for_case(&example_dir)
            .expect("`_example/expected.json` must parse + cross-check cleanly")
            .expect("`_example/expected.json` must exist (it is the documented sample)");
        assert_eq!(parsed.case, "_example");
        assert_eq!(parsed.user_text, "ping");
        assert_eq!(parsed.user_id, 1, "default user_id must be 1");
        assert_eq!(parsed.chat_id, 1, "default chat_id must be 1");
        assert!(
            parsed.prior_turns.is_empty(),
            "_example should stay the smallest possible single-turn fixture"
        );
        assert_eq!(parsed.expected_final_answer_contains, vec!["pong"]);
        assert_eq!(parsed.expected_min_llm_call_count, Some(1));
        assert_eq!(parsed.expected_max_llm_call_count, Some(4));
        assert_eq!(parsed.expected_final_status.as_deref(), Some("success"));

        // 反例 1：case 字段与目录名不符 → 报错。
        let tmp = std::env::temp_dir().join(format!(
            "rustclaw_test_expected_case_mismatch_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join(FIXTURE_EXPECTED_FILENAME),
            r#"{"case":"WRONG","user_text":"u","freeze_now":"2026-04-19T12:00:00+08:00"}"#,
        )
        .unwrap();
        let err =
            ExpectedCase::load_for_case(&tmp).expect_err("case-name mismatch must Err, not Ok");
        assert!(
            err.contains("WRONG") && err.contains("validate"),
            "err must name the offending case + the validation step. Got: {err}"
        );
        let _ = std::fs::remove_dir_all(&tmp);

        // 反例 2：freeze_now 空 → 报错。
        let tmp = std::env::temp_dir().join(format!(
            "rustclaw_test_expected_freeze_now_empty_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let dir_name = tmp.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            tmp.join(FIXTURE_EXPECTED_FILENAME),
            format!(r#"{{"case":"{dir_name}","user_text":"u","freeze_now":""}}"#),
        )
        .unwrap();
        let err = ExpectedCase::load_for_case(&tmp).expect_err("empty freeze_now must Err");
        assert!(
            err.contains("freeze_now"),
            "err must name the offending field. Got: {err}"
        );
        let _ = std::fs::remove_dir_all(&tmp);

        // 反例 3：prior_turns.updated_at 空 → 报错。
        let tmp = std::env::temp_dir().join(format!(
            "rustclaw_test_expected_prior_turn_updated_at_empty_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let dir_name = tmp.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            tmp.join(FIXTURE_EXPECTED_FILENAME),
            format!(
                r#"{{
                    "case":"{dir_name}",
                    "user_text":"u",
                    "freeze_now":"2026-04-19T12:00:00+08:00",
                    "prior_turns":[{{"user_text":"p1","assistant_text":"a1","updated_at":""}}]
                }}"#
            ),
        )
        .unwrap();
        let err =
            ExpectedCase::load_for_case(&tmp).expect_err("empty prior_turn updated_at must Err");
        assert!(
            err.contains("prior_turns[0].updated_at"),
            "err must name the offending nested field. Got: {err}"
        );
        let _ = std::fs::remove_dir_all(&tmp);

        // 反例 4：expected_llm_call_count 与 max 矛盾 → 报错。
        let tmp = std::env::temp_dir().join(format!(
            "rustclaw_test_expected_count_conflict_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let dir_name = tmp.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            tmp.join(FIXTURE_EXPECTED_FILENAME),
            format!(
                r#"{{"case":"{dir_name}","user_text":"u","freeze_now":"2026-04-19T12:00:00+08:00","expected_llm_call_count":10,"expected_max_llm_call_count":5}}"#
            ),
        )
        .unwrap();
        let err = ExpectedCase::load_for_case(&tmp).expect_err("exact > max must Err");
        assert!(
            err.contains("expected_llm_call_count") && err.contains("expected_max_llm_call_count"),
            "err must name both bounds. Got: {err}"
        );
        let _ = std::fs::remove_dir_all(&tmp);

        // 反例 5：unknown field → deny_unknown_fields trip → parse 失败。
        let tmp = std::env::temp_dir().join(format!(
            "rustclaw_test_expected_unknown_field_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let dir_name = tmp.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            tmp.join(FIXTURE_EXPECTED_FILENAME),
            format!(
                r#"{{"case":"{dir_name}","user_text":"u","freeze_now":"2026-04-19T12:00:00+08:00","totally_unrelated_typo":42}}"#
            ),
        )
        .unwrap();
        let err = ExpectedCase::load_for_case(&tmp)
            .expect_err("unknown field must Err thanks to deny_unknown_fields");
        assert!(
            err.contains("totally_unrelated_typo") || err.contains("unknown field"),
            "err must report the typo'd field. Got: {err}"
        );
        let _ = std::fs::remove_dir_all(&tmp);

        // 反例 5：fixture root 上其他真 case 目录里若有 expected.json，必须全部通过 cross-check。
        // 这一条是"未来扩 case 时的 regression 网"：任何人加新 case 误把
        // `case` 字段写错或字段拼错，本测试必跑挂。
        let real_root = fixture_workspace_root();
        if real_root.is_dir() {
            for entry in std::fs::read_dir(&real_root).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if name.starts_with('.') {
                    continue;
                }
                ExpectedCase::load_for_case(&path).unwrap_or_else(|e| {
                    panic!(
                        "expected.json in {} failed to parse / validate: {e}\n\
                         (fix the file, or delete it if the case has no business assertions yet)",
                        path.display()
                    )
                });
            }
        }
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
    ///   * 4.b.2.4（本文件 `step4b2_4_self_check_seeded_db_schema_and_task_row_round_trip`）：
    ///     [`AppState::with_seeded_db_schema`] 链式 helper 在 `core.db` 上跑
    ///     `INIT_SQL` + `ensure_memory_schema` + `ensure_channel_schema`；
    ///     [`AppState::seed_ask_task_row`] 普通方法 INSERT 一条 `tasks` 行。
    ///     先调研澄清：`migrations/001_init.sql` 不含 FK，`tasks` 行不在不会
    ///     报错而是让所有 `UPDATE tasks ... WHERE task_id = ?` 静默 no-op，
    ///     下游 `mark_running` / `record_result` 全部失效 → 必须 seed。
    ///   * 4.b.2.5（本文件 `step4b2_5_self_check_expected_json_schema_and_loader`）：
    ///     [`ExpectedCase`] schema + [`ExpectedCase::load_for_case`] loader +
    ///     fixture root 下 `README.md` / `_example/{calls.jsonl,expected.json}`
    ///     一份可机器校验的样例。`deny_unknown_fields` + 目录名 cross-check
    ///     + LLM count 区间一致性 + freeze_now 非空都已落入自检。
    ///   * 4.b.2.6.a（本测试 body + 本文件三条
    ///     `step4b2_6_self_check_*` 同步测试）：harness 真调
    ///     `process_ask_task` 跑通整条 wiring，并通过
    ///     [`super::diff_outcome_against_expected`] 把 ExpectedCase 与
    ///     [`super::ReplayOutcome`] 比对。`extract_outcome_from_state` /
    ///     `diff_outcome_against_expected` / `run_replay_case` 三段拆出来
    ///     便于纯函数单测。
    ///
    /// 仍待补的剩余工程（4.b.2.6.b，需用户在本地有真 LLM key 的环境完成）：
    ///   1. 用 `scripts/regen_fixture.sh` 按 planner-first / no-fast-path 当前拓扑
    ///      重新录制真 case 的 `calls.jsonl`，并同步 `expected.json`；
    ///   2. 删掉本测试的 `#[ignore]` —— 本测试 body 已经能在有真 fixture 时
    ///      原地启用，无需再改代码。
    ///
    /// 当前仓库里的旧 fixture 是 prompt/流程重构前录制的，尤其是部分 case
    /// 记录了 fast-path/bypass-normalizer 拓扑。继续默认启用会让 `cargo test`
    /// 被过期录制误报阻塞，所以重录前保持 ignored。
    ///
    /// **本测试 body 行为**：
    ///   * 调 [`super::load_recorded_cases`] 拿到所有非 `_*` 真 case 目录；
    ///   * 跳过没有 `expected.json` 的 case（视为 smoke fixture，由 §Step 2.b
    ///     的 self-check 路径已经覆盖）；
    ///   * 对剩下每条 case 调 [`super::run_replay_case`]，把所有 case 的
    ///     失败说明聚合到一条 panic 里（避免一条挂掉就掩盖后面的）；
    ///   * 0 条真 case 但被显式要求跑（`--ignored`）→ panic 提示用户先录。
    #[tokio::test]
    #[ignore = "LLM replay fixtures are stale after planner-first/no-fast-path prompt topology changes; re-record before enabling"]
    async fn e2e_per_case_replay_with_process_ask_task() {
        let _lock = fixture_env_lock();

        let cases = load_recorded_cases().expect("scan fixture root");
        let with_expected: Vec<_> = cases
            .into_iter()
            .filter_map(|case| {
                let case_dir = case.calls_path.parent().unwrap().to_path_buf();
                let has_expected = case_dir.join(FIXTURE_EXPECTED_FILENAME).is_file();
                has_expected.then_some(case)
            })
            .collect();

        assert!(
            !with_expected.is_empty(),
            "Step 4.b.2.6.b 入口：fixture root 下没有任何带 expected.json 的真 case。\
             请先用 scripts/regen_fixture.sh 录至少 1 条 case，配套写 expected.json，\
             再去掉本测试的 #[ignore] 跑 `cargo test e2e_per_case_replay_with_process_ask_task`。"
        );

        let mut all_failures = Vec::new();
        for case in &with_expected {
            crate::providers::fixture_replay::clear_cache_for_test();
            match run_replay_case(&case.case).await {
                Ok(case_failures) if case_failures.is_empty() => {}
                Ok(case_failures) => {
                    all_failures.push(format!(
                        "[case {}] {} assertion(s) failed:\n  - {}",
                        case.case,
                        case_failures.len(),
                        case_failures.join("\n  - "),
                    ));
                }
                Err(err) => {
                    all_failures.push(format!("[case {}] harness Err: {err}", case.case));
                }
            }
        }

        assert!(
            all_failures.is_empty(),
            "fixture-replay e2e harness saw {} failing case(s):\n\n{}",
            all_failures.len(),
            all_failures.join("\n\n"),
        );
    }

    /// §7.5 Step 4.b.2.6.a self-check：[`super::extract_outcome_from_state`]
    /// 真能从一条手工塞进 `tasks.result_json` 的 journal envelope 里抽出
    /// `final_answer` / `final_status` / `finalizer_summary.fallback`。
    ///
    /// 不调 `process_ask_task` —— 走"建 schema + INSERT 一条 succeeded 行 +
    /// 把假的 result_json 文本写进去 + 调 extract" 路径，用真 schema 验
    /// 真抽取逻辑（与 production 字段路径同步：`task_journal.summary.*`）。
    #[tokio::test]
    async fn step4b2_6_self_check_extract_outcome_reads_journal_envelope_from_result_json() {
        let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task_id = "step4b2_6_extract_round_trip";
        state.seed_ask_task_row(task_id, 7, 7, "{}");

        let result_json = serde_json::json!({
            "result": "ignored-by-extract",
            "task_journal": {
                "summary": {
                    "final_answer": "answer-payload-from-fixture",
                    "final_status": "success",
                    "finalizer_summary": {
                        "fallback": "raw_text",
                        "stage": null,
                        "disposition": null,
                    },
                },
                "trace": {},
            },
        })
        .to_string();
        state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE tasks SET status = 'succeeded', result_json = ?2, updated_at = '0' \
                 WHERE task_id = ?1",
                rusqlite::params![task_id, result_json],
            )
            .unwrap();

        // 同步 metrics 桶（extract 也读这两个）：
        state.note_task_llm_call_with_label(task_id, "normalizer");
        state.note_task_llm_call_with_label(task_id, "chat");
        state.note_task_llm_call_with_label(task_id, "chat");

        let outcome = super::extract_outcome_from_state(&state, task_id).expect("extract ok");
        assert_eq!(outcome.task_id, task_id);
        assert_eq!(outcome.task_status, "succeeded");
        assert!(
            outcome.error_text.is_none(),
            "succeeded path must leave error_text NULL, got {:?}",
            outcome.error_text
        );
        assert_eq!(
            outcome.final_answer_text.as_deref(),
            Some("answer-payload-from-fixture")
        );
        assert_eq!(outcome.final_status.as_deref(), Some("success"));
        assert_eq!(outcome.fallback_source.as_deref(), Some("raw_text"));
        assert_eq!(outcome.llm_call_count, 3);
        assert_eq!(
            outcome.prompt_sources_invoked,
            ["chat", "normalizer"]
                .iter()
                .map(|s| s.to_string())
                .collect::<std::collections::BTreeSet<_>>()
        );
    }

    /// §7.5 Step 4.b.2.6.a self-check：所有断言路径都通过时
    /// [`super::diff_outcome_against_expected`] 返回空 Vec。覆盖每个 schema
    /// 字段的"匹配"分支。
    #[test]
    fn step4b2_6_self_check_diff_outcome_passes_when_all_expected_match() {
        let expected = ExpectedCase {
            case: "_test".to_string(),
            description: None,
            user_text: "hi".to_string(),
            freeze_now: "2026-04-19T12:00:00+08:00".to_string(),
            user_id: 1,
            chat_id: 1,
            prior_turns: vec![],
            expected_final_answer_contains: vec!["pong".to_string()],
            expected_final_answer_not_contains: vec!["error".to_string()],
            expected_llm_call_count: Some(2),
            expected_min_llm_call_count: Some(1),
            expected_max_llm_call_count: Some(3),
            expected_prompt_sources: vec!["normalizer".to_string(), "chat".to_string()],
            expected_fallback_source: Some("raw_text".to_string()),
            expected_verifier_verdict: None,
            expected_final_status: Some("success".to_string()),
        };
        let outcome = super::ReplayOutcome {
            task_id: "t".to_string(),
            task_status: "succeeded".to_string(),
            error_text: None,
            final_answer_text: Some("ok pong here".to_string()),
            final_status: Some("success".to_string()),
            fallback_source: Some("raw_text".to_string()),
            llm_call_count: 2,
            prompt_sources_invoked: ["normalizer", "chat", "verifier"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        let failures = super::diff_outcome_against_expected(&expected, &outcome);
        assert!(
            failures.is_empty(),
            "expected all assertions to pass, got: {failures:?}"
        );
    }

    /// §7.5 Step 4.b.2.6.a self-check：每种约束违反**都**进 failures Vec —— 一条挂
    /// 不会掩盖另一条。这是 harness 的"诊断面板"语义：测试一次跑完报全部问题。
    #[test]
    fn step4b2_6_self_check_diff_outcome_reports_all_violations_at_once() {
        let expected = ExpectedCase {
            case: "_test".to_string(),
            description: None,
            user_text: "hi".to_string(),
            freeze_now: "2026-04-19T12:00:00+08:00".to_string(),
            user_id: 1,
            chat_id: 1,
            prior_turns: vec![],
            expected_final_answer_contains: vec!["pong".to_string()],
            expected_final_answer_not_contains: vec!["error".to_string()],
            expected_llm_call_count: Some(5),
            expected_min_llm_call_count: Some(4),
            expected_max_llm_call_count: Some(2),
            expected_prompt_sources: vec!["chat".to_string(), "verifier".to_string()],
            expected_fallback_source: Some("raw_text".to_string()),
            expected_verifier_verdict: None,
            expected_final_status: Some("success".to_string()),
        };
        let outcome = super::ReplayOutcome {
            task_id: "t".to_string(),
            task_status: "failed".to_string(),
            error_text: None,
            final_answer_text: Some("got an error here".to_string()),
            final_status: Some("failure".to_string()),
            fallback_source: Some("no_answer_parse_failed".to_string()),
            llm_call_count: 3,
            prompt_sources_invoked: ["normalizer"].iter().map(|s| s.to_string()).collect(),
        };
        let failures = super::diff_outcome_against_expected(&expected, &outcome);
        let joined = failures.join("\n");

        for needle in [
            "expected_final_answer_contains",
            "expected_final_answer_not_contains",
            "expected_llm_call_count",
            "expected_min_llm_call_count",
            "expected_max_llm_call_count",
            "expected_prompt_sources",
            "expected_fallback_source",
            "expected_final_status",
        ] {
            assert!(
                joined.contains(needle),
                "diff_outcome must surface a {needle:?} failure; got: {joined}"
            );
        }
        assert!(
            failures.len() >= 8,
            "expected at least one failure per violated field, got {} (joined = {joined})",
            failures.len()
        );
    }

    /// §7.5 Step 4.b.2.6.a self-check：当 `expected_verifier_verdict` 设了非空值
    /// 而 journal 还没暴露结构化字段时，[`super::diff_outcome_against_expected`]
    /// 必须 **panic** 而不是返回失败 —— 避免被误以为"已比对但通过"。
    #[test]
    #[should_panic(expected = "expected_verifier_verdict")]
    fn step4b2_6_self_check_diff_outcome_panics_on_unsupported_verifier_verdict() {
        let expected = ExpectedCase {
            case: "_test".to_string(),
            description: None,
            user_text: "hi".to_string(),
            freeze_now: "2026-04-19T12:00:00+08:00".to_string(),
            user_id: 1,
            chat_id: 1,
            prior_turns: vec![],
            expected_final_answer_contains: vec![],
            expected_final_answer_not_contains: vec![],
            expected_llm_call_count: None,
            expected_min_llm_call_count: None,
            expected_max_llm_call_count: None,
            expected_prompt_sources: vec![],
            expected_fallback_source: None,
            expected_verifier_verdict: Some("pass".to_string()),
            expected_final_status: None,
        };
        let outcome = super::ReplayOutcome {
            task_id: "t".to_string(),
            task_status: "succeeded".to_string(),
            error_text: None,
            final_answer_text: Some("anything".to_string()),
            final_status: Some("success".to_string()),
            fallback_source: None,
            llm_call_count: 0,
            prompt_sources_invoked: Default::default(),
        };
        let _ = super::diff_outcome_against_expected(&expected, &outcome);
    }

    /// §7.5 Step 4.b.2.4 自检：
    ///   * `AppState::with_seeded_db_schema` 真的在 `core.db` 建出 `tasks` /
    ///     `users` / `memories` / `scheduled_jobs` 等基础表（用 `sqlite_master`
    ///     探测）；
    ///   * `AppState::seed_ask_task_row` INSERT 后能被 `crate::repo::tasks`
    ///     的 `UPDATE tasks ... WHERE task_id = ?` 命中（行数 > 0）；
    ///   * 同一 task_id 二次 seed 应 panic（PK 冲突），保证不会被悄悄"双
    ///     seed"误用。
    ///
    /// 不调 `process_ask_task`，只验证 helper 自身契约 —— 真 e2e 留给被
    /// `#[ignore]` 标记的 `e2e_per_case_replay_with_process_ask_task`。
    #[tokio::test]
    async fn step4b2_4_self_check_seeded_db_schema_and_task_row_round_trip() {
        let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();

        {
            let conn = state.core.db.get().expect("acquire main-db conn");
            let table_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN \
                     ('users', 'tasks', 'audit_logs', 'memories', 'long_term_memories', 'scheduled_jobs')",
                    [],
                    |r| r.get(0),
                )
                .expect("count base tables");
            assert_eq!(
                table_count, 6,
                "with_seeded_db_schema must create all 6 base tables, got {table_count}"
            );

            let mut stmt = conn
                .prepare("PRAGMA table_info(tasks)")
                .expect("prep PRAGMA tasks");
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .expect("PRAGMA query")
                .filter_map(|r| r.ok())
                .collect();
            drop(stmt);
            for required in ["task_id", "user_id", "chat_id", "channel", "status"] {
                assert!(
                    cols.iter().any(|c| c.eq_ignore_ascii_case(required)),
                    "tasks table missing column `{required}` after schema seed; got {cols:?}",
                );
            }
        }

        let task_id = "step4b2_4_round_trip_task_id";
        state.seed_ask_task_row(task_id, 4242, 9090, "{}");

        let row_count: i64 = state
            .core
            .db
            .get()
            .expect("acquire main-db conn")
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE task_id = ?1 AND status = 'queued'",
                rusqlite::params![task_id],
                |r| r.get(0),
            )
            .expect("count seeded task row");
        assert_eq!(
            row_count, 1,
            "seed_ask_task_row must INSERT exactly one queued row for task_id `{task_id}`"
        );

        let updated = state
            .core
            .db
            .get()
            .expect("acquire main-db conn")
            .execute(
                "UPDATE tasks SET status = 'running', updated_at = '0' \
                 WHERE task_id = ?1 AND status = 'queued'",
                rusqlite::params![task_id],
            )
            .expect("dry-run mark_running UPDATE");
        assert_eq!(
            updated, 1,
            "production-style `UPDATE tasks ... WHERE status = 'queued'` must hit the seeded row \
             (rows updated, expected 1)",
        );

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.seed_ask_task_row(task_id, 4242, 9090, "{}");
        }))
        .is_err();
        assert!(
            panicked,
            "seed_ask_task_row must panic on PK collision instead of silently double-seeding \
             (so accidental double seed in a future case fixture surfaces immediately)",
        );
    }
}
