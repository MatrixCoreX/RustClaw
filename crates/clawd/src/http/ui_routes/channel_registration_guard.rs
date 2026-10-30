#[derive(Debug, thiserror::Error)]
enum ChannelRegistrationRejection {
    #[error("channel_bind_owner_unavailable")]
    OwnerUnavailable,
    #[error("channel_bind_admin_required")]
    AdministratorRequired,
    #[error("channel_bind_session_not_found")]
    SessionNotFound,
}

impl ChannelRegistrationRejection {
    fn status(&self) -> StatusCode {
        match self {
            Self::OwnerUnavailable => StatusCode::UNAUTHORIZED,
            Self::AdministratorRequired => StatusCode::FORBIDDEN,
            Self::SessionNotFound => StatusCode::NOT_FOUND,
        }
    }
}

fn channel_registration_failure_response(
    error: &anyhow::Error,
    fallback: String,
) -> (
    StatusCode,
    Json<ApiResponse<FeishuBindSessionStatusResponse>>,
) {
    let rejection = error.downcast_ref::<ChannelRegistrationRejection>();
    (
        rejection.map_or(StatusCode::INTERNAL_SERVER_ERROR, |cause| cause.status()),
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(rejection.map_or_else(|| fallback, ToString::to_string)),
        }),
    )
}

fn refresh_registration_session(
    state: &AppState,
    platform: AgentAppChannel,
    expected: &PendingChannelBindSession,
) -> anyhow::Result<PendingChannelBindSession> {
    let identity = resolve_auth_identity_by_key(state, &expected.user_key)?
        .ok_or(ChannelRegistrationRejection::OwnerUnavailable)?;
    if !identity.role.eq_ignore_ascii_case("admin") {
        return Err(ChannelRegistrationRejection::AdministratorRequired.into());
    }
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
