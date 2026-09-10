#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use agent_desktop::{
    aipp, commands::*, discovery, downloads, media, profile::ProfileStore, transfers::Transfers,
};
use std::collections::HashMap;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::Mutex;

fn main() {
    tauri::Builder::default()
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
        ])
        .run(tauri::generate_context!())
        .expect("desktop_runtime_failed");
}
