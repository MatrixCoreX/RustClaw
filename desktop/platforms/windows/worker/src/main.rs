//! Dedicated signer: no Tauri/WebView/User32 initialization before hardening.
fn main() {
    if std::env::args_os().nth(1).is_none_or(|arg| arg != "--asset-vault-worker") {
        std::process::exit(2);
    }
    std::process::exit(if agent_desktop::wallet::worker::run().is_ok() { 0 } else { 1 });
}
