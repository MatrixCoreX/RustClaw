const GIT_REMOTE_CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
struct GitCredentialStatus {
    name: &'static str,
    configured: bool,
    managed_by_environment: bool,
}

#[derive(Debug, Clone, Serialize)]
struct GitConnectionsResponse {
    schema_version: u32,
    revision: u64,
    editable: bool,
    profiles: Vec<claw_core::git_remote_config::GitConnectionProfile>,
    credentials: Vec<GitCredentialStatus>,
}

#[derive(Debug, Deserialize)]
struct UpsertGitConnectionRequest {
    expected_revision: u64,
    id: String,
    allowed_owners: Vec<String>,
    allowed_repositories: Vec<String>,
    #[serde(default = "default_github_git_username")]
    git_username: String,
}

#[derive(Debug, Deserialize)]
struct DeleteGitConnectionQuery {
    expected_revision: u64,
}

#[derive(Debug, Deserialize)]
struct SetGitCredentialRequest {
    value: String,
}

fn default_github_git_username() -> String {
    "x-access-token".to_string()
}

fn git_connections_response(
    state: &AppState,
    editable: bool,
) -> anyhow::Result<GitConnectionsResponse> {
    let connection_path = claw_core::git_remote_config::git_connection_store_path(
        &state.skill_rt.workspace_root,
    );
    let credential_path = claw_core::git_remote_config::git_credential_store_path(
        &state.skill_rt.workspace_root,
    );
    let document = claw_core::git_remote_config::load_git_connections(&connection_path)?;
    let broker = claw_core::secrets::EnvFileSecretsBroker::new(credential_path);
    let credentials = [
        claw_core::git_remote_config::GITHUB_GIT_CREDENTIAL_REF,
        claw_core::git_remote_config::GITHUB_API_CREDENTIAL_REF,
    ]
    .into_iter()
    .map(|name| {
        use claw_core::secrets::SecretsBroker as _;
        let environment = claw_core::secrets::EnvSecretsBroker::new();
        let managed_by_environment = environment.lookup(name)?.is_some();
        broker.lookup(name).map(|value| GitCredentialStatus {
            name,
            configured: value.is_some(),
            managed_by_environment,
        })
    })
    .collect::<Result<Vec<_>, _>>()?;
    Ok(GitConnectionsResponse {
        schema_version: GIT_REMOTE_CONFIG_SCHEMA_VERSION,
        revision: document.revision,
        editable,
        profiles: document.profiles,
        credentials,
    })
}

async fn get_git_connections(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<GitConnectionsResponse>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(response))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: response.ok,
                    data: None,
                    error: response.error,
                }),
            );
        }
    };
    match git_connections_response(&state, identity.role.eq_ignore_ascii_case("admin")) {
        Ok(data) => git_api_data(StatusCode::OK, data),
        Err(_) => git_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "git_connection_store_unavailable",
        ),
    }
}

async fn upsert_git_connection_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpsertGitConnectionRequest>,
) -> (StatusCode, Json<ApiResponse<GitConnectionsResponse>>) {
    let identity = match require_git_admin(&state, &headers) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let profile = claw_core::git_remote_config::GitConnectionProfile {
        id: request.id,
        forge_kind: "github".to_string(),
        git_host: "github.com".to_string(),
        api_host: "api.github.com".to_string(),
        allowed_owners: request.allowed_owners,
        allowed_repositories: request.allowed_repositories,
        git_username: request.git_username,
        auth_scheme: "token".to_string(),
        git_credential_ref:
            claw_core::git_remote_config::GITHUB_GIT_CREDENTIAL_REF.to_string(),
        api_credential_ref:
            claw_core::git_remote_config::GITHUB_API_CREDENTIAL_REF.to_string(),
    };
    let path = claw_core::git_remote_config::git_connection_store_path(
        &state.skill_rt.workspace_root,
    );
    let document = match claw_core::git_remote_config::upsert_git_connection(
        &path,
        request.expected_revision,
        profile,
    ) {
        Ok(document) => document,
        Err(error) => return git_config_error_response(&error),
    };
    audit_git_config_change(
        &state,
        identity.user_id,
        "git_connection_upsert",
        json!({"revision": document.revision}),
    );
    match git_connections_response(&state, true) {
        Ok(data) => git_api_data(StatusCode::OK, data),
        Err(_) => git_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "git_connection_store_unavailable",
        ),
    }
}

async fn delete_git_connection_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(connection_id): AxumPath<String>,
    Query(query): Query<DeleteGitConnectionQuery>,
) -> (StatusCode, Json<ApiResponse<GitConnectionsResponse>>) {
    let identity = match require_git_admin(&state, &headers) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let path = claw_core::git_remote_config::git_connection_store_path(
        &state.skill_rt.workspace_root,
    );
    let document = match claw_core::git_remote_config::delete_git_connection(
        &path,
        query.expected_revision,
        &connection_id,
    ) {
        Ok(document) => document,
        Err(error) => return git_config_error_response(&error),
    };
    audit_git_config_change(
        &state,
        identity.user_id,
        "git_connection_delete",
        json!({"connection_id": connection_id, "revision": document.revision}),
    );
    match git_connections_response(&state, true) {
        Ok(data) => git_api_data(StatusCode::OK, data),
        Err(_) => git_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "git_connection_store_unavailable",
        ),
    }
}

async fn set_git_credential_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(credential_name): AxumPath<String>,
    Json(request): Json<SetGitCredentialRequest>,
) -> (StatusCode, Json<ApiResponse<GitConnectionsResponse>>) {
    let identity = match require_git_admin(&state, &headers) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let credential_name = match allowed_git_credential_name(&credential_name) {
        Some(name) => name,
        None => return git_api_error(StatusCode::BAD_REQUEST, "git_credential_name_invalid"),
    };
    if git_credential_is_managed_by_environment(credential_name) {
        return git_api_error(
            StatusCode::CONFLICT,
            "git_credential_managed_by_environment",
        );
    }
    let path = claw_core::git_remote_config::git_credential_store_path(
        &state.skill_rt.workspace_root,
    );
    if claw_core::secrets::set_file_secret(&path, credential_name, &request.value).is_err() {
        return git_api_error(StatusCode::BAD_REQUEST, "git_credential_write_failed");
    }
    audit_git_config_change(
        &state,
        identity.user_id,
        "git_credential_set",
        json!({"credential_ref": credential_name}),
    );
    match git_connections_response(&state, true) {
        Ok(data) => git_api_data(StatusCode::OK, data),
        Err(_) => git_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "git_connection_store_unavailable",
        ),
    }
}

async fn delete_git_credential_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(credential_name): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<GitConnectionsResponse>>) {
    let identity = match require_git_admin(&state, &headers) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let credential_name = match allowed_git_credential_name(&credential_name) {
        Some(name) => name,
        None => return git_api_error(StatusCode::BAD_REQUEST, "git_credential_name_invalid"),
    };
    if git_credential_is_managed_by_environment(credential_name) {
        return git_api_error(
            StatusCode::CONFLICT,
            "git_credential_managed_by_environment",
        );
    }
    let path = claw_core::git_remote_config::git_credential_store_path(
        &state.skill_rt.workspace_root,
    );
    if claw_core::secrets::delete_file_secret(&path, credential_name).is_err() {
        return git_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "git_credential_delete_failed",
        );
    }
    audit_git_config_change(
        &state,
        identity.user_id,
        "git_credential_delete",
        json!({"credential_ref": credential_name}),
    );
    match git_connections_response(&state, true) {
        Ok(data) => git_api_data(StatusCode::OK, data),
        Err(_) => git_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "git_connection_store_unavailable",
        ),
    }
}

fn allowed_git_credential_name(value: &str) -> Option<&'static str> {
    match value.trim() {
        claw_core::git_remote_config::GITHUB_GIT_CREDENTIAL_REF => {
            Some(claw_core::git_remote_config::GITHUB_GIT_CREDENTIAL_REF)
        }
        claw_core::git_remote_config::GITHUB_API_CREDENTIAL_REF => {
            Some(claw_core::git_remote_config::GITHUB_API_CREDENTIAL_REF)
        }
        _ => None,
    }
}

fn git_credential_is_managed_by_environment(name: &str) -> bool {
    use claw_core::secrets::SecretsBroker as _;
    claw_core::secrets::EnvSecretsBroker::new()
        .lookup(name)
        .ok()
        .flatten()
        .is_some()
}

fn require_git_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, (StatusCode, Json<ApiResponse<GitConnectionsResponse>>)> {
    let identity = require_ui_identity(state, headers).map_err(|(status, Json(response))| {
        (
            status,
            Json(ApiResponse {
                ok: response.ok,
                data: None,
                error: response.error,
            }),
        )
    })?;
    if !identity.role.eq_ignore_ascii_case("admin") {
        return Err(git_api_error(
            StatusCode::FORBIDDEN,
            "git_admin_required",
        ));
    }
    Ok(identity)
}

fn git_config_error_response(
    error: &anyhow::Error,
) -> (StatusCode, Json<ApiResponse<GitConnectionsResponse>>) {
    let token = error.to_string();
    let (status, code) = match token.as_str() {
        "git_connection_revision_conflict" => {
            (StatusCode::CONFLICT, "git_connection_revision_conflict")
        }
        "git_connection_not_found" => (StatusCode::NOT_FOUND, "git_connection_not_found"),
        "git_connection_limit_exceeded" => {
            (StatusCode::CONFLICT, "git_connection_limit_exceeded")
        }
        value if value.starts_with("git_connection_") || value.starts_with("git_username_") => {
            (StatusCode::BAD_REQUEST, "git_connection_invalid")
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "git_connection_store_unavailable",
        ),
    };
    git_api_error(status, code)
}

fn git_api_data<T>(status: StatusCode, data: T) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        status,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

fn git_api_error<T>(status: StatusCode, code: &str) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(code.to_string()),
        }),
    )
}

fn audit_git_config_change(state: &AppState, user_id: i64, action: &str, detail: Value) {
    if let Err(error) = crate::repo::insert_audit_log(
        state,
        Some(user_id),
        action,
        Some(&detail.to_string()),
        None,
    ) {
        tracing::warn!(action, error = %error, "git_config_audit_failed");
    }
}
