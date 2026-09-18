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

/// Small UI-generated exports use a user-selected document, never a JS path.
#[tauri::command]
pub async fn save_export(window: WebviewWindow, filename: String, bytes: Vec<u8>) -> Result<bool> {
    main_only(&window)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("download_limit".into());
    }
    #[cfg(target_os = "android")]
    {
        let name: String = filename.chars()
            .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
            .take(180).collect();
        let name = if name.trim_matches('.').is_empty() { "export" } else { &name };
        let Some(file) = crate::file_dialog::save("Save file", name).await? else { return Ok(false); };
        tokio::fs::write(file.path(), bytes).await.map_err(|_| "download_write_failed")?;
        let path = file.path().to_str().ok_or("download_path_invalid")?.to_owned();
        tokio::task::spawn_blocking(move || crate::android::bridge::string("finishDocument", &[&path]))
            .await.map_err(|_| "download_write_failed")??;
        Ok(true)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = filename;
        Err("native_command_denied".into())
    }
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
    let selected = crate::file_dialog::save("Save file", name).await?;
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
    outcome?;
    #[cfg(target_os="android")]
    crate::android::bridge::string("finishDocument", &[target.to_str().ok_or("download_path_invalid")?])?;
    Ok(true)
}
