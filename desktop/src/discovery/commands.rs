use super::{discover, DiscoveryReport};
use crate::{commands::main_only, Result};
use std::sync::Mutex;
use tauri::{State, WebviewWindow};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct DiscoveryState(Mutex<Option<CancellationToken>>);

#[tauri::command]
pub async fn discover_devices(
    window: WebviewWindow,
    state: State<'_, DiscoveryState>,
    scan_subnet: Option<bool>,
) -> Result<DiscoveryReport> {
    main_only(&window)?;
    let cancel = CancellationToken::new();
    {
        let mut running = state.0.lock().map_err(|_| "discovery_unavailable")?;
        if running.is_some() {
            return Err("discovery_running".into());
        }
        *running = Some(cancel.clone());
    }
    let report = discover(scan_subnet.unwrap_or(false), cancel).await;
    *state.0.lock().map_err(|_| "discovery_unavailable")? = None;
    Ok(report)
}

#[tauri::command]
pub fn cancel_discovery(window: WebviewWindow, state: State<'_, DiscoveryState>) -> Result<()> {
    main_only(&window)?;
    if let Some(cancel) = state
        .0
        .lock()
        .map_err(|_| "discovery_unavailable")?
        .as_ref()
    {
        cancel.cancel();
    }
    Ok(())
}
