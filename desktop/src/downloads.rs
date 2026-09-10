use crate::{
    commands::{main_only, DesktopState},
    profile::api_path,
    Result,
};
use tauri::{State, WebviewWindow};
use uuid::Uuid;

#[derive(Clone, serde::Serialize)]
pub struct DownloadProgress {
    pub id: Uuid,
    pub written: u64,
    pub total: Option<u64>,
    pub finished: bool,
}

#[tauri::command]
pub async fn download_cancel(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    id: Uuid,
) -> Result<()> {
    main_only(&window)?;
    if let Some(token) = state.downloads.lock().await.get(&(session_id, id)) {
        token.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn download(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    path: String,
    filename: String,
    id: Option<Uuid>,
    progress: tauri::ipc::Channel<DownloadProgress>,
) -> Result<bool> {
    main_only(&window)?;
    api_path(&path)?;
    let session = state.session(session_id).await?;
    let name: String = filename
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
        .take(180)
        .collect();
    let name = if name.trim_matches('.').is_empty() {
        "download"
    } else {
        &name
    };
    let selected = rfd::AsyncFileDialog::new()
        .set_title("Save to this computer")
        .set_file_name(name)
        .save_file()
        .await;
    let Some(selected) = selected else {
        return Ok(false);
    };
    state.session(session_id).await?;
    let id = id.unwrap_or_else(Uuid::new_v4);
    let cancel = session.cancelled.child_token();
    {
        let mut downloads = state.downloads.lock().await;
        if downloads.len() >= 4 || downloads.contains_key(&(session_id, id)) {
            return Err("download_limit".into());
        }
        downloads.insert((session_id, id), cancel.clone());
    }
    let target = selected.path().to_owned();
    let outcome = crate::download_stream::save_response(
        &session,
        &path,
        &target,
        &cancel,
        |written, total, finished| {
            let _ = progress.send(DownloadProgress {
                id,
                written,
                total,
                finished,
            });
        },
    )
    .await;
    state.downloads.lock().await.remove(&(session_id, id));
    outcome.map(|_| true)
}
