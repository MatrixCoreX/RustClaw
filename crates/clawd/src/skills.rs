use claw_core::skill_registry::SkillKind;
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

mod builtin;
mod external;
mod memory_context;
mod output_dirs;

pub(crate) use builtin::{execute_builtin_skill, execute_builtin_skill_for_task, run_safe_command};
pub(crate) use external::execute_external_skill;
pub(crate) use memory_context::inject_skill_memory_context;
pub(crate) use output_dirs::ensure_default_output_dir_for_skill_args;

use crate::worker::task_runtime_channel;
use crate::{AppState, ClaimedTask, RuntimeChannel};

const READ_FILE_NOT_FOUND_PREFIX: &str = "__RC_READ_FILE_NOT_FOUND__:";

#[derive(Debug, Clone)]
pub(crate) struct SkillRunOutcome {
    pub(crate) text: String,
    pub(crate) notify: Option<bool>,
}

pub(crate) fn is_recoverable_skill_error(skill_name: &str, err: &str) -> bool {
    skill_name.eq_ignore_ascii_case("read_file") && err.starts_with(READ_FILE_NOT_FOUND_PREFIX)
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
        .tools_policy
        .is_allowed(&policy_token, state.active_provider_type.as_deref())
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
                Some(state.skill_timeout_seconds.max(s))
            } else {
                None
            }
        })
        .unwrap_or_else(|| match skill_name.as_str() {
            "image_generate" | "image_edit" => state.skill_timeout_seconds.max(180),
            "image_vision" => state.skill_timeout_seconds.max(90),
            "audio_transcribe" => state.skill_timeout_seconds.max(120),
            "audio_synthesize" => state.skill_timeout_seconds.max(90),
            "crypto" => state.skill_timeout_seconds.max(60),
            _ => state.skill_timeout_seconds,
        });

    let _permit = state
        .skill_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| format!("skill semaphore closed: {err}"))?;

    let args = inject_skill_persona_context(state, task, &skill_name, args);
    let args = inject_skill_memory_context(state, task, &skill_name, args);
    let args = ensure_default_output_dir_for_skill_args(&state.workspace_root, &skill_name, args);
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
    let _ = tokio::time::timeout(Duration::from_millis(200), err_reader.read_line(&mut err_line))
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

    if !state.skill_runner_path.exists() {
        return Err(format!(
            "skill-runner binary not found: path={} (workspace_root={})",
            state.skill_runner_path.display(),
            state.workspace_root.display()
        ));
    }

    let selected_openai_model = crate::llm_gateway::selected_openai_model(state, Some(task));
    let mut child = Command::new(&state.skill_runner_path)
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
        .env("WORKSPACE_ROOT", state.workspace_root.display().to_string())
        .env(
            "RUSTCLAW_LOCATOR_SCAN_MAX_DEPTH",
            state.locator_scan_max_depth.to_string(),
        )
        .env(
            "RUSTCLAW_LOCATOR_SCAN_MAX_FILES",
            state.locator_scan_max_files.to_string(),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| {
            format!(
                "spawn skill-runner failed: path={} err={}",
                state.skill_runner_path.display(),
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
    ctx.insert(
        "workspace_root".to_string(),
        Value::String(state.workspace_root.display().to_string()),
    );
    ctx.insert(
        "database_sqlite_path".to_string(),
        Value::String(state.database_sqlite_path.display().to_string()),
    );
    ctx.insert(
        "database_busy_timeout_ms".to_string(),
        Value::from(state.database_busy_timeout_ms),
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
    let Ok(db) = state.db.lock() else {
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
