//! Phase 7 §7.5 Step 1：fixture 回放 LLM provider。
//!
//! 用途：让 `cargo test --test intent_to_finalize_replay` 等离线测试在
//! 不发任何真实 HTTP 请求的前提下，跑完 ask 全管线（normalizer / planner /
//! repair / chat / delivery_classifier / finalize）。
//!
//! 设计要点：
//!   * 通过 `[[llm_providers]] type = "fixture_replay"` 走 [`crate::providers::client::PROVIDER_IMPLS`]
//!     正常分发，不需要修改 `LlmProvider` trait 或 `AppState` 结构。
//!   * 由两个进程级 env 控制：
//!       - `RUSTCLAW_FIXTURE_LLM_ROOT`：fixture 根目录（绝对路径）。
//!       - `RUSTCLAW_FIXTURE_CASE`：当前 case 名（root 下的子目录名）。
//!     测试 harness 在调用 `process_ask_task` 前 set，跑完 unset。
//!   * Fixture 文件位于 `<root>/<case>/calls.jsonl`，每行一条 [`RecordedCall`]
//!     JSON：`prompt_hash` 是 prompt 字符串的 [FNV-1a 64-bit] hex（16 字符），
//!     `clean_response` / `raw_response` / `usage` 直接喂给 [`LlmProviderResponse`]。
//!   * 命中：返回录制的 response，`request_payload` 标记 `{"fixture_replay":true,...}`。
//!   * 未命中：fail loud（`ProviderError::non_retryable`），错误信息含 prompt_hash
//!     与 regen 提示，方便定位是 prompt 改了还是 case 没录。
//!
//! 选 FNV-1a 而非 SHA256：避免引入新 crate（sha2/hex），FNV 算法本身固定，跨
//! Rust toolchain 稳定，64-bit 空间在单 case 几十次调用下碰撞概率为 0。如未来
//! fixture 库膨胀到需要更强 hash，再升级到 sha2 + hex 即可（本模块对外 API 不变）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::client::{
    ChatRequestHints, LlmProvider, LlmProviderResponse, LlmUsageSnapshot, ProviderCallFuture,
    ProviderError,
};
use crate::LlmProviderRuntime;

/// Provider type 字符串，与 toml 的 `[[llm_providers]] type = "fixture_replay"` 对齐，
/// 也是 `LlmProvider::name()` 的返回值。整个仓库引用此常量，避免散字符串漂移。
pub(crate) const FIXTURE_REPLAY_PROVIDER_TYPE: &str = "fixture_replay";

/// 测试 harness 通过这个 env 指定 fixture 根目录（必须是绝对路径）。
pub(crate) const FIXTURE_LLM_ROOT_ENV: &str = "RUSTCLAW_FIXTURE_LLM_ROOT";

/// 测试 harness 通过这个 env 指定当前 case 名（root 下的子目录）。
pub(crate) const FIXTURE_LLM_CASE_ENV: &str = "RUSTCLAW_FIXTURE_CASE";
/// 诊断开关：命中 / miss 时把当前 prompt hash 与前缀打印到 stderr。
pub(crate) const FIXTURE_LLM_DEBUG_ENV: &str = "RUSTCLAW_FIXTURE_DEBUG";
/// 测试专用兼容开关：hash miss 时按 calls.jsonl 录制顺序回放。
///
/// 默认关闭，保证普通 fixture 测试仍然 fail-loud。E2E replay 可显式开启它，
/// 用来承受 prompt 文案轻微演进，而不把宽松回放扩散到生产或普通单测。
pub(crate) const FIXTURE_LLM_SEQUENCE_FALLBACK_ENV: &str = "RUSTCLAW_FIXTURE_SEQUENCE_FALLBACK";

/// Fixture 文件名（位于 `<root>/<case>/<file>`）。
pub(crate) const FIXTURE_CALLS_FILENAME: &str = "calls.jsonl";

/// 每条录制：prompt 的 FNV-1a 64-bit hex hash → response。
///
/// `prompt_source` / `prompt_preview` 仅供人工排查 fixture 文件用，运行期不读。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RecordedCall {
    pub prompt_hash: String,
    #[serde(default)]
    pub prompt_source: Option<String>,
    #[serde(default)]
    pub prompt_preview: Option<String>,
    /// 经 sanitize 之后的 response（与 [`LlmProviderResponse::text`] 对齐）。
    pub clean_response: String,
    /// 协议层原始 body 字符串（与 [`LlmProviderResponse::raw_response`] 对齐）。
    /// 缺失时回放层会用 `clean_response` 兜底。
    #[serde(default)]
    pub raw_response: Option<String>,
    /// usage 快照（可选）。缺失时回放层填 None，不影响主路径。
    #[serde(default)]
    pub usage: Option<LlmUsageSnapshot>,
}

#[derive(Debug)]
struct FixtureTable {
    by_hash: HashMap<String, RecordedCall>,
    ordered: Vec<RecordedCall>,
}

/// 进程级 (root, case) → 已加载的 fixture 表。第一次访问加载并缓存，后续 O(1) 查表。
///
/// 用 `OnceLock` 包 `RwLock<HashMap>` 避免每次 LLM 调用都 disk read；测试间
/// case 切换会以 (root, case) 为 key 区分缓存项。
static FIXTURE_TABLE_CACHE: std::sync::OnceLock<
    RwLock<HashMap<(PathBuf, String), Arc<FixtureTable>>>,
> = std::sync::OnceLock::new();

fn cache() -> &'static RwLock<HashMap<(PathBuf, String), Arc<FixtureTable>>> {
    FIXTURE_TABLE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

static FIXTURE_SEQUENCE_CURSORS: std::sync::OnceLock<RwLock<HashMap<(PathBuf, String), usize>>> =
    std::sync::OnceLock::new();

fn sequence_cursors() -> &'static RwLock<HashMap<(PathBuf, String), usize>> {
    FIXTURE_SEQUENCE_CURSORS.get_or_init(|| RwLock::new(HashMap::new()))
}

fn fixture_debug_enabled() -> bool {
    std::env::var(FIXTURE_LLM_DEBUG_ENV)
        .ok()
        .map(|v| {
            let trimmed = v.trim().to_ascii_lowercase();
            trimmed == "1" || trimmed == "true" || trimmed == "yes"
        })
        .unwrap_or(false)
}

fn fixture_sequence_fallback_enabled() -> bool {
    std::env::var(FIXTURE_LLM_SEQUENCE_FALLBACK_ENV)
        .ok()
        .map(|v| {
            let trimmed = v.trim().to_ascii_lowercase();
            trimmed == "1" || trimmed == "true" || trimmed == "yes"
        })
        .unwrap_or(false)
}

fn debug_prompt_prefix(prompt: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in prompt.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...(truncated)");
            break;
        }
        out.push(ch);
    }
    out.replace('\n', "\\n")
}

/// 跨 toolchain 稳定的 64-bit FNV-1a，输出 16 字符小写 hex。
///
/// Reference: <http://www.isthe.com/chongo/tech/comp/fnv/>
pub(crate) fn fnv1a_64_hex(input: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash: u64 = FNV_OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

/// 从磁盘加载 (root, case) 对应的 calls.jsonl，转成 hash → call 表。
///
/// 失败模式：
///   * 文件不存在 / IO 失败 → `Err(io message)`，调用方包成 ProviderError 抛出。
///   * 个别行解析失败 → 跳过并 warn（保持其他录制可用，避免 1 行坏掉拖整个 case）。
fn load_table_from_disk(root: &PathBuf, case: &str) -> Result<FixtureTable, String> {
    let path = root.join(case).join(FIXTURE_CALLS_FILENAME);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("read fixture {} failed: {e}", path.display()))?;
    let mut by_hash = HashMap::new();
    let mut ordered = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        match serde_json::from_str::<RecordedCall>(trimmed) {
            Ok(rec) => {
                by_hash.insert(rec.prompt_hash.clone(), rec.clone());
                ordered.push(rec);
            }
            Err(err) => {
                tracing::warn!(
                    target: "fixture_replay",
                    "skip malformed fixture line path={} line={} err={err}",
                    path.display(),
                    idx + 1
                );
            }
        }
    }
    Ok(FixtureTable { by_hash, ordered })
}

/// 取一份 (root, case) 对应的录制表 Arc；命中缓存直接返 Arc clone。
fn get_or_load_table(root: PathBuf, case: String) -> Result<Arc<FixtureTable>, String> {
    let key = (root.clone(), case.clone());
    if let Ok(guard) = cache().read() {
        if let Some(table) = guard.get(&key) {
            return Ok(table.clone());
        }
    }
    let loaded = Arc::new(load_table_from_disk(&root, &case)?);
    if let Ok(mut guard) = cache().write() {
        guard.insert(key, loaded.clone());
    }
    Ok(loaded)
}

fn next_sequence_record(
    root: &PathBuf,
    case: &str,
    table: &FixtureTable,
) -> Option<(usize, RecordedCall)> {
    let key = (root.clone(), case.to_string());
    let mut guard = sequence_cursors().write().ok()?;
    let idx = guard.entry(key).or_insert(0);
    let rec = table.ordered.get(*idx).cloned()?;
    let used = *idx;
    *idx += 1;
    Some((used, rec))
}

fn advance_sequence_cursor(root: &PathBuf, case: &str) {
    if !fixture_sequence_fallback_enabled() {
        return;
    }
    if let Ok(mut guard) = sequence_cursors().write() {
        let key = (root.clone(), case.to_string());
        *guard.entry(key).or_insert(0) += 1;
    }
}

/// §7.5 Step 3：[`regen_fixture_from_log`] 的执行摘要。返回给调用方
/// （CLI / `scripts/regen_fixture.sh`）打印，让操作者一眼看清"写了几条 / 写到
/// 哪 / 是不是 dry-run / 是不是覆盖"。
///
/// `#[cfg(test)]`：本类型仅在 `cargo test`（含 `regen_fixture_tool` 这条
/// `--ignored` 入口）路径用，生产 bin 不引用。
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegenSummary {
    /// 写入（或 dry-run 模式下"将写入"）的 record 条数。
    pub written_records: usize,
    /// 目标 fixture 文件绝对路径（`<root>/<case>/calls.jsonl`）。
    pub dest_path: PathBuf,
    /// `true` 时未真正写盘。
    pub dry_run: bool,
    /// `true` 表示目标已存在且本次是覆盖写。
    pub overwrote_existing: bool,
}

/// §7.5 Step 3：从一段 `model_io.log` 文本生成 / 覆盖一个 fixture case 的
/// `calls.jsonl`。`scripts/regen_fixture.sh` 与 env-driven 的 `cargo test`
/// tool entry（[`crate::fixture_replay_e2e::tests::regen_fixture_tool`]）
/// 都从这里入口。
///
/// 流程：
///   1. 调 [`convert_model_io_log_to_fixture`] 把 log 抽成 `Vec<RecordedCall>`；
///   2. 空 → 报错（提示 grep 范围 / `task_id` 过滤可能漏了，避免悄悄写空文件）；
///   3. dest = `<root>/<case>/calls.jsonl`；
///      * 已存在且 `!force` → 报错（避免误覆盖，强制操作者显式 `--force`）；
///      * `dry_run = true` → 不 mkdir、不写盘，只返回 summary；
///      * 否则 mkdir -p + 序列化 records 写入。
///
/// **不**对 records 做任何 reorder / sort —— 保持
/// `convert_model_io_log_to_fixture` 的"首次出现顺序，同 hash 留最后一次值"语义，
/// 让 fixture 文件在不同录制之间 diff 友好。
///
/// `#[cfg(test)]`：本函数仅在 `cargo test`（含 `regen_fixture_tool` env-driven
/// 入口）路径用，生产 bin 不调用，避免在 release 二进制里背一份 IO 落盘逻辑。
#[cfg(test)]
pub(crate) fn regen_fixture_from_log(
    log_text: &str,
    case: &str,
    root: &std::path::Path,
    dry_run: bool,
    force: bool,
) -> Result<RegenSummary, String> {
    if case.trim().is_empty() {
        return Err("regen_fixture_from_log: case name must be non-empty".to_string());
    }
    if case.contains('/') || case.contains('\\') || case.contains("..") {
        return Err(format!(
            "regen_fixture_from_log: case name {case:?} contains path separators or `..`; \
             must be a single directory name under fixture root"
        ));
    }

    let records = convert_model_io_log_to_fixture(log_text)?;
    if records.is_empty() {
        return Err(format!(
            "regen_fixture_from_log: convert produced 0 records — check that the log is \
             grep'd to the right task_id, that `routing.debug_log_prompt = true` was on \
             during recording, and that the run actually completed (status=ok)"
        ));
    }

    let case_dir = root.join(case);
    let dest = case_dir.join(FIXTURE_CALLS_FILENAME);
    let overwrote_existing = dest.exists();
    if overwrote_existing && !force {
        return Err(format!(
            "regen_fixture_from_log: {} already exists; pass force=true (env \
             RUSTCLAW_REGEN_FIXTURE_FORCE=1) to overwrite",
            dest.display()
        ));
    }

    if !dry_run {
        std::fs::create_dir_all(&case_dir).map_err(|e| {
            format!(
                "regen_fixture_from_log: mkdir {} failed: {e}",
                case_dir.display()
            )
        })?;
        let mut body = String::with_capacity(records.len() * 256);
        for rec in &records {
            let line = serde_json::to_string(rec)
                .map_err(|e| format!("regen_fixture_from_log: serialize record failed: {e}"))?;
            body.push_str(&line);
            body.push('\n');
        }
        std::fs::write(&dest, body).map_err(|e| {
            format!(
                "regen_fixture_from_log: write {} failed: {e}",
                dest.display()
            )
        })?;
    }

    Ok(RegenSummary {
        written_records: records.len(),
        dest_path: dest,
        dry_run,
        overwrote_existing,
    })
}

/// 强制清空 (root, case) 缓存。仅供单测用 —— 录制文件改完想立刻看到新值时调用。
#[cfg(test)]
pub(crate) fn clear_cache_for_test() {
    if let Ok(mut guard) = cache().write() {
        guard.clear();
    }
    if let Ok(mut guard) = sequence_cursors().write() {
        guard.clear();
    }
}

/// §7.5 Step 2.b：从 `model_io.log`（JSONL）转出 fixture `Vec<RecordedCall>`。
///
/// **使用前提**：录制时 `routing.debug_log_prompt = true`，否则 verbose 行不写
/// 出来，slim 行没 prompt / response 也没办法回放。
///
/// **截断检测**（convert_* 拒绝两类行）：
///   * 缺 `prompt_hash` 字段 —— 老版本 clawd 写的日志，prompt 可能被截断后无法
///     反算 hash。必须升级到含 §7.5 Step 2.b 的 clawd 重新录制。
///   * `clean_response` 末尾出现 `...(truncated)` —— 响应被
///     [`crate::log_utils::truncate_for_log`] 截到 [`crate::MODEL_IO_LOG_MAX_CHARS`]
///     字符（§7.5 把阈值抬到 128_000，所以正常 chat/normalizer/planner 输出
///     不会再触发；这条仍保留作为"读旧日志"或"未来某天 prompt 又膨胀回来"的
///     fail-loud 兜底，不能让被截断的字符串当 LLM 输出喂下游 parser）。
///
/// **去重策略**：同一个 `prompt_hash` 在日志里出现多次时，**保留最后一次**
/// （最贴近"现网当前行为"）。若需切到 first 策略，调用方可在拿到 Vec 后自己
/// 反向遍历重建。
///
/// **过滤策略**：
///   * 只接受 `"mode": "verbose"`；slim / 缺失 mode 直接跳过。
///   * 只接受 `"status": "ok"`；error / retry 等失败状态跳过（fixture 是"成功
///     case 的录制"，错误路径有专门测试覆盖）。
///   * 空行 / 以 `#` 起头的注释行 / 解析失败的行：跳过，不算错。这与 fixture
///     文件读取语义保持一致（[`load_table_from_disk`]）。
///
/// `#[cfg(test)]`：本函数仅供 [`regen_fixture_from_log`] 与 e2e harness 调用，
/// 生产 bin 不需要 model_io.log → fixture 的转换路径。
#[cfg(test)]
pub(crate) fn convert_model_io_log_to_fixture(log_text: &str) -> Result<Vec<RecordedCall>, String> {
    // 用 Vec 维持首次出现顺序、HashMap 维护"同 hash 覆盖到最后一次"的下标。
    let mut latest_idx: HashMap<String, usize> = HashMap::new();
    let mut records: Vec<RecordedCall> = Vec::new();

    for (idx, line) in log_text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // 与 load_table_from_disk 保持一致：跳过坏行。
        };

        let mode = value.get("mode").and_then(|v| v.as_str()).unwrap_or("");
        if mode != "verbose" {
            continue;
        }
        let status = value.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status != "ok" {
            continue;
        }

        let prompt_hash = value
            .get("prompt_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                format!(
                    "model_io.log line {} has no `prompt_hash` field — record was made with \
                     pre-§7.5 clawd; rerun the case after rebuilding to capture hashes",
                    idx + 1
                )
            })?
            .to_string();

        let prompt_source = value
            .get("prompt_source")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // model_io.log 写的是 truncate_for_log 之后的 prompt，存到 prompt_preview
        // 仅供人工排查用，运行期不参与 hash。
        let prompt_preview = value
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let clean_response = value
            .get("clean_response")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                format!(
                    "model_io.log line {} has no `clean_response` (status=ok requires response)",
                    idx + 1
                )
            })?
            .to_string();
        if clean_response.ends_with("...(truncated)") {
            return Err(format!(
                "model_io.log line {} `clean_response` was truncated by truncate_for_log \
                 (>{} chars). Reduce prompt/response size or split the case before re-recording.",
                idx + 1,
                crate::MODEL_IO_LOG_MAX_CHARS
            ));
        }

        let raw_response = value
            .get("raw_response")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if raw_response
            .as_deref()
            .map(|s| s.ends_with("...(truncated)"))
            .unwrap_or(false)
        {
            return Err(format!(
                "model_io.log line {} `raw_response` was truncated by truncate_for_log \
                 (>{} chars). Reduce response size or split the case.",
                idx + 1,
                crate::MODEL_IO_LOG_MAX_CHARS
            ));
        }

        let usage = value.get("usage").and_then(|v| {
            if v.is_null() {
                None
            } else {
                serde_json::from_value::<LlmUsageSnapshot>(v.clone()).ok()
            }
        });

        let rec = RecordedCall {
            prompt_hash: prompt_hash.clone(),
            prompt_source,
            prompt_preview,
            clean_response,
            raw_response,
            usage,
        };
        if let Some(&existing) = latest_idx.get(&prompt_hash) {
            records[existing] = rec;
        } else {
            latest_idx.insert(prompt_hash, records.len());
            records.push(rec);
        }
    }

    Ok(records)
}

pub(crate) struct FixtureReplayProvider;

/// §7.5 Step 2.a：构造一份 fixture_replay 形态的 [`LlmProviderRuntime`]，仅供
/// in-crate 测试 harness 用。`name` 体现在 `LlmProviderConfig::name`，方便在多
/// provider fallback 链测试里区分（比如 `vendor-fixture-primary` /
/// `vendor-fixture-fallback`）。
///
/// 抽出动机：Step 1 单测 `make_runtime()` 写法在 [`crate::fixture_replay_e2e`] /
/// 未来 e2e harness 里要重复出现，复制粘贴就会漂——一处提供，单点维护。
#[cfg(test)]
pub(crate) fn build_fixture_replay_runtime(name: &str) -> Arc<LlmProviderRuntime> {
    use claw_core::config::{LlmProviderConfig, LlmProviderParams};
    use tokio::sync::Semaphore;

    Arc::new(LlmProviderRuntime {
        config: LlmProviderConfig {
            name: name.to_string(),
            provider_type: FIXTURE_REPLAY_PROVIDER_TYPE.to_string(),
            base_url: "http://fixture.invalid".to_string(),
            api_key: "fixture".to_string(),
            model: "fixture-model".to_string(),
            context_window_tokens: None,
            priority: 1,
            timeout_seconds: 5,
            max_concurrency: 1,
            params: LlmProviderParams::default(),
        },
        client: reqwest::Client::new(),
        semaphore: Arc::new(Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    })
}

impl LlmProvider for FixtureReplayProvider {
    fn name(&self) -> &'static str {
        FIXTURE_REPLAY_PROVIDER_TYPE
    }

    fn call(
        &self,
        _provider: Arc<LlmProviderRuntime>,
        prompt: String,
        _hints: ChatRequestHints,
    ) -> ProviderCallFuture {
        Box::pin(async move {
            let root_str = std::env::var(FIXTURE_LLM_ROOT_ENV).map_err(|_| {
                ProviderError::non_retryable(
                    format!(
                        "fixture_replay provider invoked but {FIXTURE_LLM_ROOT_ENV} env not set"
                    ),
                    json!({ "fixture_replay": true }),
                )
            })?;
            let case = std::env::var(FIXTURE_LLM_CASE_ENV).map_err(|_| {
                ProviderError::non_retryable(
                    format!(
                        "fixture_replay provider invoked but {FIXTURE_LLM_CASE_ENV} env not set"
                    ),
                    json!({ "fixture_replay": true }),
                )
            })?;
            let root = PathBuf::from(root_str);
            let prompt_hash = fnv1a_64_hex(&prompt);
            let table = get_or_load_table(root.clone(), case.clone()).map_err(|e| {
                ProviderError::non_retryable(
                    format!("fixture_replay load failed: {e}"),
                    json!({
                        "fixture_replay": true,
                        "case": case.clone(),
                        "prompt_hash": prompt_hash,
                    }),
                )
            })?;
            match table.by_hash.get(&prompt_hash) {
                Some(rec) => {
                    advance_sequence_cursor(&root, &case);
                    if fixture_debug_enabled() {
                        eprintln!(
                            "[fixture_replay hit] case={case} prompt_hash={prompt_hash} prompt_chars={} prompt_prefix={}",
                            prompt.chars().count(),
                            debug_prompt_prefix(&prompt, 240)
                        );
                    }
                    let raw = rec
                        .raw_response
                        .clone()
                        .unwrap_or_else(|| rec.clean_response.clone());
                    Ok(LlmProviderResponse {
                        text: rec.clean_response.clone(),
                        request_payload: json!({
                            "fixture_replay": true,
                            "case": case,
                            "prompt_hash": prompt_hash,
                            "prompt_source": rec.prompt_source,
                        }),
                        raw_response: raw,
                        usage: rec.usage.clone(),
                    })
                }
                None => {
                    if fixture_sequence_fallback_enabled() {
                        if let Some((sequence_index, rec)) =
                            next_sequence_record(&root, &case, &table)
                        {
                            if fixture_debug_enabled() {
                                eprintln!(
                                    "[fixture_replay sequence_fallback] case={case} sequence_index={sequence_index} actual_prompt_hash={prompt_hash} recorded_prompt_hash={} prompt_chars={} prompt_prefix={}",
                                    rec.prompt_hash,
                                    prompt.chars().count(),
                                    debug_prompt_prefix(&prompt, 240)
                                );
                            }
                            let raw = rec
                                .raw_response
                                .clone()
                                .unwrap_or_else(|| rec.clean_response.clone());
                            return Ok(LlmProviderResponse {
                                text: rec.clean_response.clone(),
                                request_payload: json!({
                                    "fixture_replay": true,
                                    "fixture_replay_sequence_fallback": true,
                                    "case": case,
                                    "prompt_hash": prompt_hash,
                                    "recorded_prompt_hash": rec.prompt_hash,
                                    "sequence_index": sequence_index,
                                    "prompt_source": rec.prompt_source,
                                }),
                                raw_response: raw,
                                usage: rec.usage.clone(),
                            });
                        }
                    }
                    if fixture_debug_enabled() {
                        eprintln!(
                            "[fixture_replay miss] case={case} prompt_hash={prompt_hash} prompt_chars={} prompt_prefix={}",
                            prompt.chars().count(),
                            debug_prompt_prefix(&prompt, 240)
                        );
                    }
                    Err(ProviderError::non_retryable(
                        format!(
                            "fixture_replay miss case={case} prompt_hash={prompt_hash} \
                             prompt_chars={chars} (regen: RUSTCLAW_REGEN_FIXTURE={case} bash scripts/regen_fixture.sh)",
                            chars = prompt.chars().count(),
                        ),
                        json!({
                            "fixture_replay": true,
                            "case": case,
                            "prompt_hash": prompt_hash,
                        }),
                    ))
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "fixture_replay_tests.rs"]
mod tests;
