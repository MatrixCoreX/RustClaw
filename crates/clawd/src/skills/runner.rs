use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::codec::LinesCodecError;

use claw_core::config::ToolSandboxMode;
use claw_core::skill_registry::{
    Capability, CapabilityIsolationProfile, PlannerCapabilityEffect, PlannerCapabilityMapping,
};

use crate::{AppState, ClaimedTask};

use super::credential_fallback::provision_skill_secret_envs;
use super::runner_pool::{WarmPoolCheckout, WarmRunnerKey, WarmRunnerProcess};
use super::{
    action_scoped_planner_mapping, apply_skill_runner_env_isolation, current_task_auth_role,
    place_subprocess_in_own_process_group, run_skill_with_runner_outcome,
    task_allows_path_outside_workspace, task_allows_sudo, terminate_subprocess_group,
};

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;

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

fn selected_provider_api_key_env_names(vendor: &str, provider_type: &str) -> Vec<&'static str> {
    let mut names = match vendor.trim().to_ascii_lowercase().as_str() {
        "openai" => vec!["OPENAI_API_KEY"],
        "google" | "gemini" => vec!["GOOGLE_API_KEY"],
        "anthropic" | "claude" => vec!["ANTHROPIC_API_KEY"],
        "grok" | "xai" => vec!["GROK_API_KEY"],
        "deepseek" => vec!["DEEPSEEK_API_KEY"],
        "qwen" => vec!["QWEN_API_KEY"],
        "minimax" => vec!["MINIMAX_API_KEY"],
        "mimo" | "xiaomi" => vec!["MIMO_API_KEY"],
        "custom" => Vec::new(),
        _ => Vec::new(),
    };
    if provider_type.trim().eq_ignore_ascii_case("openai_compat")
        && !names.contains(&"OPENAI_API_KEY")
    {
        names.push("OPENAI_API_KEY");
    }
    names
}

fn local_clawd_base_url_from_internal_listen(internal_listen: Option<&str>) -> String {
    let address = internal_listen
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<SocketAddr>().ok())
        .filter(|value| value.ip().is_loopback());
    address
        .map(|value| format!("http://{value}"))
        .unwrap_or_else(|| claw_core::config::CLAWD_INTERNAL_BASE_URL.to_string())
}

fn inherited_sandbox_backend(backend: &'static str) -> Option<&'static str> {
    (backend != "direct").then_some(backend)
}

fn runner_additional_writable_paths(
    secret_store_directory: Option<&std::path::Path>,
    skill_storage_directory: Option<&std::path::Path>,
    artifact_output_directory: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    secret_store_directory
        .into_iter()
        .chain(skill_storage_directory)
        .chain(artifact_output_directory)
        .map(std::path::Path::to_path_buf)
        .collect()
}

fn invocation_artifact_output_directory(
    workspace_root: &std::path::Path,
    task_id: &str,
    skill_name: &str,
) -> std::path::PathBuf {
    fn component(value: &str, fallback: &str) -> String {
        let normalized: String = value
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                    ch
                } else {
                    '_'
                }
            })
            .take(96)
            .collect();
        if normalized.is_empty() {
            fallback.to_string()
        } else {
            normalized
        }
    }

    claw_core::workspace_state::workspace_artifacts_root(workspace_root)
        .join("skill-invocations")
        .join(component(task_id, "task"))
        .join(component(skill_name, "skill"))
        .join(uuid::Uuid::new_v4().to_string())
}

fn sandbox_target_for_source(
    source: Option<&std::path::Path>,
    sources: &[std::path::PathBuf],
    targets: &[std::path::PathBuf],
) -> Option<std::path::PathBuf> {
    let source = source?;
    sources
        .iter()
        .position(|candidate| candidate == source)
        .and_then(|index| targets.get(index))
        .cloned()
}

fn map_storage_descriptor_to_sandbox(
    mut descriptor: Option<crate::skill_storage::SkillStorageDescriptor>,
    sandbox_directory: Option<&std::path::Path>,
) -> Result<Option<crate::skill_storage::SkillStorageDescriptor>, String> {
    let Some(storage_descriptor) = descriptor.as_mut() else {
        return Ok(None);
    };
    let Some(sandbox_directory) = sandbox_directory else {
        return Err("skill storage sandbox target unavailable".to_string());
    };
    let file_name = std::path::Path::new(&storage_descriptor.database_path)
        .file_name()
        .ok_or_else(|| "skill storage database path has no file name".to_string())?;
    storage_descriptor.database_path = sandbox_directory.join(file_name).display().to_string();
    Ok(descriptor)
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
    args: &serde_json::Value,
    source: &str,
    skill_timeout_secs: u64,
    execution_context: Option<&super::SkillExecutionContext>,
) -> Result<serde_json::Value, String> {
    let dispatch_started = std::time::Instant::now();
    let package_root = state.skill_rt.workspace_root.join("data/skill-packages");
    let skill_views = state.get_skill_views_snapshot();
    let admission_binding = skill_views
        .binding
        .admission_bindings
        .get(canonical_skill_name);
    let resolver = skill_sdk::SkillRuntimeResolver::new(&package_root);
    let installed_launch = admission_binding
        .map_or_else(
            || resolver.pin_current(canonical_skill_name),
            |binding| {
                resolver.pin_exact(
                    canonical_skill_name,
                    &binding.version,
                    &binding.manifest_digest,
                    &binding.install_receipt_digest,
                )
            },
        )
        .map_err(|error| {
            format!(
                "verified skill package unavailable: skill={canonical_skill_name} code={} detail={}",
                error.code, error.detail
            )
        })?;
    let version_lease = skill_sdk::InstallReceiptStore::new(&package_root)
        .acquire_version_lease(canonical_skill_name, &installed_launch.install_root)
        .map_err(|error| {
            format!(
                "skill version lease unavailable: skill={canonical_skill_name} code={} detail={}",
                error.code, error.detail
            )
        })?;
    tracing::debug!(
        skill = canonical_skill_name,
        version = installed_launch.version,
        install_dir = version_lease.install_dir(),
        "skill_version_lease_acquired"
    );
    let user_key_for_skill = task
        .user_key
        .clone()
        .map(Value::String)
        .unwrap_or(Value::Null);
    let storage_descriptor = storage_descriptor_for_skill(state, canonical_skill_name)?;
    let storage_writable_directory = storage_descriptor
        .as_ref()
        .and_then(|descriptor| std::path::Path::new(&descriptor.database_path).parent())
        .map(std::path::Path::to_path_buf);
    let artifact_output_directory = invocation_artifact_output_directory(
        &state.skill_rt.workspace_root,
        &task.task_id,
        canonical_skill_name,
    );
    std::fs::create_dir_all(&artifact_output_directory).map_err(|error| {
        format!("skill artifact directory unavailable: skill={canonical_skill_name} error={error}")
    })?;
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
    let action_mapping = action_scoped_planner_mapping(state, canonical_skill_name, args);
    let execution_policy = crate::task_execution_policy::effective_policy_for_task(state, task);
    let caps: Vec<Capability> = state
        .get_skills_registry()
        .as_ref()
        .map(|reg| reg.capabilities(canonical_skill_name).to_vec())
        .unwrap_or_default();
    let caps = action_scoped_runner_capabilities(caps, action_mapping.as_ref());
    let skill_uses_llm = caps
        .iter()
        .any(|cap| matches!(cap, claw_core::skill_registry::Capability::Llm));
    let selected_llm_connection = skill_uses_llm
        .then(|| crate::llm_gateway::selected_llm_connection(state, Some(task)))
        .flatten();
    let secret_envs = {
        let broker = claw_core::secrets::global_or_default();
        match provision_skill_secret_envs(broker.as_ref(), &caps, selected_llm_connection.as_ref())
        {
            Ok(provisioned) => {
                let pairs = provisioned.envs;
                if !pairs.is_empty() {
                    let names: Vec<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
                    tracing::info!(
                        "skill_dispatch skill={} provisioned_secrets={:?} broker={}",
                        canonical_skill_name,
                        names,
                        broker.label()
                    );
                }
                if !provisioned.fallback_credentials.is_empty() {
                    tracing::info!(
                        skill = canonical_skill_name,
                        selected_vendor = selected_llm_connection
                            .as_ref()
                            .map(|connection| connection.vendor.as_str())
                            .unwrap_or("unknown"),
                        fallback_credentials = ?provisioned.fallback_credentials,
                        "skill_dispatch_multimodal_credential_fallback"
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
                return Err(super::structured_skill_error_from_parts(
                    canonical_skill_name,
                    "skill_credentials_missing",
                    "required skill credentials are not configured",
                    Some(std::env::consts::OS),
                    Some(json!({
                        "message_key": "clawd.skill.credentials_missing",
                        "retryable": false,
                        "failure_phase": "pre_dispatch",
                        "side_effect_applied": false,
                        "recovery_action": "configure_credentials",
                        "credential_envs": env_names,
                        "broker": broker.label(),
                    })),
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
                return Err(super::structured_skill_error_from_parts(
                    canonical_skill_name,
                    "skill_credential_lookup_failed",
                    "skill credential lookup failed",
                    Some(std::env::consts::OS),
                    Some(json!({
                        "message_key": "clawd.skill.credential_lookup_failed",
                        "retryable": false,
                        "failure_phase": "pre_dispatch",
                        "side_effect_applied": false,
                        "recovery_action": "repair_credential_broker",
                        "credential_ref": name,
                        "broker": broker.label(),
                        "detail": source.to_string(),
                    })),
                ));
            }
        }
    };

    let secret_token_ttl = Duration::from_secs(300);
    let internal_skill_context = json!({
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
    let internal_llm_token = if skill_uses_llm {
        match claw_core::secrets::issue_secret_token_value(
            &claw_core::secrets::SecretValue::new(internal_skill_context.to_string()),
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
    let admission_capable = state.get_skills_registry().is_some_and(|registry| {
        registry
            .planner_capabilities(canonical_skill_name)
            .iter()
            .any(|capability| capability.name == "extension.register_skill")
    });
    let internal_admission_token = if admission_capable {
        Some(
            claw_core::secrets::issue_secret_token_value(
                &claw_core::secrets::SecretValue::new(internal_skill_context.to_string()),
                secret_token_ttl,
            )
            .map_err(|error| {
                format!(
                    "skill `{canonical_skill_name}` failed to issue internal admission token: {error}"
                )
            })?,
        )
    } else {
        None
    };
    let selected_provider_api_key = selected_llm_connection
        .as_ref()
        .map(|connection| connection.api_key.trim())
        .filter(|value| !value.is_empty());
    let selected_provider_env_names = selected_llm_connection
        .as_ref()
        .map(|connection| {
            selected_provider_api_key_env_names(&connection.vendor, &connection.provider_type)
        })
        .unwrap_or_default();
    let secret_token_scope = if secret_envs.is_empty()
        && (selected_provider_api_key.is_none() || selected_provider_env_names.is_empty())
    {
        None
    } else {
        Some(
            claw_core::secrets::SecretTokenScope::create().map_err(|err| {
                tracing::error!(
                    skill = canonical_skill_name,
                    error = %err,
                    "secret_token_scope_create_failed"
                );
                format!("secret_token_scope_create_failed; skill={canonical_skill_name}")
            })?,
        )
    };
    let tokenized_secret_envs = match secret_token_scope.as_ref() {
        Some(scope) => match scope.issue_env_tokens(&secret_envs, secret_token_ttl) {
            Ok(pairs) => pairs,
            Err(err) => {
                tracing::error!(
                    skill = canonical_skill_name,
                    secret_kind = "declared_env",
                    error = %err,
                    "secret_token_issue_failed"
                );
                return Err(format!(
                    "secret_token_issue_failed; skill={canonical_skill_name}; secret_kind=declared_env"
                ));
            }
        },
        None => Vec::new(),
    };
    let mut selected_provider_key_tokens = Vec::new();
    if let (Some(scope), Some(api_key)) = (secret_token_scope.as_ref(), selected_provider_api_key) {
        for env_name in &selected_provider_env_names {
            match scope.issue_value(
                &claw_core::secrets::SecretValue::new(api_key),
                secret_token_ttl,
            ) {
                Ok(token) => selected_provider_key_tokens.push((*env_name, token)),
                Err(err) => {
                    tracing::error!(
                        skill = canonical_skill_name,
                        credential_env = *env_name,
                        error = %err,
                        "secret_token_issue_failed"
                    );
                    return Err(format!(
                        "secret_token_issue_failed; skill={canonical_skill_name}; secret_kind=selected_provider"
                    ));
                }
            }
        }
    }
    let network = if caps.iter().any(|cap| {
        matches!(
            cap,
            claw_core::skill_registry::Capability::Net | claw_core::skill_registry::Capability::Llm
        )
    }) {
        crate::process_sandbox::ProcessNetworkPolicy::Inherit
    } else {
        crate::process_sandbox::ProcessNetworkPolicy::Deny
    };
    let sandbox_mode =
        action_scoped_runner_sandbox_mode(execution_policy.sandbox_mode, action_mapping.as_ref());
    let unrestricted_admin = execution_policy.has_unrestricted_admin_authority();
    let allow_path_outside_workspace = task_allows_path_outside_workspace(state, Some(task));
    let allow_sudo = task_allows_sudo(state, Some(task));
    let has_secrets = secret_token_scope.is_some()
        || internal_llm_token.is_some()
        || internal_admission_token.is_some()
        || !selected_provider_key_tokens.is_empty()
        || !tokenized_secret_envs.is_empty();
    let warm_reuse_allowed = stateless_readonly_reuse_allowed(
        installed_launch.execution_profile,
        &caps,
        action_mapping.as_ref(),
        sandbox_mode,
        storage_descriptor.is_some(),
        has_secrets,
        admission_capable,
        unrestricted_admin,
        allow_path_outside_workspace,
        allow_sudo,
    );
    let additional_writable_paths = runner_additional_writable_paths(
        secret_token_scope.as_ref().map(|scope| scope.store_dir()),
        storage_writable_directory.as_deref(),
        (!warm_reuse_allowed).then_some(artifact_output_directory.as_path()),
    );
    let prepared = if warm_reuse_allowed {
        // The outer runner is trusted host code. Keeping it outside a sandbox
        // lets it create a fresh receipt-controlled sandbox (and fresh /tmp on
        // Linux) for every skill child instead of sharing one warm namespace.
        crate::process_sandbox::PreparedProcessCommand {
            command: tokio::process::Command::new(&state.skill_rt.skill_runner_path),
            backend: "child_sandbox",
            additional_writable_targets: Vec::new(),
        }
    } else {
        crate::process_sandbox::prepare_process_command(
            &state.skill_rt.skill_runner_path,
            crate::process_sandbox::ProcessSandboxRequest {
                mode: sandbox_mode,
                backend: state.skill_rt.tools_policy.sandbox_backend,
                workspace_root: &state.skill_rt.workspace_root,
                execution_root: &state.skill_rt.workspace_root,
                network,
                additional_writable_paths: &additional_writable_paths,
            },
        )
        .map_err(|reason_code| {
            format!(
                "skill-runner sandbox unavailable: reason_code={reason_code} sandbox_mode={} sandbox_backend={}",
                sandbox_mode.as_token(),
                state.skill_rt.tools_policy.sandbox_backend_token()
            )
        })?
    };
    tracing::debug!(
        skill = canonical_skill_name,
        sandbox_backend = prepared.backend,
        sandbox_backend_requested = state.skill_rt.tools_policy.sandbox_backend_token(),
        sandbox_mode = sandbox_mode.as_token(),
        network_policy = ?network,
        "skill_runner_process_sandbox_prepared"
    );
    let sandbox_token_store_dir = sandbox_target_for_source(
        secret_token_scope.as_ref().map(|scope| scope.store_dir()),
        &additional_writable_paths,
        &prepared.additional_writable_targets,
    );
    let sandbox_storage_directory = sandbox_target_for_source(
        storage_writable_directory.as_deref(),
        &additional_writable_paths,
        &prepared.additional_writable_targets,
    );
    let sandbox_artifact_output_directory = if warm_reuse_allowed {
        artifact_output_directory.clone()
    } else {
        sandbox_target_for_source(
            Some(&artifact_output_directory),
            &additional_writable_paths,
            &prepared.additional_writable_targets,
        )
        .ok_or_else(|| "skill artifact sandbox target unavailable".to_string())?
    };
    let storage_descriptor = map_storage_descriptor_to_sandbox(
        storage_descriptor,
        sandbox_storage_directory.as_deref(),
    )?;
    let skill_context = build_runner_skill_context(
        state,
        task,
        source,
        storage_descriptor,
        &sandbox_artifact_output_directory,
        execution_context,
    );
    let req_line = serde_json::json!({
        "request_id": task.task_id,
        "user_id": task.user_id,
        "chat_id": task.chat_id,
        "user_key": user_key_for_skill,
        "external_user_id": task.external_user_id,
        "external_chat_id": crate::task_external_chat_id(task),
        "skill_name": canonical_skill_name,
        "expected_skill_version": installed_launch.version.clone(),
        "expected_manifest_digest": installed_launch.manifest_digest.clone(),
        "expected_receipt_digest": installed_launch.receipt_digest.clone(),
        "expected_registry_generation": skill_views.binding.registry_generation,
        "expected_registry_generation_digest": skill_views.binding.registry_generation_digest.clone(),
        "expected_base_registry_digest": skill_views.binding.base_registry_digest.clone(),
        "expected_overlay_generation_digest": skill_views.binding.overlay_generation_digest.clone(),
        "expected_policy_digest": admission_binding.and_then(|binding| binding.policy_digest.clone()),
        "expected_admission_receipt_digest": admission_binding
            .map(|binding| binding.admission_receipt_digest.clone()),
        "args": args,
        "context": skill_context
    })
    .to_string();
    let sandbox_backend = prepared.backend;
    let inherited_sandbox_backend = (!warm_reuse_allowed)
        .then(|| inherited_sandbox_backend(sandbox_backend))
        .flatten();
    let internal_listen = claw_core::product_identity::env_string("INTERNAL_LISTEN").ok();
    let local_clawd_base_url =
        local_clawd_base_url_from_internal_listen(internal_listen.as_deref());
    let mut cmd = prepared.command;
    if let Some(report) = apply_skill_runner_env_isolation(&mut cmd) {
        tracing::info!(
            "skill_dispatch skill={} env_strict=on preserved={:?} stripped_parent_env={}",
            canonical_skill_name,
            report.preserved,
            report.stripped_count
        );
    }
    place_subprocess_in_own_process_group(&mut cmd);
    cmd.kill_on_drop(true);
    cmd.env("SKILL_TIMEOUT_SECONDS", skill_timeout_secs.to_string())
        .env("CLAWD_BASE_URL", &local_clawd_base_url)
        .env(
            "APP_UNRESTRICTED_ADMIN",
            if unrestricted_admin { "1" } else { "0" },
        )
        .env(
            "APP_ALLOW_PATH_OUTSIDE_WORKSPACE",
            if allow_path_outside_workspace {
                "1"
            } else {
                "0"
            },
        )
        .env("APP_ALLOW_SUDO", if allow_sudo { "1" } else { "0" })
        .env(
            "APP_SKILL_PACKAGES_ROOT",
            package_root.display().to_string(),
        )
        .env(
            "WORKSPACE_ROOT",
            state.skill_rt.workspace_root.display().to_string(),
        )
        .env(
            "APP_WORKSPACE_STATE_DIR",
            claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME,
        );
    if let Some(token_store_dir) = sandbox_token_store_dir {
        cmd.env(
            "APP_SECRET_TOKEN_DIR",
            token_store_dir.display().to_string(),
        );
    }
    if let Some(backend) = inherited_sandbox_backend {
        cmd.env(skill_sdk::PARENT_SANDBOX_BACKEND_ENV, backend);
    } else if let Some(storage_directory) = sandbox_storage_directory {
        cmd.env(
            skill_sdk::SKILL_STORAGE_WRITABLE_DIRECTORY_ENV,
            storage_directory,
        );
    }
    if let Some(token) = &internal_llm_token {
        cmd.env(
            "AGENT_INTERNAL_LLM_URL",
            format!("{}/v1/internal/llm/text", local_clawd_base_url),
        )
        .env("AGENT_INTERNAL_LLM_TOKEN", token);
    }
    if let Some(token) = &internal_admission_token {
        cmd.env(
            "AGENT_INTERNAL_ADMISSION_URL",
            format!("{}/v1/internal/skills/admit", local_clawd_base_url),
        )
        .env("AGENT_INTERNAL_ADMISSION_TOKEN", token);
    }
    if let Some(connection) = &selected_llm_connection {
        cmd.env("OPENAI_BASE_URL", &connection.base_url)
            .env("OPENAI_MODEL", &connection.model)
            .env("APP_SELECTED_LLM_VENDOR", &connection.vendor)
            .env("APP_SELECTED_LLM_PROVIDER_TYPE", &connection.provider_type);
    }
    if !selected_provider_key_tokens.is_empty() {
        tracing::info!(
            skill = canonical_skill_name,
            vendor = selected_llm_connection
                .as_ref()
                .map(|connection| connection.vendor.as_str())
                .unwrap_or("unknown"),
            provider_type = selected_llm_connection
                .as_ref()
                .map(|connection| connection.provider_type.as_str())
                .unwrap_or("unknown"),
            credential_envs = ?selected_provider_env_names,
            "skill_dispatch_selected_provider_credentials"
        );
    }
    for (env_name, token) in &selected_provider_key_tokens {
        cmd.env(env_name, token);
    }
    for (env_name, token) in &tokenized_secret_envs {
        cmd.env(env_name, token);
    }
    cmd.current_dir(&state.skill_rt.workspace_root);
    let spawn_runner = |command| {
        WarmRunnerProcess::spawn(command).map_err(|err| {
            format!(
                "spawn skill-runner failed: path={} err={}",
                state.skill_rt.skill_runner_path.display(),
                err
            )
        })
    };
    let warm_key = warm_reuse_allowed.then(|| WarmRunnerKey {
        scope_token: canonical_skill_name.to_string(),
        version_pin: installed_launch.clone(),
        admission_binding: admission_binding.cloned(),
        registry_generation: skill_views.binding.registry_generation,
        registry_generation_digest: skill_views.binding.registry_generation_digest.clone(),
        base_registry_digest: skill_views.binding.base_registry_digest.clone(),
        overlay_generation_digest: skill_views.binding.overlay_generation_digest.clone(),
        sandbox_backend: sandbox_backend.to_string(),
        timeout_seconds: skill_timeout_secs,
    });
    let (mut runner_process, runner_dispatch_mode, runner_fallback_reason, reusable_key) =
        match warm_key {
            Some(key) => match state.skill_rt.runner_pool.checkout(&key) {
                WarmPoolCheckout::Reused(process, epoch) => {
                    (process, "warm_reused", None, Some((key, epoch)))
                }
                WarmPoolCheckout::Spawn(epoch) => {
                    (spawn_runner(cmd)?, "warm_spawned", None, Some((key, epoch)))
                }
                WarmPoolCheckout::Fallback(reason) => {
                    (spawn_runner(cmd)?, "per_request", Some(reason), None)
                }
            },
            None => (
                spawn_runner(cmd)?,
                "per_request",
                Some("execution_profile_ineligible"),
                None,
            ),
        };
    runner_process
        .send(&req_line)
        .await
        .map_err(|err| format!("write skill-runner stdin failed: {err}"))?;

    let mut out_line = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(skill_timeout_secs.max(1));
    let mut observed_frames = 0_u64;
    let mut accepted_times = VecDeque::<Instant>::new();
    loop {
        let record = match tokio::time::timeout_at(deadline, runner_process.records.next()).await {
            Ok(Some(Ok(line))) => line,
            Ok(Some(Err(LinesCodecError::MaxLineLengthExceeded))) => {
                tracing::warn!(
                    skill = canonical_skill_name,
                    reason_code = "runner_record_oversized",
                    "skill_progress_frame_discarded"
                );
                continue;
            }
            Ok(Some(Err(LinesCodecError::Io(err)))) => {
                return Err(format!("read skill-runner stdout failed: {err}"));
            }
            Ok(None) => break,
            Err(_) => {
                let _ = terminate_subprocess_group(runner_process.id()).await;
                runner_process.kill_and_wait().await;
                return Err("skill-runner timeout".to_string());
            }
        };
        let progress_record = serde_json::from_str::<Value>(&record)
            .ok()
            .and_then(|value| {
                (value.get("record_type").and_then(Value::as_str)
                    == Some(skill_sdk::SKILL_PROGRESS_FRAME_RECORD_TYPE))
                .then_some(value)
            });
        let Some(progress_record) = progress_record else {
            out_line = record;
            break;
        };

        observed_frames = observed_frames.saturating_add(1);
        if !installed_launch.progress_frames {
            tracing::warn!(
                skill = canonical_skill_name,
                reason_code = "progress_frames_not_declared",
                observed_frames,
                "skill_progress_frame_discarded"
            );
            continue;
        }
        if observed_frames > skill_sdk::MAX_PROGRESS_FRAMES_PER_INVOCATION {
            tracing::warn!(
                skill = canonical_skill_name,
                reason_code = "progress_frame_total_limit",
                observed_frames,
                "skill_progress_frame_discarded"
            );
            continue;
        }
        let encoded = serde_json::to_vec(&progress_record)
            .map_err(|err| format!("encode skill progress frame failed: {err}"))?;
        let frame_validation = skill_sdk::validate_progress_frame_line(&encoded, &task.task_id);
        let frame = match frame_validation {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(
                    skill = canonical_skill_name,
                    reason_code = error.code,
                    observed_frames,
                    "skill_progress_frame_discarded"
                );
                continue;
            }
        };
        let now = Instant::now();
        while accepted_times
            .front()
            .is_some_and(|accepted| now.duration_since(*accepted) >= Duration::from_secs(1))
        {
            accepted_times.pop_front();
        }
        if accepted_times.len() >= skill_sdk::MAX_PROGRESS_FRAMES_PER_SECOND {
            tracing::warn!(
                skill = canonical_skill_name,
                reason_code = "progress_frame_rate_limit",
                observed_frames,
                "skill_progress_frame_discarded"
            );
            continue;
        }
        accepted_times.push_back(now);
        let payload = json!({
            "schema_version": 1,
            "source": "skill_progress",
            "data_only": true,
            "render_owner": "ui_cli_channel_projection",
            "skill_name": canonical_skill_name,
            "skill_version": &installed_launch.version,
            "frame": frame,
        });
        if let Err(error) = crate::task_event_transport::publish_claimed_event(
            state,
            task,
            "skill_progress",
            payload,
        ) {
            tracing::warn!(
                skill = canonical_skill_name,
                error = %error,
                "skill_progress_event_publish_failed"
            );
        }
    }

    let mut err_line = String::new();
    if reusable_key.is_none() {
        runner_process
            .shutdown()
            .await
            .map_err(|err| format!("wait skill-runner failed: {err}"))?;
        let mut stderr = runner_process.take_stderr();
        err_line = read_skill_runner_stderr_line(&mut stderr).await;
    }

    if out_line.trim().is_empty() {
        if err_line.is_empty() {
            let mut stderr = runner_process.take_stderr();
            err_line = read_skill_runner_stderr_line(&mut stderr).await;
        }
        let detail = err_line.trim();
        if detail.is_empty() {
            return Err("empty skill-runner output".to_string());
        }
        return Err(format!("empty skill-runner output: {detail}"));
    }

    let mut response: serde_json::Value = serde_json::from_str(out_line.trim())
        .map_err(|err| format!("invalid skill-runner json: {err}"))?;
    remap_sandbox_artifact_paths(
        &mut response,
        &sandbox_artifact_output_directory,
        &artifact_output_directory,
    );
    validate_runner_execution_binding(
        &response,
        &installed_launch.skill_name,
        &installed_launch.version,
        &installed_launch.manifest_digest,
        &installed_launch.receipt_digest,
        skill_views.binding.registry_generation,
        skill_views.binding.registry_generation_digest.as_deref(),
        skill_views.binding.base_registry_digest.as_deref(),
        skill_views.binding.overlay_generation_digest.as_deref(),
        admission_binding.and_then(|binding| binding.policy_digest.as_deref()),
        admission_binding.map(|binding| binding.admission_receipt_digest.as_str()),
    )?;
    add_runner_dispatch_metadata(&mut response, runner_dispatch_mode, runner_fallback_reason);
    if let Some((key, epoch)) = reusable_key {
        state
            .skill_rt
            .runner_pool
            .checkin(key, epoch, runner_process);
    }
    tracing::info!(
        skill = canonical_skill_name,
        version = installed_launch.version,
        adapter = installed_launch.adapter.as_token(),
        receipt_digest = installed_launch.receipt_digest,
        registry_generation = skill_views.binding.registry_generation,
        registry_generation_digest = skill_views
            .binding
            .registry_generation_digest
            .as_deref()
            .unwrap_or("unavailable"),
        duration_ms = dispatch_started.elapsed().as_millis() as u64,
        status = response
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("invalid"),
        runner_dispatch_mode,
        runner_fallback_reason = runner_fallback_reason.unwrap_or("none"),
        "verified_skill_execution_completed"
    );
    Ok(response)
}

fn validate_runner_execution_binding(
    response: &Value,
    expected_skill_name: &str,
    expected_version: &str,
    expected_manifest_digest: &str,
    expected_receipt_digest: &str,
    expected_registry_generation: u64,
    expected_registry_generation_digest: Option<&str>,
    expected_base_registry_digest: Option<&str>,
    expected_overlay_generation_digest: Option<&str>,
    expected_policy_digest: Option<&str>,
    expected_admission_receipt_digest: Option<&str>,
) -> Result<(), String> {
    let binding = response.pointer("/extra/execution_binding");
    if binding.is_none()
        && response
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status != "ok")
    {
        return Ok(());
    }
    let binding = binding
        .and_then(Value::as_object)
        .ok_or_else(|| "skill runner execution binding is missing".to_string())?;
    for (field, expected_value) in [
        ("skill_name", expected_skill_name),
        ("version", expected_version),
        ("manifest_digest", expected_manifest_digest),
        ("receipt_digest", expected_receipt_digest),
    ] {
        let actual = binding.get(field).and_then(Value::as_str);
        if actual != Some(expected_value) {
            return Err(format!(
                "skill runner execution binding mismatch: field={field}"
            ));
        }
    }
    if binding.get("registry_generation").and_then(Value::as_u64)
        != Some(expected_registry_generation)
    {
        return Err(
            "skill runner execution binding mismatch: field=registry_generation".to_string(),
        );
    }
    for (field, expected_value) in [
        (
            "registry_generation_digest",
            expected_registry_generation_digest,
        ),
        ("base_registry_digest", expected_base_registry_digest),
        (
            "overlay_generation_digest",
            expected_overlay_generation_digest,
        ),
        ("policy_digest", expected_policy_digest),
        (
            "admission_receipt_digest",
            expected_admission_receipt_digest,
        ),
    ] {
        let actual = binding.get(field).and_then(Value::as_str);
        if actual != expected_value {
            return Err(format!(
                "skill runner execution binding mismatch: field={field}"
            ));
        }
    }
    Ok(())
}

fn action_scoped_runner_capabilities(
    mut capabilities: Vec<Capability>,
    mapping: Option<&PlannerCapabilityMapping>,
) -> Vec<Capability> {
    let Some(mapping) = mapping else {
        return capabilities;
    };
    capabilities.retain(|capability| match capability {
        Capability::Llm => {
            mapping.network_access != Some(false) && mapping.credential_access != Some(false)
        }
        Capability::LlmCredentialFallback(_) => {
            mapping.network_access != Some(false) && mapping.credential_access != Some(false)
        }
        Capability::Net => mapping.network_access != Some(false),
        Capability::FsWrite => mapping.filesystem_write != Some(false),
        Capability::Exec | Capability::ExecSudo => mapping.subprocess != Some(false),
        Capability::Secrets(_) => mapping.credential_access != Some(false),
        Capability::FsRead => true,
    });
    capabilities
}

fn action_scoped_runner_sandbox_mode(
    default_mode: ToolSandboxMode,
    mapping: Option<&PlannerCapabilityMapping>,
) -> ToolSandboxMode {
    if default_mode == ToolSandboxMode::DangerFull {
        ToolSandboxMode::DangerFull
    } else if mapping.is_some_and(|mapping| {
        mapping.isolation_profile == Some(CapabilityIsolationProfile::ReadOnly)
            || mapping.filesystem_write == Some(false)
    }) {
        ToolSandboxMode::ReadOnly
    } else {
        default_mode
    }
}

fn stateless_readonly_reuse_allowed(
    execution_profile: skill_sdk::ExecutionProfile,
    capabilities: &[Capability],
    mapping: Option<&PlannerCapabilityMapping>,
    sandbox_mode: ToolSandboxMode,
    has_storage: bool,
    has_secrets: bool,
    admission_capable: bool,
    unrestricted_admin: bool,
    allow_path_outside_workspace: bool,
    allow_sudo: bool,
) -> bool {
    let Some(mapping) = mapping else {
        return false;
    };
    execution_profile == skill_sdk::ExecutionProfile::StatelessReadonly
        && matches!(
            mapping.effect,
            Some(PlannerCapabilityEffect::Observe | PlannerCapabilityEffect::Validate)
        )
        && mapping.once_per_task != Some(true)
        && mapping.idempotent != Some(false)
        && mapping.network_access != Some(true)
        && mapping.filesystem_write != Some(true)
        && mapping.external_publish != Some(true)
        && mapping.credential_access != Some(true)
        && mapping.subprocess != Some(true)
        && mapping.package_install != Some(true)
        && mapping.privilege_escalation != Some(true)
        && sandbox_mode == ToolSandboxMode::ReadOnly
        && capabilities
            .iter()
            .all(|capability| matches!(capability, Capability::FsRead))
        && !has_storage
        && !has_secrets
        && !admission_capable
        && !unrestricted_admin
        && !allow_path_outside_workspace
        && !allow_sudo
}

fn add_runner_dispatch_metadata(
    response: &mut Value,
    mode: &'static str,
    fallback_reason: Option<&'static str>,
) {
    let Some(response) = response.as_object_mut() else {
        return;
    };
    let extra = response
        .entry("extra")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Some(extra) = extra.as_object_mut() else {
        return;
    };
    extra.insert(
        "runner_dispatch".to_string(),
        json!({
            "schema_version": 1,
            "mode": mode,
            "fallback_reason": fallback_reason,
        }),
    );
}

pub(crate) fn build_runner_skill_context(
    state: &AppState,
    task: &ClaimedTask,
    source: &str,
    storage_descriptor: Option<crate::skill_storage::SkillStorageDescriptor>,
    artifact_output_directory: &std::path::Path,
    execution_context: Option<&super::SkillExecutionContext>,
) -> Value {
    let mut ctx = serde_json::Map::new();
    ctx.insert("source".to_string(), Value::String(source.to_string()));
    ctx.insert("kind".to_string(), Value::String("run_skill".to_string()));
    let auth_role = current_task_auth_role(state, task).unwrap_or_else(|| "unknown".to_string());
    let unrestricted_admin =
        crate::task_execution_policy::task_has_unrestricted_admin_authority(state, task);
    let allow_path_outside_workspace = task_allows_path_outside_workspace(state, Some(task));
    let allow_sudo = task_allows_sudo(state, Some(task));
    ctx.insert("auth_role".to_string(), Value::String(auth_role));
    ctx.insert(
        "authority_scope".to_string(),
        Value::String(
            if unrestricted_admin {
                "unrestricted_admin"
            } else {
                "configured"
            }
            .to_string(),
        ),
    );
    ctx.insert(
        "allow_path_outside_workspace".to_string(),
        Value::Bool(allow_path_outside_workspace),
    );
    ctx.insert("allow_sudo".to_string(), Value::Bool(allow_sudo));
    ctx.insert(
        "permissions".to_string(),
        serde_json::json!({
            "unrestricted_admin": unrestricted_admin,
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
    if let Some(storage_descriptor) = storage_descriptor {
        ctx.insert(
            "skill_storage".to_string(),
            serde_json::to_value(storage_descriptor).unwrap_or(Value::Null),
        );
    }
    ctx.insert(
        "artifact_output_directory".to_string(),
        Value::String(artifact_output_directory.display().to_string()),
    );
    if let Some(execution_context) = execution_context {
        ctx.insert(
            "execution".to_string(),
            serde_json::json!({
                "schema_version": 1,
                "action_ref": execution_context.action_ref,
                "idempotency_key": execution_context.idempotency_key,
                "attempt_no": execution_context.attempt_no,
            }),
        );
    }
    let locale_tag = super::task_request_locale_tag(state, task);
    ctx.insert("locale".to_string(), Value::String(locale_tag.clone()));
    ctx.insert("language".to_string(), Value::String(locale_tag));
    ctx.insert(
        "workspace_root".to_string(),
        Value::String(state.skill_rt.workspace_root.display().to_string()),
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

fn remap_sandbox_artifact_paths(
    value: &mut Value,
    sandbox_directory: &std::path::Path,
    host_directory: &std::path::Path,
) {
    match value {
        Value::String(text) => {
            let sandbox = sandbox_directory.to_string_lossy();
            if let Some(suffix) = text.strip_prefix(sandbox.as_ref()) {
                *text = format!("{}{}", host_directory.display(), suffix);
            }
        }
        Value::Array(items) => {
            for item in items {
                remap_sandbox_artifact_paths(item, sandbox_directory, host_directory);
            }
        }
        Value::Object(object) => {
            for item in object.values_mut() {
                remap_sandbox_artifact_paths(item, sandbox_directory, host_directory);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn storage_descriptor_for_skill(
    state: &AppState,
    canonical_skill_name: &str,
) -> Result<Option<crate::skill_storage::SkillStorageDescriptor>, String> {
    let registry = state.get_skills_registry();
    let Some(declaration) = registry
        .as_ref()
        .and_then(|registry| registry.storage(canonical_skill_name))
    else {
        return Ok(None);
    };
    if declaration.kind != "sqlite"
        || declaration.schema_version == 0
        || declaration.migration_owner != canonical_skill_name
    {
        return Err(format!(
            "invalid skill storage declaration: skill={canonical_skill_name}"
        ));
    }
    state
        .core
        .skill_storage
        .descriptor(canonical_skill_name, declaration.schema_version)
        .map(Some)
        .map_err(|error| {
            format!("skill storage unavailable: skill={canonical_skill_name} error={error}")
        })
}
