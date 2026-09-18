#[cfg(feature = "gui")]
pub mod aipp;
pub mod asset_operations;
#[cfg(feature = "gui")]
pub mod commands;
pub mod credentials;
pub mod discovery;
pub mod download_stream;
#[cfg(feature = "gui")]
pub mod downloads;
#[cfg(feature = "gui")]
pub mod media;
pub mod media_relay;
pub mod profile;
pub mod session;
pub mod transfers;
pub mod transport;
pub mod wallet;
pub mod webview_origin;

pub type Result<T> = std::result::Result<T, String>;
pub const CHUNK_BYTES: usize = 64 * 1024;

#[cfg(feature = "gui")]
pub mod application;
#[cfg(target_os = "android")]
pub mod android;
#[cfg(all(target_os = "android", feature = "gui"))]
#[tauri::mobile_entry_point]
fn run() {
    application::run(tauri::generate_context!());
}

#[cfg(feature="gui")]
pub mod file_dialog;
