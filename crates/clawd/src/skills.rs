use claw_core::skill_registry::SkillKind;
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
/// * 严格模式默认 OFF (`RUSTCLAW_SKILL_ENV_STRICT=1` 才打开)，避免兼容性突变；
///   开启后 skill 若再依赖未声明的环境变量会立刻为空，运维可由此发现遗漏。
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

/// §E2 step1: 运行期判断是否启用 strict env 隔离。
///
/// 接受 `1` / `true` / `on` / `yes`（大小写不敏感）作为打开信号，其余视为关闭。
pub(crate) fn skill_runner_env_strict_enabled() -> bool {
    matches!(
        std::env::var("RUSTCLAW_SKILL_ENV_STRICT")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1") | Some("true") | Some("on") | Some("yes")
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
/// §E2 step2 边界澄清：本函数只对 **spawn-path** 生效（即 `kind="runner"` 的外部
/// skill），对 **builtin skill**（`read_file` / `write_file` / `run_cmd` 等
/// 内嵌实现）完全无效——它们运行在 clawd 自身进程里，自然继承 clawd 的 env。
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

mod builtin;
mod external;
mod memory_context;
mod output_dirs;
mod runner;

pub(crate) use builtin::{execute_builtin_skill_for_task, run_safe_command};
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

fn child_text_object(value: &Value) -> Option<Value> {
    string_field(value, "text").and_then(|text| serde_json::from_str::<Value>(&text).ok())
}

fn structured_skill_error_string(skill: &str, value: &Value) -> String {
    let text_object = child_text_object(value).unwrap_or(Value::Null);
    let error_kind = string_field(value, "error_kind")
        .or_else(|| string_field(&text_object, "error_kind"))
        .or_else(|| {
            value
                .get("extra")
                .and_then(|extra| string_field(extra, "error_kind"))
        })
        .unwrap_or_else(|| "unknown".to_string());
    let error_text = string_field(value, "error_text")
        .or_else(|| string_field(&text_object, "failure_reason"))
        .unwrap_or_else(|| "skill execution failed".to_string());
    let payload = json!({
        "skill": skill.trim(),
        "error_kind": error_kind,
        "error_text": error_text,
        "platform": string_field(value, "platform").or_else(|| string_field(&text_object, "platform")),
        "manager_type": string_field(&text_object, "manager_type"),
        "service_name": string_field(&text_object, "service_name"),
        "extra": value.get("extra").cloned().unwrap_or(Value::Null),
        "text": text_object,
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

fn structured_extra_i64(structured: &StructuredSkillError, key: &str) -> Option<i64> {
    structured_extra_value(structured, key).and_then(|value| value.as_i64())
}

fn structured_extra_bool(structured: &StructuredSkillError, key: &str) -> bool {
    structured_extra_value(structured, key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn compact_stream_for_user(text: &str) -> String {
    let compact = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    crate::truncate_for_agent_trace(&compact)
}

fn normalize_run_cmd_structured_error_for_user(structured: &StructuredSkillError) -> String {
    let mut message = match structured.error_kind.as_str() {
        "nonzero_exit" => {
            if let Some(exit_code) = structured_extra_i64(structured, "exit_code") {
                match structured_extra_string(structured, "exit_category").as_deref() {
                    Some("command_not_found") => {
                        format!("command failed: command not found (exit code {exit_code})")
                    }
                    Some("command_not_executable") => {
                        format!("command failed: command is not executable (exit code {exit_code})")
                    }
                    Some("terminated_by_signal_or_shell_status") => {
                        format!("command failed: terminated by signal or shell status (exit code {exit_code})")
                    }
                    _ => format!("command failed with exit code {exit_code}"),
                }
            } else {
                "command failed with a non-zero exit status".to_string()
            }
        }
        "timeout" => "command timed out".to_string(),
        "idle_timeout" => "command idle timed out".to_string(),
        "spawn_failed" => "command failed to start".to_string(),
        "wait_failed" => "command wait failed".to_string(),
        "output_read_failed" => "command output read failed".to_string(),
        "status_unavailable" => "command status unavailable".to_string(),
        "invalid_input" => "command input is invalid".to_string(),
        _ => structured.error_text.trim().to_string(),
    };

    if let Some(stderr) = structured_extra_string(structured, "stderr") {
        message.push_str("; stderr: ");
        message.push_str(&compact_stream_for_user(&stderr));
    }
    if let Some(stdout) = structured_extra_string(structured, "stdout") {
        message.push_str("; stdout: ");
        message.push_str(&compact_stream_for_user(&stdout));
    }
    if structured_extra_bool(structured, "output_truncated") {
        message.push_str("; output was truncated");
    }
    if message.trim().is_empty() {
        structured.error_text.trim().to_string()
    } else {
        message
    }
}

pub(crate) fn policy_block_error(
    reason_code: &str,
    observed_facts: Vec<String>,
    policy_boundary: Vec<String>,
) -> String {
    let payload = json!({
        "reason_code": reason_code.trim(),
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
        "reason_code": block.reason_code,
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

fn crypto_account_access_error_observation(exchange: &str, detail: &str) -> String {
    let mut parts = vec![
        "message_key=crypto.err.account_access_failed".to_string(),
        "error_kind=account_access_failed".to_string(),
    ];
    let exchange = exchange.trim();
    if !exchange.is_empty() {
        parts.push(format!("exchange={exchange}"));
    }
    let detail = detail.trim();
    if !detail.is_empty() {
        parts.push(format!("detail={detail}"));
    }
    parts.join(" ")
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

fn crypto_recoverable_i18n_error_observation(
    message_key: &str,
    error_kind: &str,
    exchange: &str,
    action: &str,
) -> String {
    let mut parts = vec![
        format!("message_key={}", message_key.trim()),
        format!("error_kind={}", error_kind.trim()),
    ];
    let exchange = exchange.trim();
    if !exchange.is_empty() {
        parts.push(format!("exchange={exchange}"));
    }
    let action = action.trim();
    if !action.is_empty() {
        parts.push(format!("action={action}"));
    }
    parts.join(" ")
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

pub(crate) fn normalize_skill_error_for_user(skill_name: &str, err: &str) -> String {
    if let Some(structured) = parse_structured_skill_error(err) {
        let effective_skill = if structured.skill.trim().is_empty() {
            skill_name
        } else {
            structured.skill.as_str()
        };
        if let Some((exchange, detail)) =
            structured_crypto_account_access_error(skill_name, &structured)
        {
            return crypto_account_access_error_observation(&exchange, &detail);
        }
        if let Some((message_key, error_kind, exchange, action)) =
            crypto_recoverable_i18n_error_from_structured(skill_name, &structured)
        {
            return crypto_recoverable_i18n_error_observation(
                &message_key,
                &error_kind,
                &exchange,
                &action,
            );
        }
        if structured.error_kind.starts_with("contract_") {
            return "planned tool step was not allowed for this request".to_string();
        }
        if matches_ignore_ascii_case(
            effective_skill,
            &[
                "read_file",
                "write_file",
                "list_dir",
                "make_dir",
                "remove_file",
            ],
        ) {
            return match structured.error_kind.as_str() {
                "permission_denied" => {
                    "file operation failed: permission denied by the operating system".to_string()
                }
                "is_directory" => {
                    "file operation failed: target is a directory, not a regular file".to_string()
                }
                "not_a_directory" => {
                    "directory operation failed: target is not a directory".to_string()
                }
                "not_found" => "file operation failed: target path was not found".to_string(),
                "ambiguous_target" => {
                    "directory operation failed: target matched multiple candidates".to_string()
                }
                "content_too_large" => "write operation failed: content is too large".to_string(),
                "invalid_args" => "file operation failed: invalid arguments".to_string(),
                _ => structured.error_text,
            };
        }
        if effective_skill.eq_ignore_ascii_case("system_basic") {
            return match structured.error_kind.as_str() {
                "permission_denied" => {
                    "read operation failed: permission denied by the operating system".to_string()
                }
                "is_directory" => {
                    "read operation failed: target is a directory, not a regular file".to_string()
                }
                "not_a_directory" => {
                    "directory operation failed: target is not a directory".to_string()
                }
                "not_found" => "read operation failed: target path was not found".to_string(),
                _ => structured.error_text,
            };
        }
        if effective_skill.eq_ignore_ascii_case("run_cmd") {
            return normalize_run_cmd_structured_error_for_user(&structured);
        }
        return structured.error_text;
    }
    if let Some(policy_block) = parse_policy_block_error(err) {
        return policy_block_machine_payload(&policy_block);
    }
    if skill_name.eq_ignore_ascii_case("read_file") {
        if let Some(path) = err.strip_prefix(READ_FILE_NOT_FOUND_PREFIX) {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return format!("file not found: {trimmed}");
            }
            return "file not found".to_string();
        }
    }
    if skill_name.eq_ignore_ascii_case("crypto") {
        if let Some((exchange, detail)) = parse_crypto_account_access_error(err) {
            return crypto_account_access_error_observation(&exchange, &detail);
        }
    }
    err.trim().to_string()
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
            "Do not modify high-risk config files from a non-admin task.".to_string(),
            "Tell the user to use the Web admin console or an admin-authorized key.".to_string(),
        ],
    ))
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
            crate::truncate_for_log(&args.to_string())
        );
    }
    if let Some(rewrite) =
        crate::virtual_tools::rewrite_virtual_tool_call(&skill_name, args.clone())?
    {
        tracing::info!(
            "skill_virtual_dispatch requested_skill={} runtime_skill={} args={}",
            skill_name,
            rewrite.runtime_tool,
            crate::truncate_for_log(&rewrite.runtime_args.to_string())
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
                "Do not execute the blocked skill.".to_string(),
                "Explain that the current tools policy blocks this capability.".to_string(),
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
                "Do not execute skills that are not enabled.".to_string(),
                "Tell the user to enable the skill in config before retrying.".to_string(),
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
                "Do not execute skills disabled for the current agent.".to_string(),
                "Tell the user to enable the skill for this agent before retrying.".to_string(),
            ],
        ));
    }
    ensure_config_mutation_allowed(state, task, &skill_name, &args)?;

    let kind = state.skill_kind_for_dispatch(&skill_name);
    let kind_str = match kind {
        SkillKind::Builtin => "builtin",
        SkillKind::Runner => "runner",
        SkillKind::External => "external",
    };
    tracing::info!(
        "skill_dispatch skill={} kind={} branch={}",
        skill_name,
        kind_str,
        kind_str
    );

    match kind {
        SkillKind::Builtin => {
            return execute_builtin_skill_for_task(state, task, &skill_name, &args)
                .await
                .map(|text| SkillRunOutcome {
                    text,
                    notify: None,
                    validation: None,
                    extra: None,
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

    let args = inject_skill_memory_context(state, task, &skill_name, args);
    let args =
        ensure_default_output_dir_for_skill_args(&state.skill_rt.workspace_root, &skill_name, args);
    let source = match task_runtime_channel(state, task) {
        RuntimeChannel::Whatsapp => "whatsapp",
        RuntimeChannel::Telegram => "telegram",
        RuntimeChannel::Wechat => "wechat",
        RuntimeChannel::Feishu => "feishu",
        RuntimeChannel::Lark => "lark",
    };

    let value = match kind {
        SkillKind::External => {
            execute_external_skill(state, task, &skill_name, &args, &source).await?
        }
        SkillKind::Runner => {
            let runner_name = state.runner_name_for_skill(&skill_name);
            tracing::info!(
                "skill_dispatch skill={} runner_name={} kind=runner",
                skill_name,
                runner_name
            );
            run_skill_with_runner_once(
                state,
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
    let extra = value.get("extra").cloned();
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
