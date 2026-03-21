use claw_core::skill_registry::SkillKind;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

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

    let args = inject_skill_memory_context(state, task, &skill_name, args);
    let args = ensure_default_output_dir_for_skill_args(&state.workspace_root, &skill_name, args);
    let source = match task_runtime_channel(state, task) {
        RuntimeChannel::Whatsapp => "whatsapp",
        RuntimeChannel::Telegram => "telegram",
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

pub(crate) async fn execute_external_skill(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    args: &Value,
    source: &str,
) -> Result<Value, String> {
    let reg = state
        .get_skills_registry()
        .ok_or_else(|| "external skill requires registry".to_string())?;
    let config = reg
        .external_config(canonical_skill_name)
        .ok_or_else(|| "external skill missing external_kind in registry".to_string())?;
    match config.kind {
        "http_json" => {
            execute_external_http_json(state, task, canonical_skill_name, args, source).await
        }
        "local_shell_recipe" => {
            execute_external_local_shell_recipe(state, task, canonical_skill_name, args, source)
                .await
        }
        "local_script" => {
            execute_external_local_script(state, task, canonical_skill_name, args, source).await
        }
        "prompt_bundle" => Ok(serde_json::json!({
            "request_id": task.task_id,
            "status": "error",
            "text": "",
            "error_text": format!(
                "Imported external skill preview is registered, but runtime execution for external_kind={} is not enabled yet.",
                config.kind
            )
        })),
        other => Err(format!("external_kind not supported: {other}")),
    }
}

pub(crate) fn external_reserved_arg_key(key: &str) -> bool {
    if key.starts_with('_') {
        return true;
    }
    matches!(
        key,
        "action"
            | "output_dir"
            | "response_language"
            | "language"
            | "confirm"
            | "dry_run"
            | "timeout_seconds"
            | "source"
            | "skill_name"
    )
}

pub(crate) fn value_to_cli_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn build_external_cli_args(args: &Value) -> Vec<String> {
    if let Some(cli_args) = args.get("cli_args").and_then(|v| v.as_array()) {
        let collected: Vec<String> = cli_args
            .iter()
            .filter_map(|value| value.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect();
        if !collected.is_empty() {
            return collected;
        }
    }

    for key in ["command", "script", "recipe"] {
        if let Some(raw) = args.get(key).and_then(|v| v.as_str()) {
            let collected = raw
                .split_whitespace()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            if !collected.is_empty() {
                return collected;
            }
        }
    }

    let Some(map) = args.as_object() else {
        return Vec::new();
    };

    let mut cli_args = Vec::new();
    for (key, value) in map {
        if external_reserved_arg_key(key) || key == "cli_args" {
            continue;
        }
        let flag = format!("--{}", key.replace('_', "-"));
        match value {
            Value::Bool(true) => cli_args.push(flag),
            Value::Bool(false) | Value::Null => {}
            Value::Array(items) => {
                for item in items {
                    cli_args.push(flag.clone());
                    cli_args.push(value_to_cli_string(item));
                }
            }
            other => {
                cli_args.push(flag);
                cli_args.push(value_to_cli_string(other));
            }
        }
    }
    cli_args
}

pub(crate) fn resolve_external_bundle_dir(
    state: &AppState,
    bundle_rel: &str,
) -> Result<PathBuf, String> {
    if bundle_rel.trim().is_empty() {
        return Err("external skill missing external_bundle_dir".to_string());
    }
    let joined = state.workspace_root.join(bundle_rel);
    let canonical = joined
        .canonicalize()
        .map_err(|err| format!("external bundle directory not found: {err}"))?;
    if !canonical.starts_with(&state.workspace_root) {
        return Err("external bundle directory must stay inside workspace_root".to_string());
    }
    Ok(canonical)
}

pub(crate) fn resolve_external_entry_path(
    bundle_dir: &Path,
    entry_rel: &str,
) -> Result<PathBuf, String> {
    if entry_rel.trim().is_empty() {
        return Err("external skill missing external_entry_file".to_string());
    }
    let entry_path = bundle_dir.join(entry_rel);
    let canonical = entry_path
        .canonicalize()
        .map_err(|err| format!("external entry file not found: {err}"))?;
    if !canonical.starts_with(bundle_dir) {
        return Err("external entry file must stay inside the imported bundle".to_string());
    }
    Ok(canonical)
}

pub(crate) fn is_bin_available(bin: &str) -> bool {
    let bin = bin.trim();
    if bin.is_empty() {
        return false;
    }
    if bin.contains('/') {
        return Path::new(bin).is_file();
    }
    let Some(path_env) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_env).any(|dir| dir.join(bin).is_file())
}

pub(crate) async fn verify_external_python_modules(
    runtime: &str,
    modules: &[String],
    bundle_dir: &Path,
) -> Result<(), String> {
    if modules.is_empty() {
        return Ok(());
    }
    let imports = modules.join(",");
    let mut cmd = Command::new(runtime);
    cmd.arg("-c")
        .arg(format!("import {imports}"))
        .current_dir(bundle_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .map_err(|_| "checking Python dependencies timed out".to_string())?
        .map_err(|err| format!("checking Python dependencies failed: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(format!(
        "missing Python dependencies for imported skill: {}{}",
        modules.join(", "),
        if detail.is_empty() {
            String::new()
        } else {
            format!(" ({detail})")
        }
    ))
}

pub(crate) async fn execute_external_local_script(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    args: &Value,
    source: &str,
) -> Result<Value, String> {
    let reg = state
        .get_skills_registry()
        .ok_or_else(|| "external skill requires registry".to_string())?;
    let config = reg
        .external_config(canonical_skill_name)
        .ok_or_else(|| "external skill missing execution config".to_string())?;
    if config.kind != "local_script" {
        return Err(format!(
            "external_kind not supported by local_script executor: {}",
            config.kind
        ));
    }

    for bin in config.require_bins {
        if !is_bin_available(bin) {
            return Err(format!(
                "missing required local command for imported skill: {}",
                bin
            ));
        }
    }

    let bundle_dir = resolve_external_bundle_dir(state, config.bundle_dir.unwrap_or_default())?;
    let entry_rel = config
        .entry_file
        .ok_or_else(|| "external skill missing external_entry_file".to_string())?;
    let entry_path = resolve_external_entry_path(&bundle_dir, entry_rel)?;
    let runtime = config
        .runtime
        .map(str::to_string)
        .or_else(|| {
            if entry_rel.ends_with(".py") {
                Some("python3".to_string())
            } else if entry_rel.ends_with(".js")
                || entry_rel.ends_with(".mjs")
                || entry_rel.ends_with(".cjs")
            {
                Some("node".to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| "external skill missing external_runtime".to_string())?;

    if runtime.starts_with("python") {
        verify_external_python_modules(&runtime, config.require_py_modules, &bundle_dir).await?;
    }

    let cli_args = build_external_cli_args(args);
    tracing::info!(
        "skill_dispatch external skill={} external_kind=local_script runtime={} entry={} cli_args={:?} source={}",
        canonical_skill_name,
        runtime,
        entry_rel,
        cli_args,
        source
    );

    let timeout_secs = config
        .timeout_seconds
        .unwrap_or(state.skill_timeout_seconds)
        .max(1);
    let entry_arg = entry_path
        .strip_prefix(&bundle_dir)
        .unwrap_or(&entry_path)
        .to_string_lossy()
        .to_string();

    let mut cmd = Command::new(&runtime);
    cmd.arg(&entry_arg);
    for arg in &cli_args {
        cmd.arg(arg);
    }
    cmd.current_dir(&bundle_dir)
        .env("WORKSPACE_ROOT", state.workspace_root.display().to_string())
        .env("RUSTCLAW_IMPORTED_SKILL", canonical_skill_name)
        .env("RUSTCLAW_TASK_ID", task.task_id.clone())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| format!("imported external skill timed out after {}s", timeout_secs))?
        .map_err(|err| format!("run imported external skill failed: {err}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() {
        let text = if !stdout.is_empty() && !stderr.is_empty() {
            format!("{stdout}\n\n{stderr}")
        } else if !stdout.is_empty() {
            stdout
        } else if !stderr.is_empty() {
            stderr
        } else {
            "Imported external skill completed with no output.".to_string()
        };
        return Ok(serde_json::json!({
            "request_id": task.task_id,
            "status": "ok",
            "text": text,
            "error_text": Value::Null,
            "extra": {
                "external_kind": config.kind,
                "runtime": runtime,
                "entry_file": entry_rel,
                "cli_args": cli_args,
            }
        }));
    }

    let exit_code = output.status.code().unwrap_or(-1);
    let mut detail = String::new();
    if !stderr.is_empty() {
        detail.push_str(&stderr);
    }
    if !stdout.is_empty() {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str(&stdout);
    }
    if detail.is_empty() {
        detail = format!("process exited with code {}", exit_code);
    }

    Ok(serde_json::json!({
        "request_id": task.task_id,
        "status": "error",
        "text": "",
        "error_text": format!("Imported external skill failed (exit={}): {}", exit_code, detail),
        "extra": {
            "external_kind": config.kind,
            "runtime": runtime,
            "entry_file": entry_rel,
            "cli_args": cli_args,
        }
    }))
}

pub(crate) fn extract_external_shell_command(args: &Value) -> Result<String, String> {
    if let Some(command) = args.get("command").and_then(|v| v.as_str()) {
        let trimmed = command.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(command) = args.get("script").and_then(|v| v.as_str()) {
        let trimmed = command.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(command) = args.get("recipe").and_then(|v| v.as_str()) {
        let trimmed = command.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    Err(
        "Imported shell skill needs a command string in args.command (or args.script / args.recipe)."
            .to_string(),
    )
}

pub(crate) async fn execute_external_local_shell_recipe(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    args: &Value,
    source: &str,
) -> Result<Value, String> {
    let reg = state
        .get_skills_registry()
        .ok_or_else(|| "external skill requires registry".to_string())?;
    let config = reg
        .external_config(canonical_skill_name)
        .ok_or_else(|| "external skill missing execution config".to_string())?;
    if config.kind != "local_shell_recipe" {
        return Err(format!(
            "external_kind not supported by local_shell_recipe executor: {}",
            config.kind
        ));
    }

    for bin in config.require_bins {
        if !is_bin_available(bin) {
            return Err(format!(
                "missing required local command for imported skill: {}",
                bin
            ));
        }
    }

    let bundle_dir = resolve_external_bundle_dir(state, config.bundle_dir.unwrap_or_default())?;
    let command = extract_external_shell_command(args)?;
    let timeout_secs = config
        .timeout_seconds
        .unwrap_or(state.cmd_timeout_seconds)
        .max(1);

    tracing::info!(
        "skill_dispatch external skill={} external_kind=local_shell_recipe command={} source={}",
        canonical_skill_name,
        crate::truncate_for_log(&command),
        source
    );

    match run_safe_command(
        &bundle_dir,
        &command,
        state.max_cmd_length,
        timeout_secs,
        state.allow_sudo,
    )
    .await
    {
        Ok(text) => Ok(serde_json::json!({
            "request_id": task.task_id,
            "status": "ok",
            "text": text,
            "error_text": Value::Null,
            "extra": {
                "external_kind": config.kind,
                "command": command,
            }
        })),
        Err(err) => Ok(serde_json::json!({
            "request_id": task.task_id,
            "status": "error",
            "text": "",
            "error_text": format!("Imported shell skill failed: {err}"),
            "extra": {
                "external_kind": config.kind,
                "command": command,
            }
        })),
    }
}

pub(crate) fn resolve_external_auth(
    auth_ref: Option<&str>,
) -> Result<Option<(String, String)>, String> {
    let s = match auth_ref {
        Some(x) => x.trim(),
        None => return Ok(None),
    };
    if s.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = s.splitn(4, ':').collect();
    let auth_type = parts.first().map(|x| x.trim()).unwrap_or("");
    if auth_type != "env" {
        return Err(format!(
            "external_auth_ref unsupported type: {:?}, only env is supported",
            auth_type
        ));
    }
    let var_name = parts.get(1).map(|x| x.trim()).filter(|x| !x.is_empty());
    let Some(var_name) = var_name else {
        return Err("external_auth_ref env: missing variable name".to_string());
    };
    let (header_name, use_bearer) = if parts.get(2) == Some(&"header") {
        let h = parts.get(3).map(|x| x.trim()).filter(|x| !x.is_empty());
        let Some(h) = h else {
            return Err("external_auth_ref env:var:header: missing header name".to_string());
        };
        (h.to_string(), false)
    } else {
        ("Authorization".to_string(), true)
    };
    let value = std::env::var(var_name).map_err(|_| {
        format!(
            "external_auth_ref env:{} not set or empty (set the environment variable)",
            var_name
        )
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!(
            "external_auth_ref env:{} is empty (set the environment variable)",
            var_name
        ));
    }
    let header_value = if use_bearer {
        format!("Bearer {}", value)
    } else {
        value.to_string()
    };
    Ok(Some((header_name, header_value)))
}

pub(crate) fn mask_endpoint_for_log(endpoint: &str) -> String {
    let s = endpoint.trim();
    if s.is_empty() {
        return "<empty>".to_string();
    }
    if let Some((scheme, rest)) = s.split_once("://") {
        if let Some(after) = rest.find('/') {
            return format!("{}://{}...", scheme, rest.split_at(after).0);
        }
        return format!("{}://...", scheme);
    }
    if s.len() > 32 {
        return format!("{}...", &s[..32.min(s.len())]);
    }
    s.to_string()
}

pub(crate) async fn execute_external_http_json(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    args: &Value,
    source: &str,
) -> Result<Value, String> {
    use claw_core::skill_registry::ExternalSkillConfig;

    let reg = state
        .get_skills_registry()
        .ok_or_else(|| "external skill requires registry".to_string())?;
    let config: ExternalSkillConfig<'_> =
        reg.external_config(canonical_skill_name).ok_or_else(|| {
            "external skill missing external_kind or external_endpoint in registry".to_string()
        })?;
    if config.kind != "http_json" {
        return Err(format!(
            "external_kind not supported: {}, only http_json is supported",
            config.kind
        ));
    }
    let timeout_secs = config
        .timeout_seconds
        .unwrap_or(state.skill_timeout_seconds)
        .max(1);
    let endpoint = config
        .endpoint
        .ok_or_else(|| "external http_json skill missing external_endpoint".to_string())?;
    let endpoint_masked = mask_endpoint_for_log(endpoint);

    let auth_header = match resolve_external_auth(config.auth_ref) {
        Ok(Some((name, value))) => {
            tracing::info!(
                "skill_dispatch external skill={} external_kind={} external_endpoint={} auth_ref_type=env auth_header={} auth_resolved=ok",
                canonical_skill_name, config.kind, endpoint_masked, name
            );
            Some((name, value))
        }
        Ok(None) => {
            tracing::info!(
                "skill_dispatch external skill={} external_kind={} external_endpoint={} auth_ref=none",
                canonical_skill_name, config.kind, endpoint_masked
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                "skill_dispatch external skill={} external_endpoint={} auth_ref_type=env auth_resolved=fail err={}",
                canonical_skill_name, endpoint_masked, e
            );
            return Err(e);
        }
    };

    let body = serde_json::json!({
        "skill": canonical_skill_name,
        "args": args,
        "task_id": task.task_id,
        "source": source,
    });

    let timeout = Duration::from_secs(timeout_secs);
    let mut req = state
        .http_client
        .post(endpoint)
        .json(&body)
        .timeout(timeout);
    if let Some((name, value)) = auth_header {
        req = req.header(name.as_str(), value);
    }
    let res = req.send().await.map_err(|e| {
        let msg = format!("external http_json request failed: {}", e);
        tracing::warn!(
            "skill_dispatch external request failed skill={} endpoint={} err={}",
            canonical_skill_name,
            endpoint_masked,
            e
        );
        msg
    })?;

    let status_code = res.status();
    let resp_body = res.text().await.map_err(|e| {
        let msg = format!("external http_json read body failed: {}", e);
        tracing::warn!(
            "skill_dispatch external read_body failed skill={} err={}",
            canonical_skill_name,
            e
        );
        msg
    })?;

    if !status_code.is_success() {
        tracing::warn!(
            "skill_dispatch external response non-2xx skill={} endpoint={} status={} body_len={}",
            canonical_skill_name,
            endpoint_masked,
            status_code,
            resp_body.len()
        );
        return Err(format!(
            "external endpoint returned {}: {}",
            status_code,
            resp_body.chars().take(200).collect::<String>()
        ));
    }

    let parsed: Value = serde_json::from_str(&resp_body).map_err(|e| {
        let msg = format!("external http_json response parse failed: {}", e);
        tracing::warn!(
            "skill_dispatch external response parse failed skill={} err={} raw_len={}",
            canonical_skill_name,
            e,
            resp_body.len()
        );
        msg
    })?;

    let ok = parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let status = if ok { "ok" } else { "error" };
    let error_str = parsed
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let text_str = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or_default();
    let messages: Vec<&str> = parsed
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let file_path = parsed
        .get("file")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let image_file_path = parsed
        .get("image_file")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let mut text = text_str.to_string();
    for m in messages {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(m);
    }
    if let Some(p) = file_path {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("FILE: ");
        text.push_str(p);
    }
    if let Some(p) = image_file_path {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("IMAGE_FILE: ");
        text.push_str(p);
    }

    tracing::info!(
        "skill_dispatch external response_parse_ok skill={} status={} text_len={}",
        canonical_skill_name,
        status,
        text.len()
    );

    let error_text = if ok {
        ""
    } else {
        error_str.unwrap_or("external returned ok=false")
    };
    let value = serde_json::json!({
        "status": status,
        "text": text,
        "error_text": error_text,
    });
    Ok(value)
}

pub(crate) async fn execute_builtin_skill(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> Result<String, String> {
    let policy_token = format!("skill:{skill_name}");
    if !state
        .tools_policy
        .is_allowed(&policy_token, state.active_provider_type.as_deref())
    {
        return Err(format!("blocked by policy: {policy_token}"));
    }

    let map = ensure_args_object(args)?;

    match skill_name {
        "read_file" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            let bytes = std::fs::read(&real_path).map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    format!("{}{}", READ_FILE_NOT_FOUND_PREFIX, real_path.display())
                } else {
                    format!("read file failed: {err}")
                }
            })?;
            let clip = if bytes.len() > crate::MAX_READ_FILE_BYTES {
                &bytes[..crate::MAX_READ_FILE_BYTES]
            } else {
                &bytes
            };
            Ok(String::from_utf8_lossy(clip).to_string())
        }
        "write_file" => {
            ensure_only_keys(map, &["path", "content"])?;
            let path = required_string(map, "path")?;
            let content = required_string(map, "content")?;
            if content.len() > crate::MAX_WRITE_FILE_BYTES {
                return Err(format!("content too large: {} bytes", content.len()));
            }
            let effective_path = crate::ensure_default_file_path(&state.workspace_root, path);
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                &effective_path,
                state.allow_path_outside_workspace,
            )?;
            if let Some(parent) = real_path.parent() {
                std::fs::create_dir_all(parent).map_err(|err| format!("mkdir failed: {err}"))?;
            }
            std::fs::write(&real_path, content)
                .map_err(|err| format!("write file failed: {err}"))?;
            Ok(format!(
                "written {} bytes to {}",
                content.len(),
                real_path.display()
            ))
        }
        "list_dir" => {
            ensure_only_keys(map, &["path"])?;
            let path = optional_string(map, "path").unwrap_or(".");
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            let mut items = Vec::new();
            for entry in
                std::fs::read_dir(&real_path).map_err(|err| format!("read_dir failed: {err}"))?
            {
                let e = entry.map_err(|err| format!("dir entry failed: {err}"))?;
                let name = e.file_name();
                let mut label = name.to_string_lossy().to_string();
                if e.path().is_dir() {
                    label.push('/');
                }
                items.push(label);
                if items.len() >= 200 {
                    break;
                }
            }
            items.sort();
            Ok(items.join("\n"))
        }
        "run_cmd" => {
            ensure_only_keys(
                map,
                &[
                    "command",
                    "cwd",
                    "request_text",
                    "suggested_params",
                    "suggest_once",
                    "llm_suggest_once",
                ],
            )?;
            let cwd = optional_string(map, "cwd").unwrap_or(".");
            let cwd_path = resolve_workspace_path(
                &state.workspace_root,
                cwd,
                state.allow_path_outside_workspace,
            )?;
            let request_text = optional_string(map, "request_text")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let _suggest_once = map
                .get("suggest_once")
                .and_then(|v| v.as_bool())
                .or_else(|| map.get("llm_suggest_once").and_then(|v| v.as_bool()))
                .unwrap_or(true);
            let mut command = optional_string(map, "command")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| suggested_command_from_args(map))
                .unwrap_or_default();
            if command.trim().is_empty() {
                if let Some(ref natural_request) = request_text {
                    command =
                        suggest_command_for_run_cmd(state, natural_request, &cwd_path, None, None)
                            .await?;
                } else {
                    return Err(
                        "command must be string (or provide request_text for NL2CMD)".to_string(),
                    );
                }
            }
            let sanitized_command =
                crate::bootstrap::sanitize_command_before_execute(&state.command_intent, &command);
            if sanitized_command.is_empty() {
                return Err("empty command after sanitize".to_string());
            }
            if sanitized_command != command.trim() {
                tracing::info!(
                    "run_cmd sanitized command: before={} after={}",
                    crate::truncate_for_log(&command),
                    crate::truncate_for_log(&sanitized_command)
                );
            }
            run_safe_command(
                &cwd_path,
                &sanitized_command,
                state.max_cmd_length,
                state.cmd_timeout_seconds,
                state.allow_sudo,
            )
            .await
        }
        "make_dir" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            std::fs::create_dir_all(&real_path)
                .map_err(|err| format!("create_dir failed: {err}"))?;
            Ok(format!("created directory {}", real_path.display()))
        }
        "remove_file" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            if real_path.is_dir() {
                return Err(
                    "remove_file only supports files; use run_cmd for directory removal"
                        .to_string(),
                );
            }
            std::fs::remove_file(&real_path).map_err(|err| format!("remove_file failed: {err}"))?;
            Ok(format!("removed {}", real_path.display()))
        }
        _ => Err(format!("unknown skill: {skill_name}")),
    }
}

pub(crate) fn inject_skill_memory_context(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: Value,
) -> Value {
    if !state.memory.skill_memory_enabled {
        return args;
    }
    let mut obj = match args {
        Value::Object(map) => map,
        other => return other,
    };
    if crate::canonical_skill_name(skill_name) == "chat" {
        return Value::Object(obj);
    }
    if obj.contains_key("_memory") {
        return Value::Object(obj);
    }
    let anchor = skill_memory_anchor(skill_name, &obj);
    let structured = crate::memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        &anchor,
        state.memory.recall_limit.max(1),
        true,
        true,
    );
    let memory_context = crate::memory::service::structured_memory_context_block(
        &structured,
        crate::memory::retrieval::MemoryContextMode::Skill,
        state.memory.skill_memory_max_chars.max(384),
    );
    let mut pref_map = serde_json::Map::new();
    for (k, v) in &structured.preferences {
        pref_map.insert(k.clone(), Value::String(v.clone()));
    }
    let lang_hint = crate::memory::service::preferred_response_language(&structured.preferences)
        .unwrap_or_default();
    obj.insert(
        "_memory".to_string(),
        serde_json::json!({
            "context": memory_context,
            "long_term_summary": structured.long_term_summary.clone().unwrap_or_default(),
            "preferences": Value::Object(pref_map),
            "lang_hint": lang_hint
        }),
    );
    Value::Object(obj)
}

fn skill_memory_anchor(skill_name: &str, args_obj: &serde_json::Map<String, Value>) -> String {
    let mut parts = vec![format!("skill={skill_name}")];
    for key in [
        "text",
        "query",
        "instruction",
        "goal",
        "prompt",
        "message",
        "content",
        "action",
    ] {
        if let Some(val) = args_obj.get(key).and_then(|v| v.as_str()) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }
    parts.join(" | ")
}

pub(crate) fn ensure_default_output_dir_for_skill_args(
    workspace_root: &Path,
    skill_name: &str,
    args: Value,
) -> Value {
    let Some(mut obj) = args.as_object().cloned() else {
        return args;
    };
    match skill_name {
        "image_generate" | "image_edit" => {
            let section = if skill_name == "image_edit" {
                "image_edit"
            } else {
                "image_generation"
            };
            let dir = resolve_output_dir_from_config(workspace_root, section);
            let ts = crate::now_ts_u64();
            let prefix = if skill_name == "image_edit" {
                "edit"
            } else {
                "gen"
            };
            let suggested = format!("{dir}/{prefix}-{ts}.png");
            obj.insert("output_path".to_string(), Value::String(suggested));
            Value::Object(obj)
        }
        _ => Value::Object(obj),
    }
}

fn resolve_output_dir_from_config(workspace_root: &Path, section: &str) -> String {
    let cfg_path = workspace_root.join("configs/config.toml");
    let raw = match std::fs::read_to_string(cfg_path) {
        Ok(v) => v,
        Err(_) => return "document".to_string(),
    };
    let value: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return "document".to_string(),
    };
    value
        .get(section)
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("default_output_dir"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("document")
        .to_string()
}

pub(crate) async fn execute_builtin_skill_for_task(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: &Value,
) -> Result<String, String> {
    if skill_name != "schedule" {
        return execute_builtin_skill(state, skill_name, args).await;
    }
    let map = ensure_args_object(args)?;
    ensure_only_keys(map, &["action", "text"])?;
    let action = required_string(map, "action")?.trim().to_ascii_lowercase();
    if action != "compile" {
        return Err("schedule.action must be compile".to_string());
    }
    let text = required_string(map, "text")?;
    let intent = crate::schedule_service::parse_schedule_intent(state, task, text)
        .await
        .ok_or_else(|| "schedule intent not detected".to_string())?;
    serde_json::to_string(&intent).map_err(|e| format!("serialize schedule intent failed: {e}"))
}

fn ensure_args_object(args: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    args.as_object()
        .ok_or_else(|| "skill args must be a JSON object".to_string())
}

fn ensure_only_keys(map: &serde_json::Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    for k in map.keys() {
        if !allowed.iter().any(|x| x == k) {
            return Err(format!("unexpected arg key: {k}"));
        }
    }
    Ok(())
}

fn required_string<'a>(
    map: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    map.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{key} must be string"))
}

fn optional_string<'a>(map: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(|v| v.as_str())
}

fn suggested_command_from_args(map: &serde_json::Map<String, Value>) -> Option<String> {
    map.get("suggested_params")
        .and_then(|v| v.as_object())
        .and_then(|obj| {
            obj.get("command")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

pub(crate) fn resolve_workspace_path(
    workspace_root: &Path,
    input: &str,
    allow_path_outside_workspace: bool,
) -> Result<PathBuf, String> {
    let base = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        workspace_root.join(input)
    };

    if allow_path_outside_workspace {
        return Ok(base);
    }

    if base.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("path with '..' is not allowed".to_string());
    }

    if !base.starts_with(workspace_root) {
        return Err("path is outside workspace".to_string());
    }

    Ok(base)
}

pub(crate) async fn run_safe_command(
    cwd: &Path,
    command: &str,
    max_cmd_length: usize,
    cmd_timeout_seconds: u64,
    allow_sudo: bool,
) -> Result<String, String> {
    if command.len() > max_cmd_length {
        return Err("command too long".to_string());
    }

    if command.trim().is_empty() {
        return Err("empty command".to_string());
    }

    if !allow_sudo && command.split_whitespace().any(|p| p == "sudo") {
        return Err("sudo is not allowed by tools config".to_string());
    }

    let mut cmd = Command::new("bash");
    cmd.arg("-lc").arg(command);
    cmd.current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let soft_timeout = cmd_timeout_seconds.max(1);
    let child = cmd
        .spawn()
        .map_err(|err| format!("run command failed: {err}"))?;
    let child_pid = child.id();
    let mut output_fut = Box::pin(child.wait_with_output());

    let out = match tokio::time::timeout(Duration::from_secs(soft_timeout), &mut output_fut).await {
        Ok(r) => r.map_err(|err| format!("run command failed: {err}"))?,
        Err(_) => {
            tracing::info!(
                "run_cmd soft-timeout reached; killing command (soft={}s): {}",
                soft_timeout,
                crate::truncate_for_log(command)
            );
            if let Some(pid) = child_pid {
                let _ = Command::new("kill")
                    .arg("-9")
                    .arg(pid.to_string())
                    .status()
                    .await;
            }
            let _ = output_fut.await;
            return Err(format!("Command timed out after {} seconds", soft_timeout));
        }
    };

    let stdout_text = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr_text = String::from_utf8_lossy(&out.stderr).to_string();

    let mut text = String::new();
    text.push_str(&stdout_text);
    if !stderr_text.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&stderr_text);
    }

    if text.len() > 8000 {
        text.truncate(8000);
    }

    let exit_code = out.status.code().unwrap_or(-1);
    if exit_code == 0 {
        if text.trim().is_empty() {
            Ok(format!("exit=0 command={}", command.trim()))
        } else {
            Ok(text)
        }
    } else if text.trim().is_empty() {
        Err(format!("Command failed with exit code {}", exit_code))
    } else {
        let mut detail = String::new();
        if !stderr_text.trim().is_empty() {
            detail.push_str("stderr:\n");
            detail.push_str(stderr_text.trim());
        }
        if !stdout_text.trim().is_empty() {
            if !detail.is_empty() {
                detail.push_str("\n\n");
            }
            detail.push_str("stdout:\n");
            detail.push_str(stdout_text.trim());
        }
        if detail.len() > 8000 {
            detail.truncate(8000);
        }
        Err(format!(
            "Command failed with exit code {}\n{}",
            exit_code, detail
        ))
    }
}

#[derive(Debug, Deserialize)]
struct RunCmdSuggestionPayload {
    command: String,
    confidence: Option<f64>,
    reason: Option<String>,
}

fn build_run_cmd_nl_prompt(
    request_text: &str,
    cwd: &std::path::Path,
    previous_command: Option<&str>,
    previous_error: Option<&str>,
) -> String {
    let mut prompt = String::new();
    prompt
        .push_str("You map a natural-language request to ONE executable bash command for Linux.\n");
    prompt.push_str("Return strict JSON only: {\"command\":\"...\",\"confidence\":0.0-1.0,\"reason\":\"...\"}\n");
    prompt.push_str("Rules:\n");
    prompt.push_str("- Prefer read-only and low-risk commands.\n");
    prompt.push_str("- Do not use sudo by default.\n");
    prompt.push_str("- Avoid destructive commands (rm -rf, mkfs, reboot, shutdown, kill -9).\n");
    prompt.push_str(
        "- If one command may be missing, use shell fallback in ONE line (e.g. cmd1 || cmd2).\n",
    );
    prompt.push_str("- Output only a single-line command.\n\n");
    prompt.push_str(&format!("cwd: {}\n", cwd.display()));
    prompt.push_str(&format!("request_text: {}\n", request_text.trim()));
    if let Some(prev) = previous_command {
        prompt.push_str(&format!("previous_command: {}\n", prev.trim()));
    }
    if let Some(err) = previous_error {
        prompt.push_str(&format!(
            "previous_error: {}\n",
            crate::truncate_for_log(err)
        ));
    }
    prompt
}

pub(crate) async fn suggest_command_for_run_cmd(
    state: &AppState,
    request_text: &str,
    cwd: &std::path::Path,
    previous_command: Option<&str>,
    previous_error: Option<&str>,
) -> Result<String, String> {
    let provider = state
        .llm_providers
        .first()
        .cloned()
        .ok_or_else(|| "run_cmd NL2CMD unavailable: no llm provider configured".to_string())?;
    let prompt = build_run_cmd_nl_prompt(request_text, cwd, previous_command, previous_error);
    let resp = crate::call_provider_with_retry(provider, &prompt)
        .await
        .map_err(|e| format!("run_cmd NL2CMD provider failed: {e}"))?;
    let parsed = crate::parse_llm_json_extract_or_any::<RunCmdSuggestionPayload>(&resp.text)
        .ok_or_else(|| {
            format!(
                "run_cmd NL2CMD invalid json: {}",
                crate::truncate_for_log(&resp.text)
            )
        })?;
    let mut command = parsed.command.trim().to_string();
    if command.contains('\n') {
        command = command
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    if command.is_empty() {
        return Err("run_cmd NL2CMD returned empty command".to_string());
    }
    if let Some(conf) = parsed.confidence {
        tracing::info!("run_cmd NL2CMD confidence={:.2}", conf);
    }
    if let Some(reason) = parsed.reason {
        tracing::info!("run_cmd NL2CMD reason={}", crate::truncate_for_log(&reason));
    }
    Ok(command)
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
    let mut err_line = String::new();

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

    if let Some(stderr) = child.stderr.take() {
        let mut err_reader = BufReader::new(stderr);
        let _ = err_reader.read_line(&mut err_line).await;
    }

    let _ = child.wait().await;

    if out_line.trim().is_empty() {
        return Err(format!("empty skill-runner output: {}", err_line.trim()));
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
