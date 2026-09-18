#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "--asset-vault-worker") {
        let code = if agent_desktop::wallet::worker::run().is_ok() { 0 } else { 1 };
        std::process::exit(code);
    }
    agent_desktop::application::run(tauri::generate_context!());
}
