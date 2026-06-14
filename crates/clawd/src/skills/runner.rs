use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::{AppState, ClaimedTask};

use super::{
    apply_skill_runner_env_isolation, current_task_auth_role, run_skill_with_runner_outcome,
    task_allows_path_outside_workspace, task_allows_sudo,
};

pub(super) fn extract_skill_provider_model(value: &Value) -> Option<(String, String, String)> {
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

fn local_clawd_base_url_from_workspace(workspace_root: &Path) -> String {
    let config_path = workspace_root.join("configs/config.toml");
    let parsed = std::fs::read_to_string(config_path)
        .ok()
        .and_then(|raw| raw.parse::<toml::Value>().ok());
    let server = parsed
        .as_ref()
        .and_then(|value| value.get("server"))
        .and_then(|value| value.as_table());
    if let Some(base_url) = server
        .and_then(|table| table.get("clawd_base_url"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return base_url.trim_end_matches('/').to_string();
    }
    let listen = server
        .and_then(|table| table.get("listen"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("127.0.0.1:8787");
    let loopback_listen = if listen.starts_with("0.0.0.0:") {
        listen.replacen("0.0.0.0", "127.0.0.1", 1)
    } else {
        listen.to_string()
    };
    format!("http://{}", loopback_listen.trim_end_matches('/'))
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

    // Manifest capabilities decide which secrets and LLM handles enter the child
    // process. Missing declared secrets fail before spawn instead of becoming
    // empty runtime environment variables.
    let caps: Vec<claw_core::skill_registry::Capability> = state
        .get_skills_registry()
        .as_ref()
        .map(|reg| reg.capabilities(canonical_skill_name).to_vec())
        .unwrap_or_default();
    let skill_uses_llm = caps
        .iter()
        .any(|cap| matches!(cap, claw_core::skill_registry::Capability::Llm));
    let secret_envs = {
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
                    "skill_dispatch skill={} missing_secrets={:?} broker={} refuse_to_spawn=true",
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

    let secret_token_ttl = Duration::from_secs(300);
    let selected_openai_model = if skill_uses_llm {
        Some(crate::llm_gateway::selected_openai_model(state, Some(task)))
    } else {
        None
    };
    let internal_llm_token = if skill_uses_llm {
        let internal_llm_context = json!({
            "task_id": task.task_id.clone(),
            "user_id": task.user_id,
            "chat_id": task.chat_id,
            "user_key": task.user_key.clone(),
            "channel": task.channel.clone(),
            "external_user_id": task.external_user_id.clone(),
            "external_chat_id": task.external_chat_id.clone(),
            "kind": task.kind.clone(),
            "payload_json": task.payload_json.clone(),
            "skill_name": canonical_skill_name,
        });
        match claw_core::secrets::issue_secret_token_value(
            &claw_core::secrets::SecretValue::new(internal_llm_context.to_string()),
            secret_token_ttl,
        ) {
            Ok(token) => Some(token),
            Err(err) => {
                return Err(format!(
                    "skill `{canonical_skill_name}` failed to issue internal LLM token: {err}"
                ));
            }
        }
    } else {
        None
    };
    let tokenized_secret_envs =
        match claw_core::secrets::issue_secret_env_tokens(&secret_envs, secret_token_ttl) {
            Ok(pairs) => pairs,
            Err(err) => {
                return Err(format!(
                "skill `{canonical_skill_name}` failed to issue short-lived secret tokens: {err}"
            ));
            }
        };
    let openai_api_key_token = if skill_uses_llm {
        let selected_openai_api_key =
            crate::llm_gateway::selected_openai_api_key(state, Some(task));
        if selected_openai_api_key.trim().is_empty() {
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
        }
    } else {
        None
    };
    let mut cmd = Command::new(&state.skill_rt.skill_runner_path);
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
    if let Some(token) = &internal_llm_token {
        cmd.env(
            "RUSTCLAW_INTERNAL_LLM_URL",
            format!(
                "{}/v1/internal/llm/text",
                local_clawd_base_url_from_workspace(&state.skill_rt.workspace_root)
            ),
        )
        .env("RUSTCLAW_INTERNAL_LLM_TOKEN", token)
        .env(
            "OPENAI_BASE_URL",
            crate::llm_gateway::selected_openai_base_url(state, Some(task)),
        );
    }
    if let Some(model) = &selected_openai_model {
        cmd.env("OPENAI_MODEL", model);
    }
    if let Some(token) = &openai_api_key_token {
        cmd.env("OPENAI_API_KEY", token);
    }
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
    let locale_tag = super::task_request_locale_tag(state, task);
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
