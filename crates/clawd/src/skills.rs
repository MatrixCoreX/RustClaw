use claw_core::skill_registry::{CapabilityIsolationProfile, SkillKind, SkillRiskLevel};
use serde_json::{json, Map, Value};
use std::path::{Component, Path};
use tokio::process::Command;

/// §E2 step1: skill-runner 子进程在 strict 模式下允许从父进程继承的 env 白名单。
///
/// 设计原则：
/// * 只放行子进程**最低运行所必需**的基础设施变量（locale / 临时目录 / TLS 根证书 /
///   PATH 之类），其它一切配置（API key、model、workspace 路径等）必须由 clawd 通过
///   `cmd.env(...)` 显式注入或经 `SecretsBroker` 走 `secrets.<usage>_<vendor>_api_key`
///   契约下来 —— 这才是 §3.4 "secrets 成为唯一渠道" 的真正落地。
/// * 严格模式默认开启；只有显式设置 `RUSTCLAW_SKILL_ENV_STRICT=0|false|off|no`
///   才临时关闭。skill 若依赖未声明的环境变量会立即暴露配置缺口。
/// * 列表保持小而稳：扩列前请先评估能否用 manifest capability 替代。
pub(crate) const SKILL_RUNNER_ENV_WHITELIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TMPDIR",
    "TMP",
    "TEMP",
    "TZ",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

/// Runtime switch for strict child-process environment isolation.
///
/// Isolation is enabled by default. Set `RUSTCLAW_SKILL_ENV_STRICT=0|false|off|no`
/// only as an explicit compatibility escape hatch.
pub(crate) fn skill_runner_env_strict_enabled() -> bool {
    !matches!(
        std::env::var("RUSTCLAW_SKILL_ENV_STRICT")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("0") | Some("false") | Some("off") | Some("no")
    )
}

/// §E2 step1: 在 strict 模式下从一个 source env map 计算应当注入子进程的白名单 env。
///
/// 抽成纯函数便于单元测试 —— `apply_skill_runner_env_isolation` 内部用 `std::env::vars()`
/// 作为 source，但测试时我们想喂一个固定 map 验证白名单语义。
///
/// 返回值已按 key 字典序排序、过滤掉空值（避免把空字符串传下去再触发 fail-loud）。
pub(crate) fn collect_whitelisted_env_pairs<I, K, V>(source: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    use std::collections::BTreeMap;
    let allowed: std::collections::HashSet<&'static str> =
        SKILL_RUNNER_ENV_WHITELIST.iter().copied().collect();
    let mut kept: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in source {
        let key = k.as_ref();
        if !allowed.contains(key) {
            continue;
        }
        let val = v.as_ref();
        if val.is_empty() {
            continue;
        }
        kept.insert(key.to_string(), val.to_string());
    }
    kept.into_iter().collect()
}

/// §E2 step1: 打开 strict 隔离时，把白名单变量原样塞回 `cmd`，并返回剥离 / 保留的统计。
///
/// 只做"清空 + 白名单注入"，不碰任何后续 `.env(K, V)` 调用 —— 那部分仍是 clawd 显式
/// 配置 + broker secrets，是子进程的**唯一**真实 env 来源。
///
/// This applies to every child-process spawn path that opts into this helper, including
/// skill-runner, imported local scripts, Python dependency probes, and builtin `run_cmd`.
/// Pure in-process builtins do not create a child environment.
pub(crate) struct StrictEnvReport {
    pub(crate) preserved: Vec<String>,
    pub(crate) stripped_count: usize,
}

pub(crate) fn apply_skill_runner_env_isolation(cmd: &mut Command) -> Option<StrictEnvReport> {
    if !skill_runner_env_strict_enabled() {
        return None;
    }
    let kept = collect_whitelisted_env_pairs(std::env::vars());
    let total_env = std::env::vars().count();
    cmd.env_clear();
    let preserved: Vec<String> = kept.iter().map(|(k, _)| k.clone()).collect();
    for (k, v) in &kept {
        cmd.env(k, v);
    }
    Some(StrictEnvReport {
        preserved,
        stripped_count: total_env.saturating_sub(kept.len()),
    })
}

#[cfg(unix)]
pub(crate) fn place_subprocess_in_own_process_group(cmd: &mut Command) {
    cmd.process_group(0);
}

#[cfg(not(unix))]
pub(crate) fn place_subprocess_in_own_process_group(_cmd: &mut Command) {}

#[cfg(unix)]
pub(crate) async fn terminate_subprocess_group(pid: Option<u32>) -> bool {
    let Some(pid) = pid.filter(|pid| *pid > 0) else {
        return false;
    };
    Command::new("kill")
        .arg("-KILL")
        .arg(format!("-{pid}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub(crate) async fn terminate_subprocess_group(_pid: Option<u32>) -> bool {
    false
}

mod builtin;
mod external;
mod memory_context;
mod output_dirs;
mod runner;

#[cfg(test)]
pub(crate) use builtin::run_safe_command;
pub(crate) use builtin::{execute_builtin_skill_for_task, run_safe_command_with_sandbox};
// `execute_builtin_skill`（无 task 版本）只在 `builtin.rs` 内部测试用，
// 不再向 crate 外暴露，避免再产生绕过 LLM 预算/日志的调用点。
// 详见 `builtin.rs` 上对 `execute_builtin_skill` 的注释。
pub(crate) use external::execute_external_skill;
pub(crate) use memory_context::inject_skill_memory_context;
pub(crate) use output_dirs::ensure_default_output_dir_for_skill_args;
pub(crate) use runner::{run_skill_with_runner, run_skill_with_runner_once};

use crate::worker::task_runtime_channel;
use crate::{AppState, ClaimedTask, RuntimeChannel};

const READ_FILE_NOT_FOUND_PREFIX: &str = "__RC_READ_FILE_NOT_FOUND__:";
const POLICY_BLOCK_ERROR_PREFIX: &str = "__RC_POLICY_BLOCK__:";
const CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX: &str = "__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:";
const STRUCTURED_SKILL_ERROR_PREFIX: &str = "__RC_SKILL_ERROR__:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicyBlockError {
    pub(crate) decision: String,
    pub(crate) reason_code: String,
    pub(crate) observed_facts: Vec<String>,
    pub(crate) policy_boundary: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StructuredSkillError {
    pub(crate) skill: String,
    pub(crate) error_kind: String,
    pub(crate) error_text: String,
    pub(crate) platform: Option<String>,
    pub(crate) manager_type: Option<String>,
    pub(crate) service_name: Option<String>,
    pub(crate) extra: Option<Value>,
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn child_extra_object(value: &Value) -> Option<&Value> {
    value.get("extra").filter(|extra| extra.is_object())
}

fn structured_skill_error_string(skill: &str, value: &Value) -> String {
    let extra_object = child_extra_object(value);
    let error_kind = string_field(value, "error_kind")
        .or_else(|| extra_object.and_then(|extra| string_field(extra, "error_kind")))
        .unwrap_or_else(|| "unknown".to_string());
    let error_text = string_field(value, "error_text")
        .or_else(|| extra_object.and_then(|extra| string_field(extra, "failure_reason")))
        .unwrap_or_else(|| "skill execution failed".to_string());
    let payload = json!({
        "skill": skill.trim(),
        "error_kind": error_kind,
        "error_text": error_text,
        "platform": string_field(value, "platform")
            .or_else(|| extra_object.and_then(|extra| string_field(extra, "platform"))),
        "manager_type": extra_object.and_then(|extra| string_field(extra, "manager_type")),
        "service_name": extra_object.and_then(|extra| string_field(extra, "service_name")),
        "extra": value.get("extra").cloned().unwrap_or(Value::Null),
        "text": Value::Null,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    format!("{STRUCTURED_SKILL_ERROR_PREFIX}{encoded}")
}

pub(crate) fn structured_skill_error_from_parts(
    skill: &str,
    error_kind: &str,
    error_text: &str,
    platform: Option<&str>,
    extra: Option<Value>,
) -> String {
    let payload = json!({
        "skill": skill.trim(),
        "error_kind": error_kind,
        "error_text": error_text,
        "platform": platform,
        "extra": extra.unwrap_or(Value::Null),
        "text": Value::Null,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    format!("{STRUCTURED_SKILL_ERROR_PREFIX}{encoded}")
}

pub(crate) fn parse_structured_skill_error(err: &str) -> Option<StructuredSkillError> {
    let payload = err.trim().strip_prefix(STRUCTURED_SKILL_ERROR_PREFIX)?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let error_kind = string_field(&value, "error_kind").unwrap_or_else(|| "unknown".to_string());
    let error_text =
        string_field(&value, "error_text").unwrap_or_else(|| "skill execution failed".to_string());
    Some(StructuredSkillError {
        skill: string_field(&value, "skill").unwrap_or_default(),
        error_kind,
        error_text,
        platform: string_field(&value, "platform"),
        manager_type: string_field(&value, "manager_type"),
        service_name: string_field(&value, "service_name"),
        extra: value.get("extra").cloned().filter(|value| !value.is_null()),
    })
}

fn structured_extra_value<'a>(
    structured: &'a StructuredSkillError,
    key: &str,
) -> Option<&'a Value> {
    structured.extra.as_ref()?.get(key)
}

fn structured_extra_string(structured: &StructuredSkillError, key: &str) -> Option<String> {
    structured_extra_value(structured, key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn policy_block_error(
    reason_code: &str,
    observed_facts: Vec<String>,
    policy_boundary: Vec<String>,
) -> String {
    let decision = crate::policy_decision::PolicyDecision::Deny.as_token();
    let payload = json!({
        "decision": decision,
        "reason_code": reason_code.trim(),
        "permission_decision": {
            "decision": decision,
            "denied_by_policy": true,
            "needs_confirmation": false,
            "background_wait": false,
        },
        "observed_facts": observed_facts,
        "policy_boundary": policy_boundary,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    format!("{POLICY_BLOCK_ERROR_PREFIX}{encoded}")
}

pub(crate) fn parse_policy_block_error(err: &str) -> Option<PolicyBlockError> {
    let payload = err.trim().strip_prefix(POLICY_BLOCK_ERROR_PREFIX)?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let reason_code = value
        .get("reason_code")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    let decision = value
        .get("decision")
        .and_then(|v| v.as_str())
        .or_else(|| {
            value
                .get("permission_decision")
                .and_then(|v| v.get("decision"))
                .and_then(|v| v.as_str())
        })
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(crate::policy_decision::PolicyDecision::Deny.as_token())
        .to_string();
    let strings_from_array = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim))
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(PolicyBlockError {
        decision,
        reason_code,
        observed_facts: strings_from_array("observed_facts"),
        policy_boundary: strings_from_array("policy_boundary"),
    })
}

fn policy_block_message_key(reason_code: &str) -> String {
    let normalized = reason_code
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "clawd.msg.policy.unknown".to_string()
    } else {
        format!("clawd.msg.policy.{normalized}")
    }
}

fn policy_observed_facts_value(facts: &[String]) -> Value {
    let mut object = Map::new();
    let mut unparsed = Vec::new();
    for fact in facts {
        let fact = fact.trim();
        if fact.is_empty() {
            continue;
        }
        if let Some((key, value)) = fact.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            if !key.is_empty() && !value.is_empty() {
                object.insert(key.to_string(), json!(value));
                continue;
            }
        }
        unparsed.push(fact.to_string());
    }
    if !unparsed.is_empty() {
        object.insert("unparsed".to_string(), json!(unparsed));
    }
    Value::Object(object)
}

fn policy_block_machine_payload(block: &PolicyBlockError) -> String {
    json!({
        "message_key": policy_block_message_key(&block.reason_code),
        "decision": &block.decision,
        "reason_code": block.reason_code,
        "permission_decision": {
            "decision": &block.decision,
            "denied_by_policy": true,
            "needs_confirmation": false,
            "background_wait": false,
        },
        "observed_facts": policy_observed_facts_value(&block.observed_facts),
        "policy_boundary_count": block.policy_boundary.len(),
    })
    .to_string()
}

fn parse_crypto_account_access_error(err: &str) -> Option<(String, String)> {
    let payload = err
        .trim()
        .strip_prefix(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX)?;
    if let Ok(value) = serde_json::from_str::<Value>(payload) {
        let exchange = value
            .get("exchange")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let detail = value
            .get("detail")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("private exchange account access failed")
            .to_string();
        return Some((exchange, detail));
    }
    let detail = payload.trim();
    Some((
        String::new(),
        if detail.is_empty() {
            "private exchange account access failed".to_string()
        } else {
            detail.to_string()
        },
    ))
}

fn crypto_account_access_error_from_structured_extra(
    structured: &StructuredSkillError,
) -> Option<(String, String)> {
    let extra_kind = structured_extra_string(structured, "error_kind");
    let message_key = structured_extra_string(structured, "message_key");
    let structured_kind = structured.error_kind.trim();
    let is_account_access = structured_kind == "account_access_failed"
        || structured_kind == "crypto_account_access_failed"
        || extra_kind.as_deref() == Some("account_access_failed")
        || extra_kind.as_deref() == Some("crypto_account_access_failed")
        || message_key.as_deref() == Some("crypto.err.account_access_failed");
    if !is_account_access {
        return None;
    }

    let legacy = parse_crypto_account_access_error(&structured.error_text);
    let exchange = structured_extra_string(structured, "exchange")
        .or_else(|| legacy.as_ref().map(|(exchange, _)| exchange.clone()))
        .unwrap_or_default();
    let detail = structured_extra_string(structured, "detail")
        .or_else(|| legacy.as_ref().map(|(_, detail)| detail.clone()))
        .unwrap_or_else(|| structured.error_text.trim().to_string())
        .trim()
        .to_string();
    Some((exchange, detail))
}

fn structured_crypto_account_access_error(
    skill_name: &str,
    structured: &StructuredSkillError,
) -> Option<(String, String)> {
    let effective_skill = if structured.skill.trim().is_empty() {
        skill_name
    } else {
        structured.skill.as_str()
    };
    if !effective_skill.eq_ignore_ascii_case("crypto") {
        return None;
    }
    if let Some(error) = crypto_account_access_error_from_structured_extra(structured) {
        return Some(error);
    }
    parse_crypto_account_access_error(&structured.error_text)
}

fn is_crypto_recoverable_i18n_message_key(message_key: &str) -> bool {
    matches!(
        message_key.trim(),
        "crypto.err.binance_not_bound"
            | "crypto.err.binance_credentials_incomplete"
            | "crypto.err.okx_not_bound"
            | "crypto.err.okx_credentials_incomplete"
    )
}

fn crypto_recoverable_i18n_error_from_structured(
    skill_name: &str,
    structured: &StructuredSkillError,
) -> Option<(String, String, String, String)> {
    let effective_skill = if structured.skill.trim().is_empty() {
        skill_name
    } else {
        structured.skill.as_str()
    };
    if !effective_skill.eq_ignore_ascii_case("crypto") {
        return None;
    }
    let message_key = structured_extra_string(structured, "message_key")?;
    if !is_crypto_recoverable_i18n_message_key(&message_key) {
        return None;
    }
    let error_kind = structured_extra_string(structured, "error_kind")
        .unwrap_or_else(|| structured.error_kind.trim().to_string());
    let exchange = structured_extra_string(structured, "exchange").unwrap_or_default();
    let action = structured_extra_string(structured, "action").unwrap_or_default();
    Some((message_key, error_kind, exchange, action))
}

pub(crate) fn crypto_recoverable_i18n_error_key(skill_name: &str, err: &str) -> Option<String> {
    let structured = parse_structured_skill_error(err)?;
    crypto_recoverable_i18n_error_from_structured(skill_name, &structured)
        .map(|(message_key, _, _, _)| message_key)
}

pub(crate) fn policy_block_default_text(
    _state: &AppState,
    _task: &ClaimedTask,
    _user_text: &str,
    block: &PolicyBlockError,
) -> String {
    policy_block_machine_payload(block)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestReplyLanguage {
    ZhCn,
    En,
    ConfigDefault,
}

#[derive(Debug, Clone)]
pub(crate) struct SkillRunOutcome {
    pub(crate) text: String,
    pub(crate) notify: Option<bool>,
    pub(crate) validation: Option<Value>,
    pub(crate) extra: Option<Value>,
}

struct SkillExecutionIsolation {
    state: AppState,
    artifact_refs: Vec<Value>,
}

fn prepare_builtin_run_cmd_async_start_args(workspace_root: &Path, args: &mut Value) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    if !obj
        .get("async_start")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return;
    }
    let job_uuid = uuid::Uuid::new_v4().to_string();
    let job_id = ["local_process", job_uuid.as_str()].join(":");
    let job_dir = workspace_root
        .join(".rustclaw")
        .join("async_jobs")
        .join(&job_uuid);
    let poll_after_seconds = obj
        .get("poll_after_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 3600);
    let expires_in_seconds = obj
        .get("expires_in_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(3600)
        .clamp(1, 86_400);
    let expires_at = (crate::now_ts_u64() as i64).saturating_add(expires_in_seconds as i64);
    obj.insert("_clawd_async_job_id".to_string(), json!(job_id));
    obj.insert(
        "_clawd_async_job_dir".to_string(),
        json!(job_dir.display().to_string()),
    );
    obj.insert(
        "_clawd_async_poll_after_seconds".to_string(),
        json!(poll_after_seconds),
    );
    obj.insert("_clawd_async_expires_at".to_string(), json!(expires_at));
}

fn builtin_success_extra(workspace_root: &Path, skill_name: &str, args: &Value) -> Option<Value> {
    let obj = args.as_object()?;
    match skill_name {
        "run_cmd" => {
            if !obj
                .get("async_start")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return None;
            }
            let job_id = obj
                .get("_clawd_async_job_id")
                .and_then(Value::as_str)?
                .trim();
            let job_dir = obj
                .get("_clawd_async_job_dir")
                .and_then(Value::as_str)?
                .trim();
            let poll_after_seconds = obj
                .get("_clawd_async_poll_after_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(5);
            let expires_at = obj
                .get("_clawd_async_expires_at")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if job_id.is_empty() || job_dir.is_empty() || expires_at <= 0 {
                return None;
            }
            Some(json!({
                "schema_version": 1,
                "source": "builtin_success_extra",
                "action": "async_start",
                "pending_async_job": {
                    "job_id": job_id,
                    "provider": "local_process",
                    "status": "accepted",
                    "poll_after_seconds": poll_after_seconds,
                    "poll_after_ms": poll_after_seconds.saturating_mul(1_000),
                    "expires_at": expires_at,
                    "cancel_ref": format!("local_process:{}", job_dir),
                    "cancel_token": format!("local_process:{}", job_dir),
                    "result_ref": job_id,
                    "retryable": true,
                    "message_key": "clawd.task.async_job_pending"
                }
            }))
        }
        "write_file" => {
            let path = obj.get("path").and_then(Value::as_str)?.trim();
            if path.is_empty() {
                return None;
            }
            let effective_path = crate::ensure_default_file_path(workspace_root, path);
            let resolved_path = workspace_resolved_path(workspace_root, &effective_path);
            let append = obj.get("append").and_then(Value::as_bool).unwrap_or(false);
            let content_bytes = obj.get("content").and_then(Value::as_str).map(str::len);
            let mut extra = json!({
                "schema_version": 1,
                "source": "builtin_success_extra",
                "action": if append { "append_text" } else { "write_text" },
                "path": path,
                "effective_path": effective_path,
                "resolved_path": resolved_path,
                "append": append,
                "content_bytes": content_bytes,
            });
            if let Some(change) = write_file_change_metadata(&resolved_path, append, obj) {
                merge_object_fields(&mut extra, change);
            }
            Some(extra)
        }
        "make_dir" => {
            let path = obj.get("path").and_then(Value::as_str)?.trim();
            if path.is_empty() {
                return None;
            }
            Some(json!({
                "schema_version": 1,
                "source": "builtin_success_extra",
                "action": "make_dir",
                "path": path,
                "resolved_path": workspace_resolved_path(workspace_root, path),
            }))
        }
        "remove_file" => {
            let path = obj.get("path").and_then(Value::as_str)?.trim();
            if path.is_empty() {
                return None;
            }
            Some(json!({
                "schema_version": 1,
                "source": "builtin_success_extra",
                "action": "remove_path",
                "path": path,
                "resolved_path": workspace_resolved_path(workspace_root, path),
                "target_kind": obj.get("target_kind").cloned().unwrap_or(Value::Null),
                "recursive": obj.get("recursive").and_then(Value::as_bool).unwrap_or(false),
            }))
        }
        "schedule" => {
            let action = obj.get("action").and_then(Value::as_str)?.trim();
            if action.is_empty() {
                return None;
            }
            Some(json!({
                "schema_version": 1,
                "source": "builtin_success_extra",
                "action": action,
                "message_key": "schedule.workflow.completed",
                "status": "ok",
                "mode": obj.get("mode").cloned().unwrap_or(Value::Null),
                "dry_run": obj.get("dry_run").cloned().unwrap_or(Value::Bool(false)),
                "preview_only": obj.get("preview_only").cloned().unwrap_or(Value::Bool(false)),
                "target_job_id": obj.get("target_job_id").cloned().unwrap_or(Value::Null),
                "intent": obj.get("intent").cloned().unwrap_or(Value::Null),
            }))
        }
        _ => None,
    }
}

fn append_extra_artifact_refs(extra: Option<Value>, artifact_refs: &[Value]) -> Option<Value> {
    if artifact_refs.is_empty() {
        return extra;
    }
    let mut value = extra.unwrap_or_else(|| {
        json!({
            "schema_version": 1,
            "source": "skill_execution_isolation",
        })
    });
    if !value.is_object() {
        value = json!({
            "schema_version": 1,
            "source": "skill_execution_isolation",
            "value": value,
        });
    }
    if let Some(obj) = value.as_object_mut() {
        append_unique_json_array(obj, "artifact_refs", artifact_refs);
        append_unique_json_array(obj, "artifacts", artifact_refs);
    }
    Some(value)
}

fn append_unique_json_array(map: &mut Map<String, Value>, key: &str, items: &[Value]) {
    if items.is_empty() {
        return;
    }
    let entry = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(array) = entry.as_array_mut() {
        for item in items {
            if !array.iter().any(|existing| existing == item) {
                array.push(item.clone());
            }
        }
    }
}

fn workspace_resolved_path(workspace_root: &Path, path: &str) -> String {
    let path = Path::new(path);
    if path.is_absolute() {
        path.display().to_string()
    } else {
        workspace_root.join(path).display().to_string()
    }
}

fn write_file_change_metadata(
    resolved_path: &str,
    append: bool,
    obj: &Map<String, Value>,
) -> Option<Value> {
    let content = obj.get("content").and_then(Value::as_str)?;
    let path = Path::new(resolved_path);
    let existing = match std::fs::read_to_string(path) {
        Ok(existing) => Some(existing),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return None,
    };
    if append {
        let changed = !content.is_empty();
        return Some(json!({
            "preexisting": existing.is_some(),
            "noop": !changed,
            "changed": changed,
        }));
    }
    let noop = existing
        .as_deref()
        .is_some_and(|current| current == content);
    Some(json!({
        "preexisting": existing.is_some(),
        "noop": noop,
        "changed": !noop,
    }))
}

fn merge_object_fields(target: &mut Value, source: Value) {
    let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) else {
        return;
    };
    for (key, value) in source {
        target.insert(key.clone(), value.clone());
    }
}

pub(crate) fn is_recoverable_skill_error(skill_name: &str, err: &str) -> bool {
    if let Some(structured) = parse_structured_skill_error(err) {
        if structured_crypto_account_access_error(skill_name, &structured).is_some() {
            return true;
        }
        if crypto_recoverable_i18n_error_from_structured(skill_name, &structured).is_some() {
            return true;
        }
        let effective_skill = if structured.skill.trim().is_empty() {
            skill_name
        } else {
            structured.skill.as_str()
        };
        return matches_ignore_ascii_case(
            effective_skill,
            &["system_basic", "read_file", "list_dir"],
        ) && matches!(
            structured.error_kind.as_str(),
            "not_found"
                | "permission_denied"
                | "not_a_directory"
                | "is_directory"
                | "ambiguous_target"
        );
    }
    if skill_name.eq_ignore_ascii_case("read_file") && err.starts_with(READ_FILE_NOT_FOUND_PREFIX) {
        return true;
    }
    if skill_name.eq_ignore_ascii_case("crypto")
        && err.starts_with(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX)
    {
        return true;
    }
    false
}

pub(crate) fn is_crypto_account_access_error(skill_name: &str, err: &str) -> bool {
    if let Some(structured) = parse_structured_skill_error(err) {
        return structured_crypto_account_access_error(skill_name, &structured).is_some();
    }
    skill_name.eq_ignore_ascii_case("crypto")
        && err.trim().starts_with(CRYPTO_ACCOUNT_ACCESS_ERROR_PREFIX)
}

pub(crate) fn is_missing_target_skill_error(skill_name: &str, err: &str) -> bool {
    if let Some(structured) = parse_structured_skill_error(err) {
        let effective_skill = if structured.skill.trim().is_empty() {
            skill_name
        } else {
            structured.skill.as_str()
        };
        return matches_ignore_ascii_case(
            effective_skill,
            &["system_basic", "read_file", "list_dir"],
        ) && structured.error_kind == "not_found";
    }
    skill_name.eq_ignore_ascii_case("read_file") && err.starts_with(READ_FILE_NOT_FOUND_PREFIX)
}

pub(crate) fn read_file_not_found_path(err: &str) -> Option<&str> {
    err.trim()
        .strip_prefix(READ_FILE_NOT_FOUND_PREFIX)
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

pub(crate) fn skill_error_machine_observation(skill_name: &str, err: &str) -> Option<String> {
    let trimmed = err.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(structured) = parse_structured_skill_error(trimmed) {
        return Some(structured_skill_error_machine_observation(
            skill_name,
            &structured,
        ));
    }
    if let Some(block) = parse_policy_block_error(trimmed) {
        return Some(policy_block_machine_payload(&block));
    }
    if let Some(path) = read_file_not_found_path(trimmed) {
        return Some(
            json!({
                "message_key": "clawd.msg.skill.error_observation",
                "reason_code": "read_file_not_found",
                "skill": if skill_name.trim().is_empty() { "read_file" } else { skill_name.trim() },
                "error_kind": "not_found",
                "path": path,
            })
            .to_string(),
        );
    }
    if let Some((exchange, detail)) = parse_crypto_account_access_error(trimmed) {
        return Some(
            json!({
                "message_key": "crypto.err.account_access_failed",
                "reason_code": "crypto_account_access_failed",
                "skill": if skill_name.trim().is_empty() { "crypto" } else { skill_name.trim() },
                "error_kind": "account_access_failed",
                "exchange": exchange,
                "detail": detail,
            })
            .to_string(),
        );
    }
    None
}

fn structured_skill_error_machine_observation(
    skill_name: &str,
    structured: &StructuredSkillError,
) -> String {
    let effective_skill = if structured.skill.trim().is_empty() {
        skill_name.trim()
    } else {
        structured.skill.trim()
    };
    let mut extra = structured.extra.clone().unwrap_or(Value::Null);
    strip_user_visible_skill_error_fields(&mut extra);
    json!({
        "message_key": "clawd.msg.skill.error_observation",
        "reason_code": "structured_skill_error",
        "skill": effective_skill,
        "error_kind": structured.error_kind.trim(),
        "platform": structured.platform,
        "manager_type": structured.manager_type,
        "service_name": structured.service_name,
        "extra": extra,
    })
    .to_string()
}

fn strip_user_visible_skill_error_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("text");
            object.remove("error_text");
            for child in object.values_mut() {
                strip_user_visible_skill_error_fields(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                strip_user_visible_skill_error_fields(child);
            }
        }
        _ => {}
    }
}

pub(crate) fn is_observable_run_cmd_error(skill_name: &str, err: &str) -> bool {
    let Some(structured) = parse_structured_skill_error(err) else {
        return false;
    };
    let effective_skill = if structured.skill.trim().is_empty() {
        skill_name
    } else {
        structured.skill.as_str()
    };
    effective_skill.eq_ignore_ascii_case("run_cmd")
        && matches!(
            structured.error_kind.as_str(),
            "nonzero_exit"
                | "timeout"
                | "idle_timeout"
                | "spawn_failed"
                | "wait_failed"
                | "output_read_failed"
                | "status_unavailable"
        )
}

pub(crate) fn error_looks_like_os_permission_denied(error: &str) -> bool {
    parse_structured_skill_error(error)
        .is_some_and(|structured| structured.error_kind == "permission_denied")
}

fn matches_ignore_ascii_case(value: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

/// 历史遗留入口：只在 [`crate::runtime::state::AppState::resolve_canonical_skill_name`]
/// 拿不到 registry 时被作为最后兜底（identity 直传）。
///
/// **§P4.1（aliases 收敛）**：原本这里维护的一张 16 行硬编码 alias → canonical 的
/// match 表，与 `configs/skills_registry.toml` 里 `[[skills]].aliases` 同时存在，
/// 是典型的"双来源真相"。现在已经把那张表里的所有别名（包括拼写错容错 `fs_rearch`）
/// 收进 registry，本函数变成纯 identity，不再维护任何 alias 知识。
///
/// 调用方应当优先走 [`crate::runtime::state::AppState::resolve_canonical_skill_name`]
/// 而不是直接调本函数，原因：
/// - 走 AppState 才能命中 registry 的真实 alias 解析；
/// - 直接调本函数等价于"假装 registry 没加载"，会丢掉别名归一化能力。
///
/// 仍然返回 `&str` 是为了向后兼容已有的 `crate::canonical_skill_name(s).to_string()`
/// 调用点，避免本轮 P4.1 一次改动牵动 runtime/state.rs 多处签名。
pub(crate) fn canonical_skill_name(name: &str) -> &str {
    name.trim()
}

fn current_task_auth_role(state: &AppState, task: &ClaimedTask) -> Option<String> {
    task.user_key
        .as_deref()
        .and_then(|user_key| {
            crate::resolve_auth_identity_by_key(state, user_key)
                .ok()
                .flatten()
        })
        .map(|identity| identity.role)
}

fn task_is_admin(state: &AppState, task: &ClaimedTask) -> bool {
    current_task_auth_role(state, task)
        .map(|role| role.eq_ignore_ascii_case("admin"))
        .unwrap_or(false)
}

pub(crate) fn task_allows_sudo(state: &AppState, task: Option<&ClaimedTask>) -> bool {
    state.policy.allow_sudo && task.map(|task| task_is_admin(state, task)).unwrap_or(false)
}

pub(crate) fn task_allows_path_outside_workspace(
    state: &AppState,
    task: Option<&ClaimedTask>,
) -> bool {
    state.policy.allow_path_outside_workspace
        && task.map(|task| task_is_admin(state, task)).unwrap_or(false)
}

fn request_reply_language(user_text: &str) -> RequestReplyLanguage {
    match crate::language_policy::request_language_hint(user_text) {
        "zh-CN" => RequestReplyLanguage::ZhCn,
        "en" => RequestReplyLanguage::En,
        _ => RequestReplyLanguage::ConfigDefault,
    }
}

fn extract_task_request_text(payload_json: &str) -> Option<String> {
    let payload = serde_json::from_str::<Value>(payload_json).ok()?;
    for candidate in [
        payload.get("text").and_then(|v| v.as_str()),
        payload.get("request_text").and_then(|v| v.as_str()),
        payload.get("prompt").and_then(|v| v.as_str()),
        payload.get("content").and_then(|v| v.as_str()),
        payload
            .get("args")
            .and_then(|v| v.get("request_text"))
            .and_then(|v| v.as_str()),
        payload
            .get("args")
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str()),
    ] {
        let trimmed = candidate.map(str::trim).filter(|text| !text.is_empty());
        if let Some(text) = trimmed {
            return Some(text.to_string());
        }
    }
    None
}

fn task_request_locale_tag(state: &AppState, task: &ClaimedTask) -> String {
    match extract_task_request_text(&task.payload_json)
        .as_deref()
        .map(request_reply_language)
        .unwrap_or(RequestReplyLanguage::ConfigDefault)
    {
        RequestReplyLanguage::ZhCn => "zh-CN".to_string(),
        RequestReplyLanguage::En => "en-US".to_string(),
        RequestReplyLanguage::ConfigDefault => {
            let locale = state.policy.schedule.locale.trim().to_ascii_lowercase();
            if locale.starts_with("en") {
                "en-US".to_string()
            } else {
                "zh-CN".to_string()
            }
        }
    }
}

fn normalized_path_components(path: &Path) -> (bool, Vec<String>) {
    let mut absolute = false;
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => absolute = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if components.last().is_some_and(|last| last != "..") {
                    components.pop();
                } else if !absolute {
                    components.push("..".to_string());
                }
            }
            Component::Normal(part) => components.push(part.to_string_lossy().to_string()),
            Component::Prefix(_) => {}
        }
    }
    (absolute, components)
}

fn path_targets_configs_dir(workspace_root: &Path, raw_path: &str) -> bool {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return false;
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        let config_root = workspace_root.join("configs");
        let (candidate_abs, candidate_parts) = normalized_path_components(candidate);
        let (config_abs, config_parts) = normalized_path_components(&config_root);
        return candidate_abs == config_abs && candidate_parts.starts_with(&config_parts);
    }

    let (_, parts) = normalized_path_components(candidate);
    parts.first().is_some_and(|part| part == "configs")
}

fn args_path_targets_configs_dir(
    workspace_root: &Path,
    args: &Value,
    path_key: &str,
    apply_default_file_path: bool,
) -> bool {
    let Some(path) = args.get(path_key).and_then(|value| value.as_str()) else {
        return false;
    };
    let effective = if apply_default_file_path {
        crate::ensure_default_file_path(workspace_root, path)
    } else {
        path.to_string()
    };
    path_targets_configs_dir(workspace_root, &effective)
}

fn clean_shell_token(raw: &str) -> String {
    raw.trim_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
            )
    })
    .to_string()
}

fn run_cmd_targets_config_mutation(workspace_root: &Path, command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let mentions_configs = command
        .split_whitespace()
        .map(clean_shell_token)
        .filter(|token| !token.is_empty())
        .any(|token| path_targets_configs_dir(workspace_root, &token));
    if !mentions_configs {
        return false;
    }

    let first_word = command
        .split_whitespace()
        .map(clean_shell_token)
        .find(|token| {
            !token.is_empty()
                && !(token.contains('=')
                    && !token.starts_with("./")
                    && !token.contains('/')
                    && !token.starts_with('-'))
        })
        .map(|token| token.to_ascii_lowercase());

    let has_explicit_write_marker = command.contains('>')
        || lower.contains(" tee ")
        || lower.starts_with("tee ")
        || lower.contains(" sed -i")
        || lower.starts_with("sed -i")
        || lower.contains(" perl -pi")
        || lower.starts_with("perl -pi")
        || matches!(
            first_word.as_deref(),
            Some(
                "cp" | "mv"
                    | "rm"
                    | "mkdir"
                    | "touch"
                    | "truncate"
                    | "install"
                    | "dd"
                    | "chmod"
                    | "chown"
                    | "ln"
            )
        );
    if has_explicit_write_marker {
        return true;
    }

    !matches!(
        first_word.as_deref(),
        Some(
            "cat"
                | "rg"
                | "grep"
                | "sed"
                | "awk"
                | "head"
                | "tail"
                | "ls"
                | "find"
                | "wc"
                | "stat"
                | "readlink"
                | "realpath"
                | "bat"
        )
    )
}

fn skill_attempts_config_mutation(state: &AppState, skill_name: &str, args: &Value) -> bool {
    match skill_name {
        "write_file" => {
            args_path_targets_configs_dir(&state.skill_rt.workspace_root, args, "path", true)
        }
        "remove_file" | "make_dir" => {
            args_path_targets_configs_dir(&state.skill_rt.workspace_root, args, "path", false)
        }
        "run_cmd" => args
            .get("command")
            .and_then(|value| value.as_str())
            .map(|command| run_cmd_targets_config_mutation(&state.skill_rt.workspace_root, command))
            .unwrap_or(false),
        "config_guard" => {
            if !args_path_targets_configs_dir(&state.skill_rt.workspace_root, args, "path", false) {
                return false;
            }
            if args.get("key").is_some() || args.get("value").is_some() {
                return true;
            }
            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            !action.is_empty()
                && ["patch", "write", "set", "update", "modify", "apply"]
                    .iter()
                    .any(|needle| action.contains(needle))
        }
        "config_edit" => {
            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            matches!(
                action.as_str(),
                "apply_config_change" | "apply_change" | "write_field" | "set_field"
            ) && if args.get("path").and_then(|value| value.as_str()).is_none() {
                true
            } else {
                args_path_targets_configs_dir(&state.skill_rt.workspace_root, args, "path", true)
            }
        }
        "extension_manager" => {
            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            matches!(
                action.as_str(),
                "register_external_skill" | "enable_external_skill"
            ) || (action == "temporary_fix_execute" && args.to_string().contains("configs/"))
        }
        _ => false,
    }
}

fn ensure_config_mutation_allowed(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: &Value,
) -> Result<(), String> {
    if !skill_attempts_config_mutation(state, skill_name, args) || task_is_admin(state, task) {
        return Ok(());
    }
    Err(policy_block_error(
        "config_requires_web_admin",
        vec![format!("skill: {skill_name}")],
        vec![
            "action=mutate_config".to_string(),
            "required_auth=admin_authorized_task".to_string(),
            "preferred_surface=web_admin_console".to_string(),
        ],
    ))
}

fn skill_action_token(args: &Value) -> Option<String> {
    args.as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase().replace(['-', ' ', '.'], "_"))
}

fn action_scoped_isolation_profile(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> Option<CapabilityIsolationProfile> {
    let action = skill_action_token(args);
    state.skill_manifest(skill_name).and_then(|manifest| {
        claw_core::skill_registry::select_planner_capability_mapping(
            &manifest.planner_capabilities,
            action.as_deref(),
        )
        .and_then(|mapping| mapping.isolation_profile)
    })
}

fn skill_execution_isolation_error(skill_name: &str, detail: String) -> String {
    structured_skill_error_from_parts(
        skill_name,
        "execution_isolation_setup_failed",
        "execution_isolation_setup_failed",
        Some(std::env::consts::OS),
        Some(json!({
            "reason_code": "execution_isolation_setup_failed",
            "message_key": "clawd.execution.isolation_setup_failed",
            "detail": detail,
        })),
    )
}

fn prepare_skill_execution_isolation(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: &Value,
) -> Result<Option<SkillExecutionIsolation>, String> {
    let Some(profile) = action_scoped_isolation_profile(state, skill_name, args) else {
        return Ok(None);
    };
    let plan = crate::execution_isolation::plan_execution_isolation(
        &state.skill_rt.workspace_root,
        &task.task_id,
        profile,
    )
    .map_err(|err| skill_execution_isolation_error(skill_name, err.to_string()))?;
    if !plan.requires_cleanup {
        return Ok(None);
    }
    let runtime =
        crate::execution_isolation::create_execution_isolation(&plan, crate::now_ts_u64())
            .map_err(|err| skill_execution_isolation_error(skill_name, err.to_string()))?;
    let mut isolated_state = state.clone();
    isolated_state.skill_rt.workspace_root = runtime.plan.execution_root.clone();
    isolated_state.skill_rt.default_locator_search_dir = runtime.plan.execution_root.clone();
    Ok(Some(SkillExecutionIsolation {
        state: isolated_state,
        artifact_refs: runtime.artifact_refs,
    }))
}

fn skill_risk_level_token(risk: SkillRiskLevel) -> &'static str {
    match risk {
        SkillRiskLevel::Unknown => "unknown",
        SkillRiskLevel::Low => "low",
        SkillRiskLevel::Medium => "medium",
        SkillRiskLevel::High => "high",
    }
}

fn effective_dispatch_risk_level(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> SkillRiskLevel {
    let Some(manifest) = state.skill_manifest(skill_name) else {
        return SkillRiskLevel::Unknown;
    };
    let action = skill_action_token(args);
    let action_risk = claw_core::skill_registry::select_planner_capability_mapping(
        &manifest.planner_capabilities,
        action.as_deref(),
    )
    .and_then(|mapping| mapping.risk_level);
    action_risk
        .or(manifest.risk_level)
        .unwrap_or(SkillRiskLevel::Unknown)
}

fn audit_high_risk_skill_start(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: &Value,
) {
    let risk_level = effective_dispatch_risk_level(state, skill_name, args);
    let requires_confirmation =
        state.skill_invocation_requires_confirmation_policy(skill_name, Some(args));
    if risk_level != SkillRiskLevel::High && !requires_confirmation {
        return;
    }
    let detail = json!({
        "task_id": task.task_id,
        "skill": skill_name,
        "action": skill_action_token(args),
        "risk_level": skill_risk_level_token(risk_level),
        "requires_confirmation": requires_confirmation,
    })
    .to_string();
    if let Err(err) = crate::repo::insert_audit_log(
        state,
        Some(task.user_id),
        "skill_dispatch.high_risk_start",
        Some(&detail),
        None,
    ) {
        tracing::warn!(error = %err, "skill_dispatch_high_risk_start_audit_failed");
    }
}

/// §P4.1 fallback：当 `SkillsRegistry` 还没装载（启动早期 / 某些测试 stub）时
/// 用这个常量名单兜底"哪些 skill 是 builtin（in-process）"。**真正生效的是
/// `AppState::is_builtin_skill`**——它优先从 registry 拿 kind，failure 才退到这里。
///
/// 维护规则：本列表必须与 `configs/skills_registry.toml` / `docker/config/skills_registry.toml`
/// 中 `kind = "builtin"` 的 skill 一一对应；新增/删除 builtin 时同步改这里，并由
/// `crates/clawd/tests/config_templates.rs` 的 `registry_covers_all_required_builtins`
/// 负责守底（registry 必须覆盖这里列出的每一个名字）。
pub(crate) fn is_builtin_skill_name(name: &str) -> bool {
    matches!(
        name,
        "run_cmd"
            | "fs_basic"
            | "config_basic"
            | "read_file"
            | "write_file"
            | "list_dir"
            | "make_dir"
            | "remove_file"
            | "workspace_patch"
            | "schedule"
    )
}

pub(crate) async fn run_skill_with_runner_outcome(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    mut args: serde_json::Value,
) -> Result<SkillRunOutcome, String> {
    let mut skill_name = state.resolve_canonical_skill_name(skill_name);
    if skill_name.is_empty() {
        return Err("skill_name is empty".to_string());
    }
    if crate::virtual_tools::normalize_virtual_tool_arg_aliases(&skill_name, &mut args) {
        tracing::info!(
            "skill_virtual_args_rewrite skill={} args={}",
            skill_name,
            crate::truncate_for_log(&crate::visible_text::sanitize_user_visible_text(
                &args.to_string()
            ))
        );
    }
    if let Some(rewrite) =
        crate::virtual_tools::rewrite_virtual_tool_call(&skill_name, args.clone())?
    {
        tracing::info!(
            "skill_virtual_dispatch requested_skill={} runtime_skill={} args={}",
            skill_name,
            rewrite.runtime_tool,
            crate::truncate_for_log(&crate::visible_text::sanitize_user_visible_text(
                &rewrite.runtime_args.to_string()
            ))
        );
        skill_name = state.resolve_canonical_skill_name(&rewrite.runtime_tool);
        args = rewrite.runtime_args;
    }

    let policy_token = format!("skill:{skill_name}");
    if !state
        .skill_rt
        .tools_policy
        .is_allowed(&policy_token, state.core.active_provider_type.as_deref())
    {
        return Err(policy_block_error(
            "skill_policy_denied",
            vec![
                format!("skill: {skill_name}"),
                format!("policy_token: {policy_token}"),
            ],
            vec![
                "action=execute_skill".to_string(),
                "policy=tools_policy".to_string(),
                "required_decision=allow".to_string(),
            ],
        ));
    }

    if !state.get_skills_list().contains(&skill_name) {
        let mut allowed: Vec<String> = state.get_skills_list().iter().cloned().collect();
        allowed.sort();
        return Err(policy_block_error(
            "skill_disabled",
            vec![
                format!("skill: {skill_name}"),
                format!("enabled_skills: {}", allowed.join(", ")),
            ],
            vec![
                "action=execute_skill".to_string(),
                "required_state=skill_enabled".to_string(),
                "config_scope=skills_list_or_skill_switches".to_string(),
            ],
        ));
    }
    if !state.task_allows_skill(task, &skill_name) {
        return Err(policy_block_error(
            "agent_skill_disabled",
            vec![
                format!("skill: {skill_name}"),
                format!("agent_id: {}", state.task_agent_id(task)),
            ],
            vec![
                "action=execute_skill".to_string(),
                "required_state=agent_skill_enabled".to_string(),
                "config_scope=agent_skill_policy".to_string(),
            ],
        ));
    }
    if let Some(reason_code) =
        crate::verifier::skill_sandbox_denial_reason(state, &skill_name, &args)
    {
        return Err(structured_skill_error_from_parts(
            &skill_name,
            "sandbox_policy_denied",
            "sandbox_policy_denied",
            Some(std::env::consts::OS),
            Some(json!({
                "reason_code": reason_code,
                "message_key": "clawd.execution.sandbox_policy_denied",
                "decision": crate::policy_decision::PolicyDecision::Deny.as_token(),
                "sandbox_mode": state.skill_rt.tools_policy.sandbox_mode_token(),
                "approval_policy": state.skill_rt.tools_policy.approval_policy_token(),
                "skill": skill_name,
                "action": skill_action_token(&args),
            })),
        ));
    }
    ensure_config_mutation_allowed(state, task, &skill_name, &args)?;

    let kind = state.skill_kind_for_dispatch(&skill_name);
    let kind_str = match kind {
        SkillKind::Builtin => "builtin",
        SkillKind::Runner => "runner",
        SkillKind::External => "external",
    };
    audit_high_risk_skill_start(state, task, &skill_name, &args);
    let execution_isolation = prepare_skill_execution_isolation(state, task, &skill_name, &args)?;
    let (execution_state, isolation_artifact_refs) =
        if let Some(isolation) = execution_isolation.as_ref() {
            (&isolation.state, isolation.artifact_refs.as_slice())
        } else {
            (state, &[][..])
        };
    tracing::info!(
        "skill_dispatch skill={} kind={} branch={}",
        skill_name,
        kind_str,
        kind_str
    );

    match kind {
        SkillKind::Builtin => {
            if skill_name == "run_cmd" {
                prepare_builtin_run_cmd_async_start_args(
                    &execution_state.skill_rt.workspace_root,
                    &mut args,
                );
            }
            let mut extra = append_extra_artifact_refs(
                builtin_success_extra(&execution_state.skill_rt.workspace_root, &skill_name, &args),
                isolation_artifact_refs,
            );
            return execute_builtin_skill_for_task(execution_state, task, &skill_name, &args)
                .await
                .map(|text| {
                    if skill_name == "workspace_patch" {
                        extra = append_extra_artifact_refs(
                            serde_json::from_str::<Value>(&text).ok(),
                            isolation_artifact_refs,
                        );
                    }
                    SkillRunOutcome {
                        text,
                        notify: None,
                        validation: None,
                        extra,
                    }
                });
        }
        SkillKind::External | SkillKind::Runner => {}
    }

    let skill_timeout_secs = state
        .get_skills_registry()
        .as_ref()
        .and_then(|r| {
            let s = r.timeout_seconds(&skill_name);
            if s > 0 {
                Some(state.skill_rt.skill_timeout_seconds.max(s))
            } else {
                None
            }
        })
        .unwrap_or_else(|| match skill_name.as_str() {
            "image_generate" | "image_edit" => state.skill_rt.skill_timeout_seconds.max(180),
            "image_vision" => state.skill_rt.skill_timeout_seconds.max(90),
            "audio_transcribe" => state.skill_rt.skill_timeout_seconds.max(120),
            "audio_synthesize" => state.skill_rt.skill_timeout_seconds.max(90),
            "crypto" => state.skill_rt.skill_timeout_seconds.max(60),
            _ => state.skill_rt.skill_timeout_seconds,
        });

    let _permit = state
        .skill_rt
        .skill_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| format!("skill semaphore closed: {err}"))?;

    let args = inject_skill_memory_context(execution_state, task, &skill_name, args);
    let args = ensure_default_output_dir_for_skill_args(
        &execution_state.skill_rt.workspace_root,
        &skill_name,
        args,
    );
    let source = match task_runtime_channel(execution_state, task) {
        RuntimeChannel::Whatsapp => "whatsapp",
        RuntimeChannel::Telegram => "telegram",
        RuntimeChannel::Wechat => "wechat",
        RuntimeChannel::Feishu => "feishu",
        RuntimeChannel::Lark => "lark",
    };

    let value = match kind {
        SkillKind::External => {
            execute_external_skill(execution_state, task, &skill_name, &args, &source).await?
        }
        SkillKind::Runner => {
            let runner_name = execution_state.runner_name_for_skill(&skill_name);
            tracing::info!(
                "skill_dispatch skill={} runner_name={} kind=runner",
                skill_name,
                runner_name
            );
            run_skill_with_runner_once(
                execution_state,
                task,
                &skill_name,
                &runner_name,
                &args,
                &source,
                skill_timeout_secs,
            )
            .await?
        }
        SkillKind::Builtin => unreachable!(),
    };
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("error")
        .to_string();

    if status != "ok" {
        return Err(structured_skill_error_string(&skill_name, &value));
    }

    if let Some((provider, model, model_kind)) = runner::extract_skill_provider_model(&value) {
        tracing::info!(
            "{} skill_model_selected task_id={} skill={} provider={} model={} model_kind={}",
            crate::highlight_tag("skill_llm"),
            task.task_id,
            skill_name,
            provider,
            model,
            model_kind
        );
    }

    if let Some(llm_meta) = value
        .get("extra")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("llm"))
        .and_then(|v| v.as_object())
    {
        let prompt_name = llm_meta
            .get("prompt_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let model = llm_meta
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        tracing::info!(
            "{} skill_llm_call task_id={} skill={} prompt={} model={}",
            crate::highlight_tag("skill_llm"),
            task.task_id,
            skill_name,
            prompt_name,
            model
        );
    }

    let notify = value
        .get("extra")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("notify"))
        .and_then(|v| v.as_bool());
    let validation = value.get("validation").cloned().or_else(|| {
        value
            .get("extra")
            .and_then(|v| v.get("validation"))
            .cloned()
    });
    let extra = append_extra_artifact_refs(value.get("extra").cloned(), isolation_artifact_refs);
    let text = value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(SkillRunOutcome {
        text,
        notify,
        validation,
        extra,
    })
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
