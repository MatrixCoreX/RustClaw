use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;

use crate::{AppState, ClaimedTask};

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
        "prompt_bundle" => Ok(external_kind_machine_error_response(
            task,
            canonical_skill_name,
            config.kind,
            "external_kind_not_enabled",
        )),
        other => Ok(external_kind_machine_error_response(
            task,
            canonical_skill_name,
            other,
            "external_kind_unsupported",
        )),
    }
}

fn external_kind_machine_error_response(
    task: &ClaimedTask,
    canonical_skill_name: &str,
    external_kind: &str,
    error_code: &str,
) -> Value {
    serde_json::json!({
        "request_id": task.task_id,
        "status": "error",
        "text": "",
        "error_kind": error_code,
        "error_text": error_code,
        "extra": {
            "schema_version": 1,
            "owner_layer": "external_skill_adapter",
            "status_code": error_code,
            "error_code": error_code,
            "message_key": format!("clawd.msg.external_skill.{error_code}"),
            "skill_name": canonical_skill_name,
            "external_kind": external_kind,
            "provider_supported": false,
            "unsupported_reason": error_code,
        }
    })
}

fn external_reserved_arg_key(key: &str) -> bool {
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

fn value_to_cli_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn build_external_cli_args(args: &Value) -> Vec<String> {
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

fn resolve_external_bundle_dir(state: &AppState, bundle_rel: &str) -> Result<PathBuf, String> {
    if bundle_rel.trim().is_empty() {
        return Err("external skill missing external_bundle_dir".to_string());
    }
    let joined = state.skill_rt.workspace_root.join(bundle_rel);
    let canonical = joined
        .canonicalize()
        .map_err(|err| format!("external bundle directory not found: {err}"))?;
    if !canonical.starts_with(&state.skill_rt.workspace_root) {
        return Err("external bundle directory must stay inside workspace_root".to_string());
    }
    Ok(canonical)
}

fn resolve_external_entry_path(bundle_dir: &Path, entry_rel: &str) -> Result<PathBuf, String> {
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

fn is_bin_available(bin: &str) -> bool {
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

async fn verify_external_python_modules(
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

async fn execute_external_local_script(
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
        .unwrap_or(state.skill_rt.skill_timeout_seconds)
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
        .env(
            "WORKSPACE_ROOT",
            state.skill_rt.workspace_root.display().to_string(),
        )
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

fn extract_external_shell_command(args: &Value) -> Result<String, String> {
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

async fn execute_external_local_shell_recipe(
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
        .unwrap_or(state.skill_rt.cmd_timeout_seconds)
        .max(1);

    tracing::info!(
        "skill_dispatch external skill={} external_kind=local_shell_recipe command={} source={}",
        canonical_skill_name,
        crate::truncate_for_log(&command),
        source
    );

    match super::run_safe_command(
        &bundle_dir,
        &command,
        state.skill_rt.max_cmd_length,
        timeout_secs,
        state.skill_rt.cmd_idle_timeout_seconds,
        state.skill_rt.cmd_max_output_bytes,
        crate::skills::task_allows_sudo(state, Some(task)),
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

fn resolve_external_auth(auth_ref: Option<&str>) -> Result<Option<(String, String)>, String> {
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

fn mask_endpoint_for_log(endpoint: &str) -> String {
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

async fn execute_external_http_json(
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
        .unwrap_or(state.skill_rt.skill_timeout_seconds)
        .max(1);
    let endpoint = config
        .endpoint
        .ok_or_else(|| "external http_json skill missing external_endpoint".to_string())?;
    let endpoint_masked = mask_endpoint_for_log(endpoint);

    let auth_header = match resolve_external_auth(config.auth_ref) {
        Ok(Some((name, value))) => {
            tracing::info!(
                "skill_dispatch external skill={} external_kind={} external_endpoint={} auth_ref_type=env auth_header={} auth_resolved=ok",
                canonical_skill_name,
                config.kind,
                endpoint_masked,
                name
            );
            Some((name, value))
        }
        Ok(None) => {
            tracing::info!(
                "skill_dispatch external skill={} external_kind={} external_endpoint={} auth_ref=none",
                canonical_skill_name,
                config.kind,
                endpoint_masked
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                "skill_dispatch external skill={} external_endpoint={} auth_ref_type=env auth_resolved=fail err={}",
                canonical_skill_name,
                endpoint_masked,
                e
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
        .core
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
