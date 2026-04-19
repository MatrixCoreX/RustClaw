//! Phase 7 §7.5 Step 1：fixture 回放 LLM provider。
//!
//! 用途：让 `cargo test --test intent_to_finalize_replay` 等离线测试在
//! 不发任何真实 HTTP 请求的前提下，跑完 ask 全管线（normalizer / planner /
//! repair / chat / classifier_direct / finalize）。
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

/// 进程级 (root, case) → 已加载的 hash 表。第一次访问加载并缓存，后续 O(1) 查表。
///
/// 用 `OnceLock` 包 `RwLock<HashMap>` 避免每次 LLM 调用都 disk read；测试间
/// case 切换会以 (root, case) 为 key 区分缓存项。
static FIXTURE_TABLE_CACHE: std::sync::OnceLock<
    RwLock<HashMap<(PathBuf, String), Arc<HashMap<String, RecordedCall>>>>,
> = std::sync::OnceLock::new();

fn cache() -> &'static RwLock<HashMap<(PathBuf, String), Arc<HashMap<String, RecordedCall>>>> {
    FIXTURE_TABLE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
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
fn load_table_from_disk(
    root: &PathBuf,
    case: &str,
) -> Result<HashMap<String, RecordedCall>, String> {
    let path = root.join(case).join(FIXTURE_CALLS_FILENAME);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("read fixture {} failed: {e}", path.display()))?;
    let mut table = HashMap::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        match serde_json::from_str::<RecordedCall>(trimmed) {
            Ok(rec) => {
                table.insert(rec.prompt_hash.clone(), rec);
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
    Ok(table)
}

/// 取一份 (root, case) 对应的录制表 Arc；命中缓存直接返 Arc clone。
fn get_or_load_table(
    root: PathBuf,
    case: String,
) -> Result<Arc<HashMap<String, RecordedCall>>, String> {
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

/// 强制清空 (root, case) 缓存。仅供单测用 —— 录制文件改完想立刻看到新值时调用。
#[cfg(test)]
pub(crate) fn clear_cache_for_test() {
    if let Ok(mut guard) = cache().write() {
        guard.clear();
    }
}

/// §7.5 Step 2.b：从 `model_io.log`（JSONL）转出 fixture `Vec<RecordedCall>`。
///
/// **使用前提**：录制时 `routing.debug_log_prompt = true`，否则 verbose 行不写
/// 出来，slim 行没 prompt / response 也没办法回放。
///
/// **截断检测**（convert_* 拒绝两类行）：
///   * 缺 `prompt_hash` 字段 —— 老版本 clawd 写的日志，prompt 被截断后无法反算
///     hash。必须升级到含 §7.5 Step 2.b 的 clawd 重新录制。
///   * `clean_response` 末尾出现 `...(truncated)` —— 响应被
///     [`crate::log_utils::truncate_for_log`] 截到 [`crate::MODEL_IO_LOG_MAX_CHARS`]
///     字符。回放时把截断后的字符串当 LLM 输出会让下游 parser 在结尾意外失败。
///     这种情况极少（chat/normalizer/planner 的输出一般都 < 5000 字符），如果
///     真的撞上，应在 prompt 端做减肥，不能装作没事。
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
pub(crate) fn convert_model_io_log_to_fixture(
    log_text: &str,
) -> Result<Vec<RecordedCall>, String> {
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

        let usage = value
            .get("usage")
            .and_then(|v| {
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
            let table = get_or_load_table(root, case.clone()).map_err(|e| {
                ProviderError::non_retryable(
                    format!("fixture_replay load failed: {e}"),
                    json!({
                        "fixture_replay": true,
                        "case": case,
                        "prompt_hash": prompt_hash,
                    }),
                )
            })?;
            match table.get(&prompt_hash) {
                Some(rec) => {
                    let raw = rec.raw_response.clone().unwrap_or_else(|| rec.clean_response.clone());
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
                None => Err(ProviderError::non_retryable(
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
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// 进程内 env 串扰隔离锁：本模块所有用 set_var 的测试串行化。
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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
            .call(
                runtime,
                "anything".to_string(),
                ChatRequestHints::default(),
            )
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
        let mut errored = serde_json::from_str::<serde_json::Value>(
            &verbose_ok_line(&hash, prompt, "ignored", "x"),
        )
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
        assert!(err.contains("pre-§7.5"), "err msg should hint upgrade: {err}");
    }

    #[test]
    fn convert_fails_when_response_was_truncated() {
        let prompt = "p";
        let hash = fnv1a_64_hex(prompt);
        let mut v = serde_json::from_str::<serde_json::Value>(&verbose_ok_line(
            &hash, prompt, "ok-base", "x",
        ))
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
            .call(runtime, good_prompt.to_string(), ChatRequestHints::default())
            .await
            .expect("good line should still load");
        assert_eq!(resp.text, "pong");
        assert_eq!(resp.raw_response, "pong", "raw fallback to clean when absent");

        std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
        std::env::remove_var(FIXTURE_LLM_CASE_ENV);
    }
}
