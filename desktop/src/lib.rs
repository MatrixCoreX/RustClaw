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
