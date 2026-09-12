use super::commands::WalletState;
use std::time::{Duration, Instant, SystemTime};
use tauri::Manager;

pub fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut previous = Instant::now();
        let mut previous_wall = SystemTime::now();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let gap = previous.elapsed();
            previous = Instant::now();
            // Linux's monotonic clock can exclude suspend time. Wall-clock gaps
            // (including a clock rollback) also revoke the unlocked session.
            let wall_gap = previous_wall.elapsed().unwrap_or(Duration::MAX);
            previous_wall = SystemTime::now();
            let interrupted = gap > Duration::from_secs(5) || wall_gap > Duration::from_secs(5);
            let locked = system_locked().await;
            let state = app.state::<WalletState>();
            // Password derivation runs on a blocking worker. Polling must not hold
            // the async runtime or mistake that deliberate work for suspension.
            let expired = match state.vault.try_lock() {
                Ok(mut vault) => vault.expire(),
                Err(_) if !locked && !interrupted => continue,
                Err(_) => false,
            };
            if interrupted || locked || expired {
                state.lock().await;
            }
        }
    });
}

#[cfg(target_os = "linux")]
async fn system_locked() -> bool {
    // A missing system service does not replace the additional app-focus lock.
    tokio::time::timeout(Duration::from_millis(700), async {
        let connection = zbus::Connection::system().await.ok()?;
        let manager = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await
        .ok()?;
        let path: zbus::zvariant::OwnedObjectPath = manager
            .call("GetSessionByPID", &(std::process::id(),))
            .await
            .ok()?;
        let session = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            path,
            "org.freedesktop.login1.Session",
        )
        .await
        .ok()?;
        session.get_property::<bool>("LockedHint").await.ok()
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(false)
}
#[cfg(target_os = "windows")]
#[path = "session_windows.rs"]
mod native_session;
#[cfg(target_os = "macos")]
#[path = "session_macos.rs"]
mod native_session;
#[cfg(any(target_os = "windows", target_os = "macos"))]
async fn system_locked() -> bool {
    native_session::locked()
}

pub fn focus_changed(app: tauri::AppHandle, focused: bool) {
    if focused {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        if app
            .state::<WalletState>()
            .native_dialogs
            .load(std::sync::atomic::Ordering::SeqCst)
            > 0
        {
            return;
        }
        if !["main", "wallet"].iter().any(|name| {
            app.get_webview_window(name)
                .is_some_and(|w| w.is_focused().unwrap_or(false))
        }) {
            app.state::<WalletState>().lock().await;
        }
    });
}
