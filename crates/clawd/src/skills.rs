use claw_core::skill_registry::SkillKind;
use serde_json::{json, Value};
use std::path::{Component, Path};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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

pub(crate) use builtin::{execute_builtin_skill_for_task, run_safe_command};
// `execute_builtin_skill`（无 task 版本）只在 `builtin.rs` 内部测试用，
// 不再向 crate 外暴露，避免再产生绕过 LLM 预算/日志的调用点。
// 详见 `builtin.rs` 上对 `execute_builtin_skill` 的注释。
pub(crate) use external::execute_external_skill;
pub(crate) use memory_context::inject_skill_memory_context;
pub(crate) use output_dirs::ensure_default_output_dir_for_skill_args;

use crate::worker::task_runtime_channel;
use crate::{AppState, ClaimedTask, RuntimeChannel};

const READ_FILE_NOT_FOUND_PREFIX: &str = "__RC_READ_FILE_NOT_FOUND__:";
const POLICY_BLOCK_ERROR_PREFIX: &str = "__RC_POLICY_BLOCK__:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicyBlockError {
    pub(crate) reason_code: String,
    pub(crate) observed_facts: Vec<String>,
    pub(crate) policy_boundary: Vec<String>,
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

fn policy_fact_value<'a>(facts: &'a [String], key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    facts
        .iter()
        .find_map(|fact| fact.trim().strip_prefix(&prefix).map(str::trim))
        .filter(|value| !value.is_empty())
}

pub(crate) fn policy_block_default_text(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    block: &PolicyBlockError,
) -> String {
    let prefer_english =
        crate::language_policy::task_response_language_hint(state, task, user_text) == "en";
    let fact = |key: &str| policy_fact_value(&block.observed_facts, key).unwrap_or("");
    match block.reason_code.as_str() {
        "path_parent_traversal" => {
            if prefer_english {
                "That path contains `..`, so I will not access it. Please provide a path inside the workspace.".to_string()
            } else {
                "这个路径包含 `..`，我不会访问它。请提供 workspace 内的明确路径。".to_string()
            }
        }
        "path_outside_workspace" => {
            let path = fact("denied_path");
            if prefer_english {
                if path.is_empty() {
                    "The requested path is outside the allowed workspace. Please provide a path inside the workspace or use an admin-authorized run.".to_string()
                } else {
                    format!("The requested path `{path}` is outside the allowed workspace. Please provide a workspace path or use an admin-authorized run.")
                }
            } else if path.is_empty() {
                "请求路径在允许的 workspace 外。请提供 workspace 内路径，或使用管理员授权运行。"
                    .to_string()
            } else {
                format!("请求路径 `{path}` 在允许的 workspace 外。请提供 workspace 内路径，或使用管理员授权运行。")
            }
        }
        "sudo_not_allowed" => {
            if prefer_english {
                "This task is not allowed to use sudo. Run clawd with an admin-authorized key and sudo-enabled policy if you need elevated access.".to_string()
            } else {
                "当前任务不允许使用 sudo。如果需要提权访问，请使用管理员 key 并开启 sudo 权限后运行。".to_string()
            }
        }
        "config_requires_web_admin" => config_requires_web_admin_message(state, task),
        "skill_policy_denied" => {
            let skill = fact("skill");
            if prefer_english {
                if skill.is_empty() {
                    "This capability is blocked by the current tools policy. Enable it in policy before retrying.".to_string()
                } else {
                    format!("The `{skill}` capability is blocked by the current tools policy. Enable it in policy before retrying.")
                }
            } else if skill.is_empty() {
                "当前工具策略阻止了这个能力。请在策略里开启后再试。".to_string()
            } else {
                format!("当前工具策略阻止了 `{skill}` 能力。请在策略里开启后再试。")
            }
        }
        "skill_disabled" | "agent_skill_disabled" => {
            let skill = fact("skill");
            if prefer_english {
                if skill.is_empty() {
                    "The required skill is not enabled for this run. Enable it in config and retry."
                        .to_string()
                } else {
                    format!("The `{skill}` skill is not enabled for this run. Enable it in config and retry.")
                }
            } else if skill.is_empty() {
                "这次运行需要的技能没有启用。请先在配置中开启后再试。".to_string()
            } else {
                format!("这次运行需要的 `{skill}` 技能没有启用。请先在配置中开启后再试。")
            }
        }
        _ => {
            if prefer_english {
                "This request is blocked by the current runtime policy. Adjust the policy or provide a safer target, then retry.".to_string()
            } else {
                "当前运行策略阻止了这个请求。请调整策略或提供更安全的目标后再试。".to_string()
            }
        }
    }
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
}

pub(crate) fn is_recoverable_skill_error(skill_name: &str, err: &str) -> bool {
    if skill_name.eq_ignore_ascii_case("read_file") && err.starts_with(READ_FILE_NOT_FOUND_PREFIX) {
        return true;
    }
    // system_basic 的 read 类 action 拿到 OS error（permission denied / not a directory / ...）
    // 时，把任务整体标 failed 不利于用户体验：本质是"读不到"语义，应该让 finalizer
    // 把错误 wrap 成自然语言对话回复（observed scalar fallback 路径），而不是丢出
    // raw OS error。判别基于 system_basic 的 read 错误前缀（见 system_basic/src/main.rs
    // 与 builtin.rs read_file 分支），不依赖 path / args。
    if skill_name.eq_ignore_ascii_case("system_basic")
        && (err.starts_with("read file failed:") || err.starts_with("read_file failed:"))
    {
        return true;
    }
    false
}

pub(crate) fn normalize_skill_error_for_user(skill_name: &str, err: &str) -> String {
    if let Some(policy_block) = parse_policy_block_error(err) {
        return format!("blocked by runtime policy: {}", policy_block.reason_code);
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
    if skill_name.eq_ignore_ascii_case("system_basic") {
        if let Some(rest) = err
            .strip_prefix("read file failed: ")
            .or_else(|| err.strip_prefix("read_file failed: "))
        {
            let trimmed = rest.trim();
            if trimmed.contains("Permission denied") {
                return "read file failed: permission denied (operating-system level)".to_string();
            }
            if trimmed.contains("Is a directory") {
                return "read file failed: target is a directory, not a regular file".to_string();
            }
            if trimmed.contains("No such file") || trimmed.contains("(os error 2)") {
                return "read file failed: file not found".to_string();
            }
        }
    }
    err.trim().to_string()
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

fn text_contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        )
    })
}

fn text_contains_ascii_alpha(text: &str) -> bool {
    text.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn request_reply_language(user_text: &str) -> RequestReplyLanguage {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return RequestReplyLanguage::ConfigDefault;
    }
    match (
        text_contains_cjk(trimmed),
        text_contains_ascii_alpha(trimmed),
    ) {
        (true, false) => RequestReplyLanguage::ZhCn,
        (false, true) => RequestReplyLanguage::En,
        (true, true) | (false, false) => RequestReplyLanguage::ConfigDefault,
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

fn config_requires_web_admin_message(state: &AppState, task: &ClaimedTask) -> String {
    const KEY: &str = "clawd.msg.config_requires_web_admin";
    const DEFAULT_ZH: &str = "这是高风险配置，请登录 Web 管理端修改。";
    const DEFAULT_EN: &str =
        "This is a high-risk configuration. Please sign in to the Web admin console to modify it.";

    match extract_task_request_text(&task.payload_json)
        .as_deref()
        .map(request_reply_language)
        .unwrap_or(RequestReplyLanguage::ConfigDefault)
    {
        RequestReplyLanguage::ZhCn => {
            crate::app_helpers::bilingual_t_with_default(state, KEY, DEFAULT_ZH, DEFAULT_EN, false)
        }
        RequestReplyLanguage::En => {
            crate::app_helpers::bilingual_t_with_default(state, KEY, DEFAULT_ZH, DEFAULT_EN, true)
        }
        RequestReplyLanguage::ConfigDefault => {
            if state
                .policy
                .schedule
                .locale
                .trim()
                .to_ascii_lowercase()
                .starts_with("en")
            {
                crate::i18n_t_with_default(state, KEY, DEFAULT_EN)
            } else {
                crate::i18n_t_with_default(state, KEY, DEFAULT_ZH)
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
    args: serde_json::Value,
) -> Result<SkillRunOutcome, String> {
    let skill_name = state.resolve_canonical_skill_name(skill_name);
    if skill_name.is_empty() {
        return Err("skill_name is empty".to_string());
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
                .map(|text| SkillRunOutcome { text, notify: None });
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
        return Err(value
            .get("error_text")
            .and_then(|v| v.as_str())
            .unwrap_or("skill execution failed")
            .to_string());
    }

    if let Some((provider, model, model_kind)) = extract_skill_provider_model(&value) {
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
    let text = value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(SkillRunOutcome { text, notify })
}

#[cfg(test)]
mod tests {
    use super::{
        collect_whitelisted_env_pairs, extract_task_request_text, is_recoverable_skill_error,
        normalize_skill_error_for_user, parse_policy_block_error, policy_block_default_text,
        policy_block_error, request_reply_language, skill_runner_env_strict_enabled,
        task_allows_path_outside_workspace, task_allows_sudo, task_request_locale_tag,
        RequestReplyLanguage, READ_FILE_NOT_FOUND_PREFIX, SKILL_RUNNER_ENV_WHITELIST,
    };
    use crate::{
        runtime::state::ClaimedTask, AgentRuntimeConfig, AppState, CommandIntentRuntime,
        ScheduleRuntime, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
    };
    use claw_core::config::{AgentConfig, ToolsConfig};
    use rusqlite::params;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};

    use std::sync::{Arc, RwLock};

    fn test_state(locale: &str) -> AppState {
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: None,
                    skills_list: Arc::new(HashSet::new()),
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                tools_policy: Arc::new(
                    ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
                ),
                ..crate::SkillRuntime::test_default()
            },
            policy: crate::PolicyConfig {
                command_intent: CommandIntentRuntime {
                    all_result_suffixes: Vec::new(),
                    default_locale: locale.to_string(),
                    verify_enforce_enabled: false,
                },
                schedule: ScheduleRuntime {
                    timezone: "Asia/Shanghai".to_string(),
                    intent_prompt_template: Arc::new(RwLock::new(String::new())),
                    intent_prompt_source: String::new(),
                    intent_rules_template: Arc::new(RwLock::new(String::new())),
                    locale: locale.to_string(),
                    i18n_dict: HashMap::new(),
                },
                ..crate::PolicyConfig::test_default()
            },
            worker: crate::WorkerConfig::test_default(),
            metrics: crate::TaskMetricsRegistry::default(),
            channels: crate::ChannelConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
            ask_states: crate::AskStateRegistry::default(),
        }
    }

    fn test_task(payload: serde_json::Value) -> ClaimedTask {
        ClaimedTask {
            task_id: "task-test".to_string(),
            user_id: 1,
            chat_id: 1,
            user_key: Some("rk-test".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: payload.to_string(),
        }
    }

    fn insert_auth_key(state: &AppState, user_key: &str, role: &str) {
        let db = state.core.db.get().expect("db pool");
        db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
            .expect("create auth schema");
        db.execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             VALUES (?1, ?2, 1, '123', NULL)",
            params![user_key, role],
        )
        .expect("insert auth key");
    }

    #[test]
    fn request_reply_language_prefers_english_for_ascii_requests() {
        assert_eq!(
            request_reply_language("open the config page"),
            RequestReplyLanguage::En
        );
    }

    #[test]
    fn request_reply_language_prefers_chinese_for_cjk_requests() {
        assert_eq!(
            request_reply_language("去改配置"),
            RequestReplyLanguage::ZhCn
        );
    }

    #[test]
    fn request_reply_language_falls_back_for_mixed_requests() {
        assert_eq!(
            request_reply_language("用 English 改配置"),
            RequestReplyLanguage::ConfigDefault
        );
    }

    #[test]
    fn extract_task_request_text_reads_top_level_text() {
        let payload = json!({
            "text": "please update the config"
        });
        assert_eq!(
            extract_task_request_text(&payload.to_string()).as_deref(),
            Some("please update the config")
        );
    }

    #[test]
    fn extract_task_request_text_reads_nested_request_text() {
        let payload = json!({
            "skill_name": "run_cmd",
            "args": {
                "request_text": "set the config flag"
            }
        });
        assert_eq!(
            extract_task_request_text(&payload.to_string()).as_deref(),
            Some("set the config flag")
        );
    }

    #[test]
    fn task_request_locale_tag_prefers_english_request_text() {
        let state = test_state("zh-CN");
        let task = test_task(json!({
            "text": "check my binance spot balances"
        }));
        assert_eq!(task_request_locale_tag(&state, &task), "en-US");
    }

    #[test]
    fn task_request_locale_tag_falls_back_to_schedule_locale() {
        let state = test_state("en-US");
        let task = test_task(json!({
            "text": "12345"
        }));
        assert_eq!(task_request_locale_tag(&state, &task), "en-US");
    }

    #[test]
    fn task_allows_privileged_tools_for_admin_only() {
        let mut state = test_state("zh-CN");
        state.policy.allow_sudo = true;
        state.policy.allow_path_outside_workspace = true;

        insert_auth_key(&state, "rk-admin", "admin");
        insert_auth_key(&state, "rk-user", "user");

        let mut admin_task = test_task(json!({ "text": "run privileged command" }));
        admin_task.user_key = Some("rk-admin".to_string());
        assert!(task_allows_sudo(&state, Some(&admin_task)));
        assert!(task_allows_path_outside_workspace(
            &state,
            Some(&admin_task)
        ));

        let mut user_task = test_task(json!({ "text": "run privileged command" }));
        user_task.user_key = Some("rk-user".to_string());
        assert!(!task_allows_sudo(&state, Some(&user_task)));
        assert!(!task_allows_path_outside_workspace(
            &state,
            Some(&user_task)
        ));
    }

    #[test]
    fn read_file_not_found_is_recoverable() {
        let err = format!("{}/etc/missing", READ_FILE_NOT_FOUND_PREFIX);
        assert!(is_recoverable_skill_error("read_file", &err));
        assert!(is_recoverable_skill_error("READ_FILE", &err));
        let normalized = normalize_skill_error_for_user("read_file", &err);
        assert!(normalized.contains("file not found"));
        assert!(normalized.contains("/etc/missing"));
    }

    #[test]
    fn system_basic_read_failures_are_recoverable() {
        let perm_err = "read file failed: Permission denied (os error 13)";
        let dir_err = "read file failed: Is a directory (os error 21)";
        let nf_err = "read file failed: No such file or directory (os error 2)";

        assert!(is_recoverable_skill_error("system_basic", perm_err));
        assert!(is_recoverable_skill_error("system_basic", dir_err));
        assert!(is_recoverable_skill_error("system_basic", nf_err));
        assert!(is_recoverable_skill_error("SYSTEM_BASIC", perm_err));

        let n1 = normalize_skill_error_for_user("system_basic", perm_err);
        assert!(n1.contains("permission denied"), "got: {n1}");
        let n2 = normalize_skill_error_for_user("system_basic", dir_err);
        assert!(n2.contains("directory"), "got: {n2}");
        let n3 = normalize_skill_error_for_user("system_basic", nf_err);
        assert!(n3.contains("file not found"), "got: {n3}");
    }

    #[test]
    fn other_skill_errors_are_not_recoverable() {
        assert!(!is_recoverable_skill_error(
            "git_basic",
            "fatal: not a git repository"
        ));
        assert!(!is_recoverable_skill_error(
            "system_basic",
            "command not found"
        ));
        assert!(!is_recoverable_skill_error(
            "read_file",
            "some random error"
        ));
    }

    #[test]
    fn policy_block_error_roundtrips_structured_payload() {
        let encoded = policy_block_error(
            "path_outside_workspace",
            vec!["denied_path: /etc/shadow".to_string()],
            vec!["Do not access the denied path.".to_string()],
        );
        let parsed = parse_policy_block_error(&encoded).expect("policy block payload");
        assert_eq!(parsed.reason_code, "path_outside_workspace");
        assert_eq!(parsed.observed_facts, vec!["denied_path: /etc/shadow"]);
        assert_eq!(
            parsed.policy_boundary,
            vec!["Do not access the denied path."]
        );
        assert_eq!(
            normalize_skill_error_for_user("read_file", &encoded),
            "blocked by runtime policy: path_outside_workspace"
        );
    }

    #[test]
    fn policy_block_default_text_uses_request_language() {
        let state = test_state("zh-CN");
        let task = test_task(json!({
            "text": "读取 /etc/shadow 第一行"
        }));
        let encoded = policy_block_error(
            "path_outside_workspace",
            vec!["denied_path: /etc/shadow".to_string()],
            Vec::new(),
        );
        let parsed = parse_policy_block_error(&encoded).expect("policy block payload");
        let text = policy_block_default_text(&state, &task, "读取 /etc/shadow 第一行", &parsed);
        assert!(text.contains("/etc/shadow"));
        assert!(text.contains("workspace"));

        let english_task = test_task(json!({
            "text": "Read the first line of /etc/shadow"
        }));
        let english = policy_block_default_text(
            &state,
            &english_task,
            "Read the first line of /etc/shadow",
            &parsed,
        );
        assert!(english.contains("outside the allowed workspace"));
    }

    // §E2 step1 ===============================================================
    // 抽象 helper 才能稳定测：apply_skill_runner_env_isolation 直接读 std::env::vars()
    // 在并发测试里读到的是 cargo runner 的环境，没法稳定断言；所以靠 collect 函数 +
    // 显式 source map 验证白名单语义本身。

    #[test]
    fn skill_env_strict_off_when_env_unset_or_empty() {
        // 暂存 + 清掉避免邻测污染
        let prev = std::env::var_os("RUSTCLAW_SKILL_ENV_STRICT");
        std::env::remove_var("RUSTCLAW_SKILL_ENV_STRICT");
        assert!(!skill_runner_env_strict_enabled(), "默认应 OFF");

        std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", "");
        assert!(!skill_runner_env_strict_enabled(), "空字符串视为 OFF");

        std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", "0");
        assert!(!skill_runner_env_strict_enabled(), "\"0\" 视为 OFF");

        std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", "false");
        assert!(!skill_runner_env_strict_enabled(), "\"false\" 视为 OFF");

        // 恢复
        match prev {
            Some(v) => std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", v),
            None => std::env::remove_var("RUSTCLAW_SKILL_ENV_STRICT"),
        }
    }

    #[test]
    fn skill_env_strict_on_for_truthy_values() {
        let prev = std::env::var_os("RUSTCLAW_SKILL_ENV_STRICT");
        for val in ["1", "true", "TRUE", "True", "on", "yes"] {
            std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", val);
            assert!(
                skill_runner_env_strict_enabled(),
                "RUSTCLAW_SKILL_ENV_STRICT={val:?} 应被识别为 ON"
            );
        }
        match prev {
            Some(v) => std::env::set_var("RUSTCLAW_SKILL_ENV_STRICT", v),
            None => std::env::remove_var("RUSTCLAW_SKILL_ENV_STRICT"),
        }
    }

    #[test]
    fn whitelist_keeps_only_listed_keys_and_drops_secrets_or_unknown() {
        let source = vec![
            ("PATH", "/usr/bin:/bin"),
            ("HOME", "/home/u"),
            ("LANG", "en_US.UTF-8"),
            // 以下都不在白名单，必须被剥离
            ("OPENAI_API_KEY", "sk-fake-leak"),
            ("MINIMAX_API_KEY", "sk-fake-leak2"),
            ("MIMO_API_KEY", "sk-fake-leak3"),
            ("XIAOMI_API_KEY", "sk-fake-leak4"),
            ("RUSTCLAW_USER_KEY", "rk-leak"),
            ("DATABASE_URL", "postgres://leak"),
            ("AWS_ACCESS_KEY_ID", "AKIA..."),
        ];
        let kept = collect_whitelisted_env_pairs(source);
        let kept_keys: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(kept_keys, vec!["HOME", "LANG", "PATH"], "字典序 + 仅白名单");
        for (k, _) in &kept {
            assert!(SKILL_RUNNER_ENV_WHITELIST.contains(&k.as_str()));
        }
    }

    #[test]
    fn whitelist_drops_empty_value_to_avoid_silent_propagation() {
        let source = vec![
            ("PATH", "/usr/bin"),
            ("HOME", ""), // 空值不传，避免 skill 拿到 "" 又 fail-loud
            ("LC_ALL", "C"),
        ];
        let kept = collect_whitelisted_env_pairs(source);
        let kept_keys: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(kept_keys, vec!["LC_ALL", "PATH"]);
    }

    #[test]
    fn whitelist_does_not_invent_keys_for_missing_source() {
        let source: Vec<(&str, &str)> = vec![("UNRELATED", "x")];
        let kept = collect_whitelisted_env_pairs(source);
        assert!(kept.is_empty(), "没有白名单匹配时不应注入任何 env");
    }

    #[test]
    fn whitelist_constant_does_not_include_obvious_secrets_or_clawd_specific_keys() {
        // §E2 step1 防回归：白名单不能不小心放进 API key / RustClaw 专属变量。
        let banned = [
            "OPENAI_API_KEY",
            "MINIMAX_API_KEY",
            "MIMO_API_KEY",
            "XIAOMI_API_KEY",
            "QWEN_API_KEY",
            "ANTHROPIC_API_KEY",
            "RUSTCLAW_USER_KEY",
            "RUSTCLAW_ADMIN_KEY",
            "DATABASE_URL",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
        ];
        for needle in banned {
            assert!(
                !SKILL_RUNNER_ENV_WHITELIST.contains(&needle),
                "{needle} 不能进白名单 —— 必须走 secrets broker 或 clawd 显式 env 注入"
            );
        }
    }
}

fn extract_skill_provider_model(value: &Value) -> Option<(String, String, String)> {
    let extra = value.get("extra")?.as_object()?;
    let provider = extra
        .get("provider")
        .or_else(|| extra.get("vendor"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let model = extra
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let model_kind = extra
        .get("model_kind")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("unknown");
    Some((
        provider.to_string(),
        model.to_string(),
        model_kind.to_string(),
    ))
}

pub(crate) async fn run_skill_with_runner(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: Value,
) -> Result<String, String> {
    run_skill_with_runner_outcome(state, task, skill_name, args)
        .await
        .map(|r| r.text)
}

async fn read_skill_runner_stderr_line(stderr: &mut Option<tokio::process::ChildStderr>) -> String {
    let Some(stderr) = stderr.take() else {
        return String::new();
    };
    let mut err_reader = BufReader::new(stderr);
    let mut err_line = String::new();
    let _ = tokio::time::timeout(
        Duration::from_millis(200),
        err_reader.read_line(&mut err_line),
    )
    .await;
    err_line
}

pub(crate) async fn run_skill_with_runner_once(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    runner_name: &str,
    args: &serde_json::Value,
    source: &str,
    skill_timeout_secs: u64,
) -> Result<serde_json::Value, String> {
    let credential_context = if canonical_skill_name == "crypto" {
        exchange_credential_context_for_task(state, task)
    } else {
        serde_json::json!({})
    };
    let user_key_for_skill = task
        .user_key
        .clone()
        .map(Value::String)
        .unwrap_or(Value::Null);
    let skill_context = build_runner_skill_context(state, task, source, credential_context);
    let req_line = serde_json::json!({
        "request_id": task.task_id,
        "user_id": task.user_id,
        "chat_id": task.chat_id,
        "user_key": user_key_for_skill,
        "external_user_id": task.external_user_id,
        "external_chat_id": crate::task_external_chat_id(task),
        "skill_name": runner_name,
        "args": args,
        "context": skill_context
    })
    .to_string();

    if !state.skill_rt.skill_runner_path.exists() {
        return Err(format!(
            "skill-runner binary not found: path={} (workspace_root={})",
            state.skill_rt.skill_runner_path.display(),
            state.skill_rt.workspace_root.display()
        ));
    }

    // §E1.b: 按 manifest capabilities 注入 secrets env。fail-loud：声明了
    // 但 broker 找不到 ⇒ 直接拒绝 spawn，绝不让 skill 拿空字符串去打 vendor。
    // 当前 manifest 里没有任何 skill 声明 `secrets.*`（image_generate 走的是
    // 父进程 env 继承），所以 provisioned 大概率为空 ⇒ 行为零变化；下一步
    // §E1.c 给 image_generate 声明 secrets.image_generation_<vendor>_api_key
    // 时本路径自动接管。
    let secret_envs = {
        let caps: Vec<claw_core::skill_registry::Capability> = state
            .get_skills_registry()
            .as_ref()
            .map(|reg| reg.capabilities(canonical_skill_name).to_vec())
            .unwrap_or_default();
        let broker = claw_core::secrets::global_or_default();
        match claw_core::secrets::provision_secret_envs(broker.as_ref(), &caps) {
            Ok(pairs) => {
                if !pairs.is_empty() {
                    let names: Vec<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
                    tracing::info!(
                        "skill_dispatch skill={} provisioned_secrets={:?} broker={}",
                        canonical_skill_name,
                        names,
                        broker.label()
                    );
                }
                pairs
            }
            Err(claw_core::secrets::ProvisionError::MissingSecrets { missing }) => {
                let env_names: Vec<String> =
                    missing.iter().map(|n| n.to_ascii_uppercase()).collect();
                tracing::error!(
                    "skill_dispatch skill={} missing_secrets={:?} broker={} — refuse to spawn",
                    canonical_skill_name,
                    env_names,
                    broker.label()
                );
                return Err(format!(
                    "skill `{canonical_skill_name}` declared secrets but broker `{}` is missing: {} (set the corresponding env var(s) and retry)",
                    broker.label(),
                    env_names.join(", ")
                ));
            }
            Err(claw_core::secrets::ProvisionError::Lookup { name, source }) => {
                tracing::error!(
                    "skill_dispatch skill={} secret_lookup_failed name={} err={} broker={}",
                    canonical_skill_name,
                    name,
                    source,
                    broker.label()
                );
                return Err(format!(
                    "skill `{canonical_skill_name}` secret `{name}` lookup failed via broker `{}`: {source}",
                    broker.label()
                ));
            }
        }
    };

    let selected_openai_model = crate::llm_gateway::selected_openai_model(state, Some(task));
    let secret_token_ttl = Duration::from_secs(300);
    let tokenized_secret_envs =
        match claw_core::secrets::issue_secret_env_tokens(&secret_envs, secret_token_ttl) {
            Ok(pairs) => pairs,
            Err(err) => {
                return Err(format!(
                "skill `{canonical_skill_name}` failed to issue short-lived secret tokens: {err}"
            ));
            }
        };
    let selected_openai_api_key = crate::llm_gateway::selected_openai_api_key(state, Some(task));
    let openai_api_key_token = if selected_openai_api_key.trim().is_empty() {
        None
    } else {
        match claw_core::secrets::issue_secret_token_value(
            &claw_core::secrets::SecretValue::new(selected_openai_api_key),
            secret_token_ttl,
        ) {
            Ok(token) => Some(token),
            Err(err) => {
                return Err(format!(
                    "skill `{canonical_skill_name}` failed to mint OPENAI_API_KEY token: {err}"
                ));
            }
        }
    };
    let mut cmd = Command::new(&state.skill_rt.skill_runner_path);
    // §E2 step1: 严格模式下先 env_clear + 白名单，让后续 `.env(...)` / secrets 注入
    // 成为子进程 env 的唯一来源。默认 OFF，行为与历史一致。
    if let Some(report) = apply_skill_runner_env_isolation(&mut cmd) {
        tracing::info!(
            "skill_dispatch skill={} env_strict=on preserved={:?} stripped_parent_env={}",
            canonical_skill_name,
            report.preserved,
            report.stripped_count
        );
    }
    cmd.env("SKILL_TIMEOUT_SECONDS", skill_timeout_secs.to_string())
        .env(
            "RUSTCLAW_SECRET_TOKEN_DIR",
            claw_core::secrets::secret_token_store_dir()
                .display()
                .to_string(),
        )
        .env(
            "OPENAI_BASE_URL",
            crate::llm_gateway::selected_openai_base_url(state, Some(task)),
        )
        .env("OPENAI_MODEL", selected_openai_model.clone())
        .env(
            "WORKSPACE_ROOT",
            state.skill_rt.workspace_root.display().to_string(),
        )
        .env(
            "RUSTCLAW_LOCATOR_SCAN_MAX_DEPTH",
            state.skill_rt.locator_scan_max_depth.to_string(),
        )
        .env(
            "RUSTCLAW_LOCATOR_SCAN_MAX_FILES",
            state.skill_rt.locator_scan_max_files.to_string(),
        );
    if let Some(token) = &openai_api_key_token {
        cmd.env("OPENAI_API_KEY", token);
    }
    // §E1.b: secrets 在最后注入，确保覆盖任何上面无意命中的同名硬编码键
    // （目前已经覆盖 OPENAI_API_KEY：这里与 manifest secrets.* 一起统一变成
    // 短期 token，而不是把明文 secret 直接塞进 child env）。
    for (env_name, token) in &tokenized_secret_envs {
        cmd.env(env_name, token);
    }
    cmd.current_dir(&state.skill_rt.workspace_root);
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| {
            format!(
                "spawn skill-runner failed: path={} err={}",
                state.skill_rt.skill_runner_path.display(),
                err
            )
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{req_line}\n").as_bytes())
            .await
            .map_err(|err| format!("write skill-runner stdin failed: {err}"))?;
        stdin
            .flush()
            .await
            .map_err(|err| format!("flush skill-runner stdin failed: {err}"))?;
    }

    let mut out_line = String::new();
    let mut stderr = child.stderr.take();

    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let read_out = tokio::time::timeout(
            Duration::from_secs(skill_timeout_secs.max(1)),
            reader.read_line(&mut out_line),
        )
        .await;

        match read_out {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => return Err(format!("read skill-runner stdout failed: {err}")),
            Err(_) => {
                let _ = child.kill().await;
                return Err("skill-runner timeout".to_string());
            }
        }
    }

    let wait_result = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    let mut err_line = String::new();

    match wait_result {
        Ok(Ok(_)) => {
            err_line = read_skill_runner_stderr_line(&mut stderr).await;
        }
        Ok(Err(err)) => {
            err_line = read_skill_runner_stderr_line(&mut stderr).await;
            if out_line.trim().is_empty() {
                let detail = err_line.trim();
                if detail.is_empty() {
                    return Err(format!("wait skill-runner failed: {err}"));
                }
                return Err(format!("wait skill-runner failed: {err}; stderr: {detail}"));
            }
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = tokio::time::timeout(Duration::from_millis(200), child.wait()).await;
            if out_line.trim().is_empty() {
                err_line = read_skill_runner_stderr_line(&mut stderr).await;
                let detail = err_line.trim();
                if detail.is_empty() {
                    return Err("skill-runner exit wait timeout".to_string());
                }
                return Err(format!("skill-runner exit wait timeout: {detail}"));
            }
        }
    }

    if out_line.trim().is_empty() {
        let detail = err_line.trim();
        if detail.is_empty() {
            return Err("empty skill-runner output".to_string());
        }
        return Err(format!("empty skill-runner output: {detail}"));
    }

    serde_json::from_str(out_line.trim()).map_err(|err| format!("invalid skill-runner json: {err}"))
}

pub(crate) fn build_runner_skill_context(
    state: &AppState,
    task: &ClaimedTask,
    source: &str,
    credential_context: Value,
) -> Value {
    let mut ctx = serde_json::Map::new();
    ctx.insert("source".to_string(), Value::String(source.to_string()));
    ctx.insert("kind".to_string(), Value::String("run_skill".to_string()));
    let auth_role = current_task_auth_role(state, task).unwrap_or_else(|| "unknown".to_string());
    let allow_path_outside_workspace = task_allows_path_outside_workspace(state, Some(task));
    let allow_sudo = task_allows_sudo(state, Some(task));
    ctx.insert("auth_role".to_string(), Value::String(auth_role));
    ctx.insert(
        "allow_path_outside_workspace".to_string(),
        Value::Bool(allow_path_outside_workspace),
    );
    ctx.insert("allow_sudo".to_string(), Value::Bool(allow_sudo));
    ctx.insert(
        "permissions".to_string(),
        serde_json::json!({
            "allow_path_outside_workspace": allow_path_outside_workspace,
            "allow_sudo": allow_sudo,
        }),
    );
    ctx.insert(
        "user_key".to_string(),
        task.user_key
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    ctx.insert("exchange_credentials".to_string(), credential_context);
    let locale_tag = task_request_locale_tag(state, task);
    ctx.insert("locale".to_string(), Value::String(locale_tag.clone()));
    ctx.insert("language".to_string(), Value::String(locale_tag));
    ctx.insert(
        "workspace_root".to_string(),
        Value::String(state.skill_rt.workspace_root.display().to_string()),
    );
    ctx.insert(
        "database_sqlite_path".to_string(),
        Value::String(state.worker.database_sqlite_path.display().to_string()),
    );
    ctx.insert(
        "database_busy_timeout_ms".to_string(),
        Value::from(state.worker.database_busy_timeout_ms),
    );

    let recent_images = crate::collect_recent_image_candidates(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        200,
    );
    ctx.insert(
        "recent_image_paths".to_string(),
        Value::Array(
            recent_images
                .into_iter()
                .map(Value::String)
                .collect::<Vec<_>>(),
        ),
    );

    if let Ok(payload) = serde_json::from_str::<Value>(&task.payload_json) {
        if let Some(p) = payload.as_object() {
            for key in [
                "schedule_job_id",
                "invocation_source",
                "scheduled",
                "schedule_triggered",
            ] {
                if let Some(v) = p.get(key) {
                    ctx.insert(key.to_string(), v.clone());
                }
            }
        }
    }
    Value::Object(ctx)
}

pub(crate) fn exchange_credential_context_for_task(
    state: &AppState,
    task: &ClaimedTask,
) -> serde_json::Value {
    let Some(user_key) = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return serde_json::json!({});
    };
    let Ok(db) = state.core.db.get() else {
        return serde_json::json!({});
    };
    let mut stmt = match db.prepare(
        "SELECT exchange, api_key, api_secret, passphrase
         FROM exchange_api_credentials
         WHERE user_key = ?1 AND enabled = 1",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return serde_json::json!({}),
    };
    let rows = match stmt.query_map(rusqlite::params![user_key], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return serde_json::json!({}),
    };
    let mut exchanges = serde_json::Map::new();
    for row in rows.flatten() {
        let (exchange, api_key, api_secret, passphrase) = row;
        exchanges.insert(
            exchange,
            serde_json::json!({
                "api_key": api_key,
                "api_secret": api_secret,
                "passphrase": passphrase,
            }),
        );
    }
    Value::Object(exchanges)
}
