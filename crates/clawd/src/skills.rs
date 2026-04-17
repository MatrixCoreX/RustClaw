use claw_core::skill_registry::SkillKind;
use serde_json::Value;
use std::path::{Component, Path};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

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

pub(crate) fn canonical_skill_name(name: &str) -> &str {
    match name {
        "fs_rearch" | "fs-search" | "filesystem_search" | "file_search" | "search_files" => {
            "fs_search"
        }
        "package_install" | "pkg_manager" | "packages" => "package_manager",
        "module_install" | "install_modules" => "install_module",
        "process" | "process_manager" => "process_basic",
        "archive" | "archive_tool" => "archive_basic",
        "database" | "sqlite_tool" => "db_basic",
        "docker" | "docker_ops" => "docker_basic",
        "rss" | "rss_reader" | "rss_fetcher" => "rss_fetch",
        "image_vision_skill" | "vision" | "vision_image" | "image-analyze" => "image_vision",
        "image_generation" | "generate_image" | "draw_image" | "text_to_image" => "image_generate",
        "image_modify" | "image_editor" | "edit_image" | "image_outpaint" => "image_edit",
        "coin" | "coins" | "crypto_trade" | "market_data" | "crypto_market" => "crypto",
        "talk" | "smalltalk" | "joke" | "chitchat" => "chat",
        "git" => "git_basic",
        "http" => "http_basic",
        "system" => "system_basic",
        _ => name,
    }
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
                .policy.schedule
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
        "write_file" => args_path_targets_configs_dir(&state.skill_rt.workspace_root, args, "path", true),
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
    Err(config_requires_web_admin_message(state, task))
}

fn inject_skill_persona_context(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: Value,
) -> Value {
    if canonical_skill_name(skill_name) != "chat" {
        return args;
    }
    let mut obj = match args {
        Value::Object(map) => map,
        other => return other,
    };
    if obj.contains_key("persona_prompt") {
        return Value::Object(obj);
    }
    let persona_prompt = state.task_persona_prompt(task);
    let trimmed = persona_prompt.trim();
    if trimmed.is_empty() {
        return Value::Object(obj);
    }
    obj.insert(
        "persona_prompt".to_string(),
        Value::String(trimmed.to_string()),
    );
    Value::Object(obj)
}

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
            // Phase 2.2: chat 并入 clawd 内部，享受 LLM gateway 治理
            // （fallback / circuit breaker / 预算 / model_io.log）。
            // chat-skill 二进制保留，外部 caller 仍可独立 spawn，但 clawd
            // 内部 dispatch 一律走 builtin 实现，不再起子进程。
            | "chat"
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
        .skill_rt.tools_policy
        .is_allowed(&policy_token, state.core.active_provider_type.as_deref())
    {
        return Err(format!("blocked by policy: {policy_token}"));
    }

    if !state.get_skills_list().contains(&skill_name) {
        let mut allowed: Vec<String> = state.get_skills_list().iter().cloned().collect();
        allowed.sort();
        let enabled = allowed.join(", ");
        let err_text = crate::i18n_t_with_default(
            state,
            "clawd.msg.skill_disabled_with_enabled_list",
            "Skill is not enabled: {skill}. Please enable it in config and try again. (Currently enabled: {enabled_skills})",
        )
        .replace("{skill}", &skill_name)
        .replace("{enabled_skills}", &enabled);
        return Err(err_text);
    }
    if !state.task_allows_skill(task, &skill_name) {
        return Err(format!(
            "Skill is not enabled for agent {}: {}",
            state.task_agent_id(task),
            skill_name
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
        .skill_rt.skill_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| format!("skill semaphore closed: {err}"))?;

    let args = inject_skill_persona_context(state, task, &skill_name, args);
    let args = inject_skill_memory_context(state, task, &skill_name, args);
    let args = ensure_default_output_dir_for_skill_args(&state.skill_rt.workspace_root, &skill_name, args);
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
        extract_task_request_text, is_recoverable_skill_error, normalize_skill_error_for_user,
        request_reply_language, task_request_locale_tag, RequestReplyLanguage,
        READ_FILE_NOT_FOUND_PREFIX,
    };
    use crate::{
        runtime::state::ClaimedTask, AgentRuntimeConfig, AppState, CommandIntentRuntime, ScheduleRuntime, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
    };
    use claw_core::config::{
        AgentConfig, ToolsConfig,
    };
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
                                intent_prompt_template: String::new(),
                                intent_prompt_source: String::new(),
                                intent_rules_template: String::new(),
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
        assert!(!is_recoverable_skill_error("git_basic", "fatal: not a git repository"));
        assert!(!is_recoverable_skill_error("system_basic", "command not found"));
        assert!(!is_recoverable_skill_error("read_file", "some random error"));
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
    let llm_skill = canonical_skill_name == "chat";
    let user_key_for_skill = if llm_skill {
        Value::Null
    } else {
        task.user_key
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null)
    };
    let skill_context =
        build_runner_skill_context(state, task, source, llm_skill, credential_context);
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

    let selected_openai_model = crate::llm_gateway::selected_openai_model(state, Some(task));
    let mut child = Command::new(&state.skill_rt.skill_runner_path)
        .env("SKILL_TIMEOUT_SECONDS", skill_timeout_secs.to_string())
        .env(
            "OPENAI_API_KEY",
            crate::llm_gateway::selected_openai_api_key(state, Some(task)),
        )
        .env(
            "OPENAI_BASE_URL",
            crate::llm_gateway::selected_openai_base_url(state, Some(task)),
        )
        .env("OPENAI_MODEL", selected_openai_model.clone())
        .env("CHAT_SKILL_MODEL", selected_openai_model)
        .env("WORKSPACE_ROOT", state.skill_rt.workspace_root.display().to_string())
        .env(
            "RUSTCLAW_LOCATOR_SCAN_MAX_DEPTH",
            state.skill_rt.locator_scan_max_depth.to_string(),
        )
        .env(
            "RUSTCLAW_LOCATOR_SCAN_MAX_FILES",
            state.skill_rt.locator_scan_max_files.to_string(),
        )
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
    llm_skill: bool,
    credential_context: Value,
) -> Value {
    let mut ctx = serde_json::Map::new();
    ctx.insert("source".to_string(), Value::String(source.to_string()));
    ctx.insert("kind".to_string(), Value::String("run_skill".to_string()));
    ctx.insert(
        "user_key".to_string(),
        if llm_skill {
            Value::Null
        } else {
            task.user_key
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null)
        },
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
