#[derive(Debug, thiserror::Error)]
enum ChannelRegistrationRejection {
    #[error("channel_bind_owner_unavailable")]
    OwnerUnavailable,
    #[error("channel_bind_admin_required")]
    AdministratorRequired,
    #[error("channel_bind_session_not_found")]
    SessionNotFound,
    #[error("channel_bind_application_recovery_required")]
    ApplicationRecoveryRequired,
    #[error("channel_bind_registration_superseded")]
    RegistrationSuperseded,
    #[error("channel_bind_registration_prepare_required")]
    RegistrationPrepareRequired,
}

impl ChannelRegistrationRejection {
    fn status(&self) -> StatusCode {
        match self {
            Self::OwnerUnavailable => StatusCode::UNAUTHORIZED,
            Self::AdministratorRequired => StatusCode::FORBIDDEN,
            Self::SessionNotFound => StatusCode::NOT_FOUND,
            Self::ApplicationRecoveryRequired
            | Self::RegistrationSuperseded
            | Self::RegistrationPrepareRequired => StatusCode::CONFLICT,
        }
    }
}

fn channel_registration_failure_response<T>(
    error: &anyhow::Error,
    fallback: String,
) -> (StatusCode, Json<ApiResponse<T>>) {
    let rejection = error.downcast_ref::<ChannelRegistrationRejection>();
    let journal =
        error.downcast_ref::<crate::repo::channel_registration::RegistrationJournalError>();
    let configuration = error.downcast_ref::<ChannelConfigurationOperationError>();
    let transport = error.downcast_ref::<ChannelRegistrationTransportError>();
    let status = rejection
        .map(|cause| cause.status())
        .or_else(|| journal.map(|_| StatusCode::CONFLICT))
        .or_else(|| configuration.map(|_| StatusCode::INTERNAL_SERVER_ERROR))
        .or_else(|| transport.map(|_| StatusCode::BAD_GATEWAY))
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let code = rejection
        .map(ToString::to_string)
        .or_else(|| {
            journal.map(|cause| {
                use crate::repo::channel_registration::RegistrationJournalError;
                match cause {
                    RegistrationJournalError::Superseded => {
                        ChannelRegistrationRejection::RegistrationSuperseded.to_string()
                    }
                    RegistrationJournalError::AlreadyPrepared => {
                        ChannelRegistrationRejection::RegistrationPrepareRequired.to_string()
                    }
                    RegistrationJournalError::ClaimLost => {
                        ChannelRegistrationRejection::ApplicationRecoveryRequired.to_string()
                    }
                    _ => cause.to_string(),
                }
            })
        })
        .or_else(|| configuration.map(ToString::to_string))
        .or_else(|| transport.map(ToString::to_string));
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(code.unwrap_or(fallback)),
        }),
    )
}

fn refresh_registration_session(
    state: &AppState,
    platform: AgentAppChannel,
    expected: &PendingChannelBindSession,
) -> anyhow::Result<PendingChannelBindSession> {
    require_current_registration_admin(state, &expected.user_key)?;
    let mut db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("db pool: {err}"))?;
    let current = get_pending_channel_bind_session_by_id(&db, expected.id)?
        .filter(|session| {
            session.user_key == expected.user_key && session.channel == platform.channel()
        })
        .ok_or(ChannelRegistrationRejection::SessionNotFound)?;
    maybe_expire_channel_bind_session(&mut db, current)
}

fn require_current_registration_admin(
    state: &AppState,
    actor_key: &str,
) -> anyhow::Result<AuthIdentity> {
    let identity = resolve_auth_identity_by_key(state, actor_key)?
        .ok_or(ChannelRegistrationRejection::OwnerUnavailable)?;
    if !identity.role.eq_ignore_ascii_case("admin") {
        return Err(ChannelRegistrationRejection::AdministratorRequired.into());
    }
    Ok(identity)
}
