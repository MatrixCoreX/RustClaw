#![allow(dead_code)]
pub type Result<T> = std::result::Result<T, String>;
mod protocol { pub const MAX_FRAME: usize = 128 * 1024; }
#[path = "../../../src/wallet/secure_memory.rs"]
pub mod secure_memory;
#[cfg(windows)]
#[path = "../../../src/wallet/windows_security.rs"]
pub mod windows_security;
#[path = "../../../src/wallet/worker/sandbox.rs"]
pub mod sandbox;
#[path = "../../../src/wallet/worker/platform.rs"]
pub mod platform;
#[cfg(windows)]
#[path = "../../../src/wallet/worker/windows_job.rs"]
pub mod windows_job;
#[cfg(windows)]
#[path = "../../../src/wallet/worker/windows_pipe.rs"]
pub mod windows_pipe;
#[path = "../../../src/wallet/worker/transport.rs"]
pub mod transport;
pub mod wallet { #[cfg(windows)] pub use crate::windows_security; }

#[cfg(windows)]
#[path="../../../src/wallet/session_windows.rs"]
mod session_windows;
#[cfg(target_os="macos")]
#[path="../../../src/wallet/session_macos.rs"]
mod session_macos;

#[path="../../../src/wallet/files.rs"]
mod files;

#[cfg(all(test, windows))]
#[path="../../../src/wallet/windows_tests.rs"]
mod windows_tests;
pub mod worker {
    pub use crate::transport;
    #[cfg(windows)] pub use crate::windows_pipe;
}

#[cfg(windows)]
#[path="../../../src/wallet/worker/peer_memory_windows.rs"]
mod peer_memory_windows;
