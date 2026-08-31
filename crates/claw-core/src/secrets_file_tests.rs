use std::sync::{Mutex, OnceLock};

use super::*;

fn environment_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn fixture_path() -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-secret-file-test-{}",
        Uuid::new_v4().simple()
    ));
    (root.clone(), root.join("secrets.json"))
}

#[test]
fn file_secret_round_trip_and_delete_never_exposes_through_status() {
    let (root, path) = fixture_path();
    set_file_secret(&path, "github_git_token", "fixture-value").expect("set secret");
    assert!(file_secret_is_configured(&path, "github_git_token").expect("status"));

    let broker = EnvFileSecretsBroker::new(&path);
    assert_eq!(
        broker
            .lookup("github_git_token")
            .expect("lookup")
            .expect("configured")
            .expose(),
        "fixture-value"
    );
    assert!(delete_file_secret(&path, "github_git_token").expect("delete"));
    assert!(!file_secret_is_configured(&path, "github_git_token").expect("status"));
    assert!(broker.lookup("github_git_token").expect("lookup").is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn environment_value_overrides_private_file() {
    let _guard = environment_lock();
    let (root, path) = fixture_path();
    set_file_secret(&path, "github_api_token", "file-value").expect("set secret");
    std::env::set_var("GITHUB_API_TOKEN", "environment-value");
    let broker = EnvFileSecretsBroker::new(&path);
    assert_eq!(
        broker
            .lookup("github_api_token")
            .expect("lookup")
            .expect("configured")
            .expose(),
        "environment-value"
    );
    std::env::remove_var("GITHUB_API_TOKEN");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn systemd_credential_precedes_private_file_and_reports_protection_source() {
    let _guard = environment_lock();
    let (root, path) = fixture_path();
    let credentials = root.join("credentials");
    std::fs::create_dir_all(&credentials).expect("credential directory");
    std::fs::write(credentials.join("github_api_token"), "systemd-value\n")
        .expect("systemd credential");
    set_file_secret(&path, "github_api_token", "file-value").expect("set secret");
    std::env::set_var("CREDENTIALS_DIRECTORY", &credentials);
    let broker = EnvFileSecretsBroker::new(&path);
    let (secret, source) = broker
        .lookup_with_source("github_api_token")
        .expect("lookup")
        .expect("configured");
    assert_eq!(secret.expose(), "systemd-value");
    assert_eq!(source, SecretProtectionSource::SystemdCredential);
    std::env::remove_var("CREDENTIALS_DIRECTORY");
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn systemd_credential_rejects_symlink() {
    use std::os::unix::fs::symlink;

    let _guard = environment_lock();
    let (root, path) = fixture_path();
    let credentials = root.join("credentials");
    std::fs::create_dir_all(&credentials).expect("credential directory");
    std::fs::write(root.join("outside"), "secret").expect("outside secret");
    symlink(root.join("outside"), credentials.join("github_api_token")).expect("symlink");
    std::env::set_var("CREDENTIALS_DIRECTORY", &credentials);
    let broker = EnvFileSecretsBroker::new(&path);
    assert!(broker.lookup("github_api_token").is_err());
    std::env::remove_var("CREDENTIALS_DIRECTORY");
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn secret_file_is_private_and_symlinks_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    let (root, path) = fixture_path();
    set_file_secret(&path, "github_git_token", "fixture-value").expect("set secret");
    assert_eq!(
        std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let link = root.join("linked.json");
    symlink(&path, &link).expect("symlink");
    assert!(set_file_secret(&link, "github_git_token", "other").is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_secret_rotations_preserve_independent_credentials() {
    let _guard = environment_lock();
    let (root, path) = fixture_path();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = [
        ("github_git_token", "git-value"),
        ("github_api_token", "api-value"),
    ]
    .map(|(name, value)| {
        let path = path.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            set_file_secret(&path, name, value).expect("concurrent secret write");
        })
    });
    for handle in handles {
        handle.join().expect("secret writer");
    }
    let broker = EnvFileSecretsBroker::new(&path);
    assert_eq!(
        broker
            .lookup("github_git_token")
            .expect("Git lookup")
            .expect("Git value")
            .expose(),
        "git-value"
    );
    assert_eq!(
        broker
            .lookup("github_api_token")
            .expect("API lookup")
            .expect("API value")
            .expose(),
        "api-value"
    );
    let _ = std::fs::remove_dir_all(root);
}
