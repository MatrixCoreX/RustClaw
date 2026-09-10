use crate::{
    credentials::{self, LoginSecret},
    profile::{Connection, Profile, ProfileStore},
    session::{LoginResult, Session, SessionInfo},
    transfers::{RequestSpec, ResponseHead, Transfers},
    Result,
};
use bytes::Bytes;
use std::{collections::HashMap, sync::Arc};
use tauri::{
    ipc::{InvokeBody, Request, Response},
    State, WebviewWindow,
};
use tokio::sync::Mutex;
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct DesktopState {
    pub profiles: Mutex<ProfileStore>,
    pub session: Mutex<Option<Arc<Session>>>,
    pub transfers: Transfers,
    pub aipps: Mutex<HashMap<String, crate::aipp::AippScope>>,
    pub aipp_limits: Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>,
    pub downloads: Mutex<HashMap<(Uuid, Uuid), tokio_util::sync::CancellationToken>>,
    // Serialize connect/login/switch operations; late connections cannot become active.
    pub transition: Mutex<()>,
    pub media: Arc<crate::media_relay::MediaRelay>,
}
impl DesktopState {
    pub async fn session(&self, id: Uuid) -> Result<Arc<Session>> {
        self.session
            .lock()
            .await
            .as_ref()
            .filter(|s| s.id == id && !s.cancelled.is_cancelled())
            .cloned()
            .ok_or("stale_connection".into())
    }
}
pub fn main_only(window: &WebviewWindow) -> Result<()> {
    if window.label() != "main" {
        return Err("native_command_denied".into());
    }
    Ok(())
}
#[tauri::command]
pub async fn media_open(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    path: String,
) -> Result<String> {
    main_only(&window)?;
    state
        .media
        .grant(state.session(session_id).await?, path)
        .await
}
#[tauri::command]
pub async fn media_close(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    url: String,
) -> Result<()> {
    main_only(&window)?;
    state.media.revoke(&url).await;
    Ok(())
}
#[tauri::command]
pub async fn profiles(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Vec<Profile>> {
    main_only(&window)?;
    state.profiles.lock().await.list()
}
#[tauri::command]
pub async fn add_profile(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    alias: String,
    connection: Connection,
) -> Result<Profile> {
    main_only(&window)?;
    state.profiles.lock().await.add(alias, connection)
}
#[tauri::command]
pub async fn forget_profile(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    profile_id: Uuid,
) -> Result<()> {
    main_only(&window)?;
    let _transition = state.transition.lock().await;
    if state
        .session
        .lock()
        .await
        .as_ref()
        .is_some_and(|s| s.profile.id == profile_id)
    {
        return Err("disconnect_before_forget".into());
    }
    if state.profiles.lock().await.get(profile_id)?.saved_login {
        tokio::task::spawn_blocking(move || credentials::forget(profile_id))
            .await
            .map_err(|_| "credential_store_unavailable")??;
    }
    state.profiles.lock().await.forget(profile_id)
}
#[tauri::command]
pub async fn connect_device(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    profile_id: Uuid,
    ssh_secret: String,
    ssh_key: Option<String>,
    ssh_key_passphrase: Option<String>,
) -> Result<SessionInfo> {
    main_only(&window)?;
    let _transition = state.transition.lock().await;
    if state.session.lock().await.is_some() {
        return Err("disconnect_before_connect".into());
    }
    let profile = state.profiles.lock().await.get(profile_id)?;
    let session = Session::connect_with_key(
        profile,
        Zeroizing::new(ssh_secret),
        ssh_key.map(Zeroizing::new),
        ssh_key_passphrase.map(Zeroizing::new),
    )
    .await?;
    let info = session.info().await;
    *state.session.lock().await = Some(session);
    Ok(info)
}
#[tauri::command]
pub async fn current_session(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<SessionInfo>> {
    main_only(&window)?;
    let session = state.session.lock().await.clone();
    match session {
        Some(s) => Ok(Some(s.info().await)),
        None => Ok(None),
    }
}
#[tauri::command]
pub async fn login(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    input: Option<LoginSecret>,
    remember: bool,
) -> Result<LoginResult> {
    main_only(&window)?;
    let _transition = state.transition.lock().await;
    let session = state.session(session_id).await?;
    let input = if let Some(input) = input {
        input
    } else {
        let id = session.profile.id;
        tokio::task::spawn_blocking(move || credentials::load(id))
            .await
            .map_err(|_| "credential_store_unavailable")??
    };
    // Persist the reference before saving, so a successful vault write can never become orphaned.
    if remember {
        state
            .profiles
            .lock()
            .await
            .mark_saved_login(session.profile.id)?;
    }
    session.login(input, remember).await
}
#[tauri::command]
pub async fn disconnect_device(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<()> {
    main_only(&window)?;
    let _transition = state.transition.lock().await;
    let session = state.session.lock().await.take();
    if let Some(session) = session {
        state.transfers.cancel_session(session.id).await;
        session.close().await;
    }
    use tauri::Manager;
    for label in state.aipps.lock().await.drain().map(|(label, _)| label) {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
    }
    state.aipp_limits.lock().await.clear();
    Ok(())
}
#[tauri::command]
pub async fn request_start(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    spec: RequestSpec,
) -> Result<Uuid> {
    main_only(&window)?;
    state
        .transfers
        .start(state.session(session_id).await?, spec)
        .await
}
#[tauri::command]
pub async fn request_headers(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    id: Uuid,
) -> Result<ResponseHead> {
    main_only(&window)?;
    state.session(session_id).await?;
    state.transfers.headers(session_id, id).await
}
#[tauri::command]
pub async fn request_read(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    id: Uuid,
) -> Result<Response> {
    main_only(&window)?;
    state.session(session_id).await?;
    Ok(Response::new(
        state.transfers.read(session_id, id).await?.to_vec(),
    ))
}
#[tauri::command]
pub async fn request_cancel(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    id: Uuid,
) -> Result<()> {
    main_only(&window)?;
    state.transfers.cancel(session_id, id).await;
    Ok(())
}
#[tauri::command]
pub async fn upload_chunk(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: Request<'_>,
) -> Result<()> {
    main_only(&window)?;
    let header_uuid = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| Uuid::parse_str(v).ok())
            .ok_or("transfer_id_invalid")
    };
    let session_id = header_uuid("x-session-id")?;
    let id = header_uuid("x-transfer-id")?;
    state.session(session_id).await?;
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("binary_body_required".into());
    };
    state
        .transfers
        .upload(session_id, id, Bytes::copy_from_slice(bytes))
        .await
}
#[tauri::command]
pub async fn upload_finish(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    id: Uuid,
) -> Result<()> {
    main_only(&window)?;
    state.session(session_id).await?;
    state.transfers.finish_upload(session_id, id).await
}
#[tauri::command]
pub async fn open_external(window: WebviewWindow, url: String) -> Result<()> {
    main_only(&window)?;
    let parsed = reqwest::Url::parse(&url).map_err(|_| "external_url_denied")?;
    if !matches!(parsed.scheme(), "https" | "http" | "mailto")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("external_url_denied".into());
    }
    open::that_detached(url).map_err(|_| "external_open_failed".into())
}
