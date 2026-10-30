use crate::repo::channel_registration::{
    claim_registration_application, finish_registration_application, registration_gate,
    RegistrationClaimOutcome, RegistrationGate, RegistrationOutcome,
};

fn pending_registration_configuration(
    state: &AppState,
    platform: AgentAppChannel,
    session: &PendingChannelBindSession,
) -> anyhow::Result<Option<String>> {
    let gate = {
        let db = state.core.db.get()?;
        registration_gate(&db, session)?
    };
    match gate {
        RegistrationGate::Ready { config_digest } => {
            let observed = channel_configuration_fingerprint(state, platform)?;
            if observed != config_digest {
                return Err(ChannelRegistrationRejection::RegistrationSuperseded.into());
            }
            Ok(Some(observed))
        }
        RegistrationGate::Applied => Ok(None),
        RegistrationGate::RecoveryRequired => {
            Err(ChannelRegistrationRejection::ApplicationRecoveryRequired.into())
        }
        RegistrationGate::Superseded => {
            Err(ChannelRegistrationRejection::RegistrationSuperseded.into())
        }
        RegistrationGate::Unprepared => {
            Err(ChannelRegistrationRejection::RegistrationPrepareRequired.into())
        }
    }
}

fn write_generated_credentials(
    state: &AppState,
    platform: AgentAppChannel,
    app_id: &str,
    app_secret: &str,
) -> anyhow::Result<()> {
    let (relative_path, output) = match platform {
        AgentAppChannel::Feishu => {
            let raw = read_feishu_config_raw(state)?;
            (
                "configs/channels/feishu.toml",
                update_feishu_config_raw_preserving_format(&raw, app_id, app_secret),
            )
        }
        AgentAppChannel::Lark => {
            let raw = read_lark_config_raw(state)?;
            (
                "configs/channels/lark.toml",
                update_lark_config_raw_preserving_format(&raw, app_id, app_secret),
            )
        }
    };
    write_workspace_and_mounted_file(&state.skill_rt.workspace_root, relative_path, &output)?;
    Ok(())
}

async fn start_service_if_needed(state: &AppState, service: &str) -> anyhow::Result<()> {
    if service_is_running(service) {
        return Ok(());
    }
    let profile = claw_core::product_identity::env_string("START_PROFILE")
        .ok()
        .filter(|v| matches!(v.as_str(), "debug" | "release"))
        .unwrap_or_else(|| runtime_profile_default().to_string());
    let script_name =
        service_start_script(service).ok_or_else(|| anyhow::anyhow!("unsupported_service"))?;
    validate_service_start_readiness(state, service)
        .map_err(|err| anyhow::anyhow!(err.error_code))?;
    let workspace = state.skill_rt.workspace_root.to_string_lossy();
    let log_file = format!("logs/{}.log", service);
    let cmd = format!(
        "cd {} && mkdir -p logs .pids && nohup ./{} {} > {} 2>&1 &",
        shell_escape_arg(workspace.as_ref()),
        script_name,
        shell_escape_arg(profile.as_str()),
        shell_escape_arg(log_file.as_str())
    );
    spawn_background_shell(&cmd)?;
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    if !service_is_running(service) {
        anyhow::bail!(
            "service did not enter running state: {service}. check logs/{service}.log and channel config"
        );
    }
    Ok(())
}

async fn maybe_complete_official_scan(
    state: &AppState,
    platform: AgentAppChannel,
    session: PendingChannelBindSession,
) -> anyhow::Result<PendingChannelBindSession> {
    maybe_complete_official_scan_with_start(
        state,
        platform,
        session,
        start_service_if_needed(state, platform.service()),
    )
    .await
}

async fn maybe_complete_official_scan_with_start<Start>(
    state: &AppState,
    platform: AgentAppChannel,
    session: PendingChannelBindSession,
    start: Start,
) -> anyhow::Result<PendingChannelBindSession>
where
    Start: std::future::Future<Output = anyhow::Result<()>>,
{
    maybe_complete_official_scan_with_effects(
        state,
        platform,
        session,
        start,
        write_generated_credentials,
    )
    .await
}

async fn maybe_complete_official_scan_with_effects<Start, Write>(
    state: &AppState,
    platform: AgentAppChannel,
    session: PendingChannelBindSession,
    start: Start,
    write: Write,
) -> anyhow::Result<PendingChannelBindSession>
where
    Start: std::future::Future<Output = anyhow::Result<()>>,
    Write: FnOnce(&AppState, AgentAppChannel, &str, &str) -> anyhow::Result<()>,
{
    if !matches!(session.status.as_str(), "pending" | "detected") {
        return Ok(session);
    }
    if session
        .install_device_code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Ok(session);
    }

    let session = {
        let _operation = lock_channel_configuration(state, platform).await?;
        let current = refresh_registration_session(state, platform, &session)?;
        if !matches!(current.status.as_str(), "pending" | "detected")
            || current.install_device_code != session.install_device_code
            || pending_registration_configuration(state, platform, &current)?.is_none()
        {
            return Ok(current);
        }
        current
    };

    let device_code = session.install_device_code.as_deref().unwrap().trim();
    let poll_result = poll_official_registration(state, platform, device_code).await;
    let _operation = lock_channel_configuration(state, platform).await?;
    // Registration I/O may outlive the actor, session, or registration attempt.
    let current = refresh_registration_session(state, platform, &session)?;
    if !matches!(current.status.as_str(), "pending" | "detected")
        || current.install_device_code != session.install_device_code
    {
        return Ok(current);
    }
    let session = current;
    let Some(config_digest) = pending_registration_configuration(state, platform, &session)? else {
        return Ok(session);
    };
    let poll = poll_result?;
    if let Some(error_code) = poll
        .error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let mut db = state.core.db.get()?;
        return match error_code {
            "authorization_pending" | "slow_down" => Ok(session),
            "expired_token" => mark_pending_channel_bind_session_expired(&mut db, session.id),
            "access_denied" => mark_pending_channel_bind_session_failed(
                &mut db,
                session.id,
                &ChannelRegistrationFailure::AccessDenied.to_string(),
            ),
            _ => mark_pending_channel_bind_session_failed(
                &mut db,
                session.id,
                &ChannelRegistrationFailure::ProviderRejected.to_string(),
            ),
        };
    }
    if let (Some(client_id), Some(client_secret), Some(_open_id)) = (
        poll.client_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        poll.client_secret
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        poll.user_info
            .as_ref()
            .and_then(|user| user.open_id.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    ) {
        let claim = {
            let mut db = state.core.db.get()?;
            match claim_registration_application(&mut db, &session, &config_digest)? {
                RegistrationClaimOutcome::Acquired(claim) => claim,
                RegistrationClaimOutcome::Gate(RegistrationGate::Applied) => return Ok(session),
                RegistrationClaimOutcome::Gate(RegistrationGate::RecoveryRequired) => {
                    return Err(ChannelRegistrationRejection::ApplicationRecoveryRequired.into())
                }
                RegistrationClaimOutcome::Gate(_) => {
                    return Err(ChannelRegistrationRejection::RegistrationSuperseded.into())
                }
            }
        };
        let applied = write(state, platform, client_id, client_secret).and_then(|()| {
            verify_applied_channel_configuration(state, platform, client_id, client_secret)
        });
        if applied.is_err() {
            let db = state.core.db.get()?;
            finish_registration_application(
                &db,
                &claim,
                RegistrationOutcome::ConfigurationWriteFailed,
            )?;
            return Err(ChannelRegistrationRejection::ApplicationRecoveryRequired.into());
        }
        let start_result = start.await;
        let current_result = refresh_registration_session(state, platform, &session);
        let receipt_result = (|| -> anyhow::Result<()> {
            let db = state.core.db.get()?;
            finish_registration_application(
                &db,
                &claim,
                if start_result.is_ok() {
                    RegistrationOutcome::ServiceStarted
                } else {
                    RegistrationOutcome::ServiceStartFailed
                },
            )
        })();
        if receipt_result.is_err() {
            tracing::warn!(
                channel = platform.channel(),
                error_code = "channel_registration_receipt_unavailable"
            );
        }
        let current = current_result?;
        if !matches!(current.status.as_str(), "pending" | "detected")
            || current.install_device_code != session.install_device_code
        {
            return Ok(current);
        }
        receipt_result?;
        if start_result.is_err() {
            let failed = {
                let mut db = state
                    .core
                    .db
                    .get()
                    .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
                mark_pending_channel_bind_session_failed(
                    &mut db,
                    session.id,
                    &ChannelRegistrationFailure::ServiceStartFailed.to_string(),
                )
            };
            return failed.or_else(|error| {
                // Conditional failure may lose to a binding finalized concurrently.
                let latest = refresh_registration_session(state, platform, &session)?;
                if !matches!(latest.status.as_str(), "pending" | "detected")
                    || latest.install_device_code != session.install_device_code
                {
                    Ok(latest)
                } else {
                    Err(error)
                }
            });
        }
        return Ok(current);
    }

    Ok(session)
}
