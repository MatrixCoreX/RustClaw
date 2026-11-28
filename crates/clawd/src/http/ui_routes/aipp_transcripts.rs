const TRANSCRIPT_CONTRACT: &str = "owned_transcript_jobs_v1";
const TRANSCRIPT_ENTRYPOINT: &str = "aipp_attachment_transcript_v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AippTranscriptRequest {
    request_id: uuid::Uuid,
    package_version: String,
    attachment: Value,
}

fn authorize_aipp_transcripts(
    state: &AppState,
    headers: &HeaderMap,
    skill: &str,
) -> Result<(AuthIdentity, ActiveAippPackage), (StatusCode, Json<ApiResponse<Value>>)> {
    let identity = require_ui_identity(state, headers)?;
    let active = active_aipp_package(state, skill)
        .ok()
        .flatten()
        .filter(|app| app.aipp.data_contract == TRANSCRIPT_CONTRACT)
        .filter(|_| aipp_is_installed(state, skill))
        .ok_or_else(|| aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available"))?;
    Ok((identity, active))
}

fn transcript_upload_kind(value: &Value) -> Option<&str> {
    let kind = value.get("kind")?.as_str()?;
    let mime = value.get("mime_type")?.as_str()?;
    let size = value.get("size")?.as_u64()?;
    if !matches!(kind, "audio" | "video")
        || mime.split_once('/')?.0 != kind
        || !(1..=crate::ui_attachments::MAX_UI_ATTACHMENT_BYTES as u64).contains(&size)
    {
        return None;
    }
    Some(kind)
}

async fn submit_aipp_transcript(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill): AxumPath<String>,
    Json(request): Json<AippTranscriptRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let (identity, app) = match authorize_aipp_transcripts(&state, &headers, &skill) {
        Ok(auth) => auth,
        Err(error) => return error,
    };
    if request.package_version != app.manifest.package.version {
        return aipp_api_error(StatusCode::CONFLICT, "aipp_package_changed");
    }
    let Some(kind) = transcript_upload_kind(&request.attachment) else {
        return aipp_api_error(
            StatusCode::BAD_REQUEST,
            "aipp_transcript_attachment_invalid",
        );
    };
    let video = kind == "video";
    let root = match state.identity_files_root(&identity) {
        Ok(root) => root,
        Err(_) => {
            return aipp_api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "workspace_scope_unavailable",
            )
        }
    };
    let client_id = request.request_id.to_string();
    let mut payload = json!({
        "entrypoint": TRANSCRIPT_ENTRYPOINT, "aipp_skill": skill,
        "aipp_package_version": request.package_version, "aipp_upload_id": client_id,
        "attachments": [request.attachment],
    });
    if let Err(code) = crate::ui_attachments::validate_upload_references(
        &root,
        identity.user_id,
        identity.chat_id,
        &client_id,
        &mut payload,
    ) {
        return aipp_api_error(StatusCode::BAD_REQUEST, &code);
    }
    if !crate::ask_flow::transcript_submission_allowed(&state, &identity, video) {
        return aipp_api_error(StatusCode::FORBIDDEN, "skill_policy_denied");
    }
    let submit = claw_core::types::SubmitTaskRequest {
        kind: claw_core::types::TaskKind::Ask,
        channel: Some(claw_core::types::ChannelKind::Ui),
        idempotency_key: Some(format!("aipp-transcript:{client_id}")),
        user_key: Some(identity.user_key),
        user_id: None,
        chat_id: None,
        external_chat_id: Some(format!("aipp-transcript:{client_id}")),
        external_user_id: None,
        ingress: None,
        payload,
    };
    let (status, Json(response)) = crate::submit_task(State(state), headers, Json(submit)).await;
    (
        status,
        Json(ApiResponse {
            ok: response.ok,
            data: response.data.map(|data| json!(data)),
            error: response.error,
        }),
    )
}

pub(crate) fn is_aipp_transcript_job(payload: &Value) -> bool {
    payload.get("entrypoint").and_then(Value::as_str) == Some(TRANSCRIPT_ENTRYPOINT)
}

pub(crate) async fn process_aipp_transcript_job(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
) -> anyhow::Result<()> {
    let transcript = execute_aipp_transcript_job(state, task, payload).await;
    match transcript {
        Ok(text) => crate::repo::update_task_success(
            state,
            &task.task_id,
            task.claim_attempt,
            &json!({ "schema_version": 1, "text": text }).to_string(),
        ),
        Err(error) => crate::repo::update_task_failure_with_result(
            state,
            &task.task_id,
            task.claim_attempt,
            &json!({"schema_version": 1, "error_code": error.error_code,
                       "message_key": error.message_key, "retryable": error.retryable})
            .to_string(),
            &error.error_code,
        ),
    }
}

async fn execute_aipp_transcript_job(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
) -> Result<String, crate::ask_flow::TranscriptJobError> {
    let skill = payload["aipp_skill"].as_str().unwrap_or_default();
    let key = task
        .user_key
        .as_deref()
        .ok_or("aipp_transcript_unauthorized")?;
    let actor = crate::resolve_auth_identity_by_key(state, key)
        .ok()
        .flatten()
        .filter(|actor| actor.user_id == task.user_id && task.channel == "ui")
        .ok_or("aipp_transcript_unauthorized")?;
    let version = payload["aipp_package_version"].as_str();
    let _app = active_aipp_package(state, skill)
        .ok()
        .flatten()
        .filter(|app| {
            app.aipp.data_contract == TRANSCRIPT_CONTRACT
                && Some(app.manifest.package.version.as_str()) == version
        })
        .filter(|_| aipp_is_installed(state, skill))
        .ok_or("aipp_not_available")?;
    let client_id = payload["aipp_upload_id"]
        .as_str()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .ok_or("aipp_transcript_attachment_invalid")?;
    let root = state
        .identity_files_root(&actor)
        .map_err(|_| "workspace_scope_unavailable")?;
    // Public job submission cannot replace upload ownership metadata or attach
    // alternate locators. Rehydrate only the single manifest-owned attachment.
    let mut verified = json!({ "attachments": payload["attachments"] });
    crate::ui_attachments::validate_upload_references(
        &root,
        actor.user_id,
        actor.chat_id,
        client_id,
        &mut verified,
    )?;
    let attachment = verified["attachments"]
        .as_array()
        .filter(|items| items.len() == 1)
        .and_then(|items| items.first())
        .ok_or("aipp_transcript_attachment_invalid")?;
    let kind = transcript_upload_kind(attachment).ok_or("aipp_transcript_attachment_invalid")?;
    crate::ask_flow::transcribe_single_attachment(state, task, attachment, kind == "video").await
}
