const TRANSCRIPT_FRAME_MAX_BYTES: usize = 64 * 1024;
const TRANSCRIPT_FRAME_TICKET_LIMIT: usize = 64;
const TRANSCRIPT_FRAME_TICKET_TTL: Duration = Duration::from_secs(30);

struct TranscriptFrameScript {
    workspace: PathBuf,
    skill: String,
    package_version: String,
    expires_at: std::time::Instant,
    bytes: Vec<u8>,
}

static TRANSCRIPT_FRAME_SCRIPTS: OnceLock<Mutex<BTreeMap<uuid::Uuid, TranscriptFrameScript>>> =
    OnceLock::new();

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AippTranscriptFrameRequest {
    package_version: String,
}

async fn issue_aipp_transcript_frame(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill): AxumPath<String>,
    Json(request): Json<AippTranscriptFrameRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    tokio::task::spawn_blocking(move || {
        match prepare_aipp_transcript_frame(&state, &headers, &skill, &request.package_version) {
            Ok(frame) => crate::api_ok(frame),
            Err(error) => error,
        }
    })
    .await
    .unwrap_or_else(|_| aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, "aipp_frame_unavailable"))
}

fn prepare_aipp_transcript_frame(
    state: &AppState,
    headers: &HeaderMap,
    skill: &str,
    version: &str,
) -> Result<Value, (StatusCode, Json<ApiResponse<Value>>)> {
    let (_, active) = authorize_aipp_transcripts(state, headers, skill)?;
    if active.manifest.package.version != version {
        return Err(aipp_api_error(StatusCode::CONFLICT, "aipp_package_changed"));
    }
    let unavailable = |code: &str| aipp_api_error(StatusCode::NOT_FOUND, code);
    verify_aipp_bundle_artifacts(state, skill, &active).map_err(|code| unavailable(&code))?;
    let entrypoint = active.aipp.entrypoint.as_deref().unwrap_or_default();
    let script = Path::new(entrypoint).with_file_name("app.js");
    let (html_path, _) =
        resolve_aipp_bundle_asset(&active, entrypoint).map_err(|code| unavailable(&code))?;
    let (script_path, _) = resolve_aipp_bundle_asset(&active, &script.to_string_lossy())
        .map_err(|code| unavailable(&code))?;
    let html = String::from_utf8(read_transcript_frame_asset(&html_path)?)
        .map_err(|_| unavailable("aipp_frame_invalid"))?;
    let bytes = read_transcript_frame_asset(&script_path)?;

    let ticket = uuid::Uuid::new_v4();
    let now = std::time::Instant::now();
    let mut scripts = TRANSCRIPT_FRAME_SCRIPTS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| unavailable("aipp_frame_unavailable"))?;
    scripts.retain(|_, grant| grant.expires_at > now);
    if scripts.len() >= TRANSCRIPT_FRAME_TICKET_LIMIT {
        return Err(aipp_api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "aipp_frame_busy",
        ));
    }
    scripts.insert(
        ticket,
        TranscriptFrameScript {
            workspace: state.skill_rt.workspace_root.clone(),
            skill: skill.to_owned(),
            package_version: version.to_owned(),
            expires_at: now + TRANSCRIPT_FRAME_TICKET_TTL,
            bytes,
        },
    );
    Ok(json!({ "html": html, "script_ticket": ticket }))
}

fn read_transcript_frame_asset(
    path: &Path,
) -> Result<Vec<u8>, (StatusCode, Json<ApiResponse<Value>>)> {
    let invalid = || aipp_api_error(StatusCode::NOT_FOUND, "aipp_frame_invalid");
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| invalid())?
        .take((TRANSCRIPT_FRAME_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.is_empty() || bytes.len() > TRANSCRIPT_FRAME_MAX_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}

fn redeem_aipp_transcript_script(
    state: &AppState,
    skill: &str,
    ticket: uuid::Uuid,
) -> Option<Vec<u8>> {
    let grant = TRANSCRIPT_FRAME_SCRIPTS
        .get()?
        .lock()
        .ok()?
        .remove(&ticket)?;
    if grant.expires_at <= std::time::Instant::now()
        || grant.workspace != state.skill_rt.workspace_root
        || grant.skill != skill
    {
        return None;
    }
    let active = active_aipp_package(state, skill).ok()??;
    if active.aipp.data_contract != TRANSCRIPT_CONTRACT
        || active.manifest.package.version != grant.package_version
        || !aipp_is_installed(state, skill)
    {
        return None;
    }
    Some(grant.bytes)
}

async fn get_aipp_transcript_script(
    State(state): State<AppState>,
    AxumPath((skill, ticket)): AxumPath<(String, uuid::Uuid)>,
) -> axum::response::Response {
    let result =
        tokio::task::spawn_blocking(move || redeem_aipp_transcript_script(&state, &skill, ticket))
            .await;
    let Ok(Some(bytes)) = result else {
        return aipp_api_error(StatusCode::NOT_FOUND, "aipp_frame_unavailable").into_response();
    };
    let mut response = (StatusCode::OK, bytes).into_response();
    apply_aipp_bundle_headers(&mut response, "text/javascript; charset=utf-8");
    response
}
