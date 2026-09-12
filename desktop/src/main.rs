#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use agent_desktop::{
    aipp, asset_operations, commands::*, discovery, downloads, media, profile::ProfileStore,
    transfers::Transfers, wallet,
};
use std::collections::HashMap;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::Mutex;

fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--asset-vault-worker")
    {
        let code = if wallet::worker::run().is_ok() { 0 } else { 1 };
        std::process::exit(code);
    }
    tauri::Builder::default()
        .on_window_event(|window, event| {
            if window.label() == "wallet"
                && matches!(
                    event,
                    tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
                )
            {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    app.state::<wallet::commands::WalletState>().lock().await;
                });
            }
            if matches!(window.label(), "main" | "wallet") {
                if let tauri::WindowEvent::Focused(focused) = event {
                    wallet::lifecycle::focus_changed(window.app_handle().clone(), *focused);
                }
            }
        })
        .register_asynchronous_uri_scheme_protocol("device", |context, request, responder| {
            let app = context.app_handle().clone();
            let label = context.webview_label().to_owned();
            tauri::async_runtime::spawn(async move {
                let response = media::serve(app, label, request)
                    .await
                    .unwrap_or_else(|error| {
                        #[cfg(debug_assertions)]
                        eprintln!("desktop_media_error {error}");
                        #[cfg(not(debug_assertions))]
                        let _ = error;
                        http::Response::builder()
                            .status(403)
                            .header("cache-control", "no-store")
                            .body(Vec::new())
                            .unwrap()
                    });
                responder.respond(response);
            });
        })
        .setup(|app| {
            app.manage(discovery::DiscoveryState::default());
            let directory = app.path().app_data_dir()?;
            app.manage(
                wallet::commands::WalletState::new(directory.join("asset-wallet"))
                    .map_err(std::io::Error::other)?,
            );
            app.manage(
                asset_operations::standalone::StandaloneState::new(
                    directory.join("asset-wallet/nodes-v1.json"),
                )
                .map_err(std::io::Error::other)?,
            );
            wallet::lifecycle::start(app.handle().clone());
            app.manage(DesktopState {
                profiles: Mutex::new(ProfileStore::new(directory).map_err(std::io::Error::other)?),
                session: Mutex::new(None),
                transfers: Transfers::default(),
                aipps: Mutex::new(HashMap::new()),
                aipp_limits: Mutex::new(HashMap::new()),
                transition: Mutex::new(()),
                downloads: Mutex::new(HashMap::new()),
                media: tauri::async_runtime::block_on(
                    agent_desktop::media_relay::MediaRelay::start(),
                )
                .map_err(std::io::Error::other)?,
            });
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(env!("DESKTOP_DISPLAY_NAME"))
                .inner_size(1280.0, 860.0)
                .min_inner_size(860.0, 600.0)
                .on_page_load(|_, payload| {
                    #[cfg(debug_assertions)]
                    eprintln!("desktop_page_load {:?} {}", payload.event(), payload.url());
                    #[cfg(not(debug_assertions))]
                    let _ = payload;
                })
                .on_navigation(|url| media::local_page(url, "/index.html"))
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .on_download(|_, event| {
                    if let tauri::webview::DownloadEvent::Requested { url, destination } = event {
                        if url.scheme() != "blob" {
                            return false;
                        }
                        let name = destination
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("export");
                        if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
                            *destination = path;
                            return true;
                        }
                        return false;
                    }
                    true
                })
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            profiles,
            media_open,
            media_close,
            add_profile,
            forget_profile,
            connect_device,
            current_session,
            login,
            login_prefill,
            disconnect_device,
            request_start,
            request_headers,
            request_read,
            request_cancel,
            upload_chunk,
            upload_finish,
            downloads::download,
            downloads::download_cancel,
            open_external,
            aipp::aipp_open,
            aipp::aipp_context,
            aipp::aipp_bridge,
            discovery::commands::discover_devices,
            discovery::commands::cancel_discovery,
            wallet::commands::wallet_open,
            wallet::commands::wallet_status,
            wallet::commands::wallet_lock,
            wallet::commands::wallet_select,
            wallet::commands::wallet_initialize,
            wallet::commands::wallet_unlock,
            wallet::commands::wallet_create,
            wallet::commands::wallet_backup,
            wallet::commands::wallet_restore,
            asset_operations::standalone::wallet_nodes,
            asset_operations::standalone::wallet_add_node,
            asset_operations::standalone::wallet_connect_node,
            asset_operations::standalone::wallet_prefer_node,
            asset_operations::standalone::wallet_disconnect_node,
            asset_operations::standalone::wallet_market_read,
            asset_operations::commands::wallet_pending,
            asset_operations::commands::wallet_cancel_operation,
            asset_operations::commands::wallet_capabilities,
            asset_operations::commands::wallet_read,
            asset_operations::commands::wallet_prepare,
            asset_operations::commands::wallet_confirm,
            asset_operations::commands::wallet_operations,
            asset_operations::commands::wallet_check_operation,
        ])
        .run(tauri::generate_context!())
        .expect("desktop_runtime_failed");
}
