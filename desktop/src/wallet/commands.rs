use super::{worker::VaultClient, Account, Status};
use crate::{asset_operations::Operations, commands::main_only, Result};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tauri::{Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct Selection {
    pub account_id: Option<Uuid>,
    pub generation: u64,
}
pub struct WalletState {
    pub vault: Arc<Mutex<VaultClient>>,
    pub operations: tokio::sync::Mutex<Operations>,
    pub selection: Mutex<Selection>,
    pub operation_gate: tokio::sync::Mutex<()>,
    pub native_dialogs: AtomicUsize,
}
impl WalletState {
    pub fn new(directory: PathBuf) -> Result<Self> {
        let vault = VaultClient::new(directory.clone())?;
        Ok(Self {
            vault: Arc::new(Mutex::new(vault)),
            operations: tokio::sync::Mutex::new(Operations::new(
                directory.join("operations-v1.json"),
            )?),
            selection: Mutex::new(Selection {
                account_id: None,
                generation: 0,
            }),
            operation_gate: tokio::sync::Mutex::new(()),
            native_dialogs: AtomicUsize::new(0),
        })
    }
    pub async fn lock(&self) {
        self.vault.lock().unwrap().lock();
        self.selection.lock().unwrap().generation += 1;
        self.operations.lock().await.pending = None;
    }
    pub fn check(&self, id: Uuid, generation: u64) -> Result<()> {
        let selection = self.selection.lock().unwrap();
        if selection.account_id != Some(id) || selection.generation != generation {
            return Err("wallet_selection_changed".into());
        }
        Ok(())
    }
}
pub fn wallet_only(window: &WebviewWindow) -> Result<()> {
    if window.label() != "wallet" {
        return Err("native_command_denied".into());
    }
    Ok(())
}
fn local_only(window: &WebviewWindow) -> Result<()> {
    if !matches!(window.label(), "main" | "wallet") {
        return Err("native_command_denied".into());
    }
    Ok(())
}
pub fn open_window(app: &tauri::AppHandle) -> Result<()> {
    if let Some(window) = app.get_webview_window("wallet") {
        window.show().map_err(|_| "wallet_window_failed")?;
        return window
            .set_focus()
            .map_err(|_| "wallet_window_failed".into());
    }
    WebviewWindowBuilder::new(app, "wallet", WebviewUrl::App("wallet.html".into()))
        .title(format!(
            "{} · 本地资产安全窗口",
            env!("DESKTOP_DISPLAY_NAME")
        ))
        .inner_size(620., 760.)
        .min_inner_size(480., 580.)
        .on_navigation(|url| crate::media::local_page(url, "/wallet.html"))
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build()
        .map_err(|_| "wallet_window_failed")?;
    Ok(())
}
#[tauri::command]
pub fn wallet_open(window: WebviewWindow, app: tauri::AppHandle) -> Result<()> {
    main_only(&window)?;
    open_window(&app)
}
#[tauri::command]
pub async fn wallet_status(window: WebviewWindow, state: State<'_, WalletState>) -> Result<Status> {
    local_only(&window)?;
    let vault = state.vault.clone();
    tokio::task::spawn_blocking(move || vault.lock().unwrap().status())
        .await
        .map_err(|_| "wallet_storage_unavailable")?
}
#[tauri::command]
pub async fn wallet_lock(window: WebviewWindow, state: State<'_, WalletState>) -> Result<()> {
    local_only(&window)?;
    state.lock().await;
    Ok(())
}
#[tauri::command]
pub async fn wallet_select(
    window: WebviewWindow,
    state: State<'_, WalletState>,
    account_id: Option<Uuid>,
) -> Result<()> {
    main_only(&window)?;
    if let Some(id) = account_id {
        state.vault.lock().unwrap().account(id)?;
    }
    {
        let mut selection = state.selection.lock().unwrap();
        selection.account_id = account_id;
        selection.generation += 1;
    }
    state.operations.lock().await.pending = None;
    Ok(())
}
#[tauri::command]
pub async fn wallet_initialize(
    window: WebviewWindow,
    state: State<'_, WalletState>,
    password: String,
) -> Result<()> {
    let password = Zeroizing::new(password);
    wallet_only(&window)?;
    let vault = state.vault.clone();
    tokio::task::spawn_blocking(move || vault.lock().unwrap().initialize(&password))
        .await
        .map_err(|_| "wallet_storage_unavailable")?
}
#[tauri::command]
pub async fn wallet_unlock(
    window: WebviewWindow,
    state: State<'_, WalletState>,
    password: String,
) -> Result<()> {
    let password = Zeroizing::new(password);
    wallet_only(&window)?;
    let vault = state.vault.clone();
    tokio::task::spawn_blocking(move || vault.lock().unwrap().unlock(&password))
        .await
        .map_err(|_| "wallet_storage_unavailable")?
}
#[tauri::command]
pub async fn wallet_create(
    window: WebviewWindow,
    state: State<'_, WalletState>,
    name: String,
) -> Result<Account> {
    wallet_only(&window)?;
    let vault = state.vault.clone();
    tokio::task::spawn_blocking(move || vault.lock().unwrap().create(&name))
        .await
        .map_err(|_| "wallet_storage_unavailable")?
}
#[tauri::command]
pub async fn wallet_backup(
    window: WebviewWindow,
    state: State<'_, WalletState>,
    account_id: Uuid,
    password: String,
    vault_password: String,
) -> Result<bool> {
    let password = Zeroizing::new(password);
    let vault_password = Zeroizing::new(vault_password);
    wallet_only(&window)?;
    let public = state.vault.lock().unwrap().account(account_id)?.public_key;
    let guard = DialogGuard::new(&state.native_dialogs);
    let file = rfd::AsyncFileDialog::new()
        .set_title("保存加密资产备份")
        .set_file_name(format!("asset-account-{}.backup.json", &public[..8]))
        .add_filter("加密账户备份", &["json"])
        .save_file()
        .await;
    drop(guard);
    let Some(file) = file else { return Ok(false) };
    let vault = state.vault.clone();
    tokio::task::spawn_blocking(move || {
        vault
            .lock()
            .unwrap()
            .backup(account_id, &vault_password, &password, file.path())
    })
    .await
    .map_err(|_| "wallet_storage_unavailable")??;
    Ok(true)
}
#[tauri::command]
pub async fn wallet_restore(
    window: WebviewWindow,
    state: State<'_, WalletState>,
    password: String,
    name: String,
) -> Result<Option<Account>> {
    let password = Zeroizing::new(password);
    wallet_only(&window)?;
    let guard = DialogGuard::new(&state.native_dialogs);
    let file = rfd::AsyncFileDialog::new()
        .set_title("恢复加密资产备份")
        .add_filter("加密账户备份", &["json"])
        .pick_file()
        .await;
    drop(guard);
    let Some(file) = file else { return Ok(None) };
    let vault = state.vault.clone();
    let account = tokio::task::spawn_blocking(move || {
        vault.lock().unwrap().restore(&password, file.path(), &name)
    })
    .await
    .map_err(|_| "wallet_storage_unavailable")??;
    Ok(Some(account))
}
struct DialogGuard<'a>(&'a AtomicUsize);
impl<'a> DialogGuard<'a> {
    fn new(value: &'a AtomicUsize) -> Self {
        value.fetch_add(1, Ordering::SeqCst);
        Self(value)
    }
}
impl Drop for DialogGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
