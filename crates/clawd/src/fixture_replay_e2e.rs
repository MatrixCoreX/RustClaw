//! Phase 7 §7.5 Step 2.a/2.b：fixture-replay 端到端 harness。
//!
//! 与 [`crate::providers::fixture_replay`] 单测的区别：
//!   * 单测只覆盖 provider 内部（hash / 缓存 / miss / malformed line）以及
//!     `convert_model_io_log_to_fixture` 转换器逻辑。
//!   * 本模块覆盖 **整条 wiring 链**：
//!       1. `FIXTURE_LLM_ROOT` / `FIXTURE_LLM_CASE` env 切换；
//!       2. 通过 [`crate::providers::client::PROVIDER_IMPLS`] 这条生产分发表
//!          找到 `fixture_replay` provider 并真的调起来；
//!       3. RAII guard 保证：测试 panic / 提前 return 都能把两条 env 还原，
//!          下一条测试不会"幽灵命中"上一条 case；
//!       4. **日志到 fixture 再到回放**闭环：把一条合成的 `model_io.log` 喂给
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
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::providers::fixture_replay::{
    FIXTURE_LLM_CASE_ENV, FIXTURE_LLM_ROOT_ENV, FIXTURE_REPLAY_PROVIDER_TYPE,
};

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

/// 进程级 env guard：构造时 set 两条 env，drop 时清掉。
///
/// `Drop` impl 即使被 panic unwind 触发也会执行（panic = stack unwind 路径），
/// 所以 self-check / e2e 测试不会因为一个失败 case 把 env 残留给下一条。
///
/// **不是线程安全**——所有用到本 guard 的测试必须共享 [`fixture_env_lock`] 序列化。
pub(crate) struct FixtureEnvGuard;

impl FixtureEnvGuard {
    pub(crate) fn install(root: &std::path::Path, case: &str) -> Self {
        std::env::set_var(FIXTURE_LLM_ROOT_ENV, root);
        std::env::set_var(FIXTURE_LLM_CASE_ENV, case);
        Self
    }
}

impl Drop for FixtureEnvGuard {
    fn drop(&mut self) {
        std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
        std::env::remove_var(FIXTURE_LLM_CASE_ENV);
    }
}

/// 全局串行锁：所有 fixture e2e 测试共享，保证两条 env 不会在并行测试间撕裂。
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
///     集（HashMap，无调用顺序）。允许值：`context_compaction` / `plan` /
///     `plan_repair` / `delivery_classifier` / `direct_classifier` /
///     `observed` / `user_response_composer` / `user_response_validator` /
///     `clarify` / `schedule` / `nl2cmd` / `memory` / `verifier` / `chat` /
///     `semantic_judge` / `other`。
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

    let _env = FixtureEnvGuard::install(&root, case_name);

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
#[path = "fixture_replay_e2e_tests.rs"]
mod tests;
