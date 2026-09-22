use super::*;
use crate::secrets::{EnvFileSecretsBroker, SecretsBroker};

const VENDORS: [&str; 9] = [
    "openai",
    "google",
    "anthropic",
    "grok",
    "deepseek",
    "qwen",
    "minimax",
    "mimo",
    "custom",
];

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("model-env-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn model_environment_roundtrip_all_vendors_and_restart_loader() {
    let root = fixture();
    for vendor in VENDORS {
        save(&root, vendor, &format!("fixture-{vendor}-key")).unwrap();
    }
    let broker_path = crate::git_remote_config::git_credential_store_path(&root);
    crate::secrets::set_file_secret(&broker_path, "text_minimax_api_key", "old-json-key").unwrap();
    save(&root, "minimax", "rotated-key").unwrap();
    let broker = EnvFileSecretsBroker::new(&broker_path);
    assert_eq!(
        broker
            .lookup("text_minimax_api_key")
            .unwrap()
            .unwrap()
            .expose(),
        "rotated-key"
    );
    for vendor in VENDORS {
        let expected = if vendor == "minimax" {
            "rotated-key".into()
        } else {
            format!("fixture-{vendor}-key")
        };
        assert_eq!(
            lookup_at(&path(&root), vendor).unwrap().unwrap().expose(),
            expected
        );
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/model_environment.sh");
        let output = std::process::Command::new("bash")
            .args([
                "-euc",
                "source \"$1\"; load_managed_model_environment \"$2\"; printenv \"$3\"",
                "test",
            ])
            .arg(script)
            .arg(&root)
            .arg(environment_name(vendor).unwrap())
            .env("MINIMAX_API_KEY", "old-inherited-key")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path(&root)).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path(&root).parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn model_environment_special_characters_are_literal_not_shell() {
    let root = fixture();
    let marker = root.join("must-not-exist");
    let key = format!(
        "$(touch {})`touch {}`;'\"\\= $HOME #",
        marker.display(),
        marker.display()
    );
    save(&root, "minimax", &key).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/model_environment.sh");
    let output = std::process::Command::new("bash")
        .args(["-euc", "source \"$1\"; load_managed_model_environment \"$2\"; printf '%s' \"$MINIMAX_API_KEY\"", "test"])
        .arg(script).arg(&root).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), key);
    assert!(!marker.exists());
    assert_eq!(
        lookup_at(&path(&root), "minimax")
            .unwrap()
            .unwrap()
            .expose(),
        key
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn model_environment_invalid_updates_preserve_existing_file() {
    let root = fixture();
    save(&root, "minimax", "fixture-valid").unwrap();
    let before = fs::read(path(&root)).unwrap();
    for bad in [
        "",
        "x\ny",
        "x\ry",
        "x\0y",
        "x\ty",
        "REPLACE_ME",
        &"x".repeat(4097),
    ] {
        assert!(save(&root, "minimax", bad).is_err());
        assert_eq!(fs::read(path(&root)).unwrap(), before);
    }
    assert!(save(&root, "LD_PRELOAD", "fixture").is_err());
    fs::write(
        path(&root),
        "MINIMAX_API_KEY=first\nMINIMAX_API_KEY=second\n",
    )
    .unwrap();
    assert!(lookup_at(&path(&root), "minimax").is_err());
    fs::write(path(&root), "LD_PRELOAD=malicious\n").unwrap();
    assert!(lookup_at(&path(&root), "minimax").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn model_environment_rejects_symlink_file_and_directory() {
    use std::os::unix::fs::symlink;
    let root = fixture();
    let outside = fixture();
    let target = outside.join("secret");
    fs::write(&target, "untouched").unwrap();
    let path = path(&root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink(&target, &path).unwrap();
    assert!(save(&root, "minimax", "fixture-key").is_err());
    assert!(lookup_at(&path, "minimax").is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
    fs::remove_file(&path).unwrap();
    fs::remove_dir(path.parent().unwrap()).unwrap();
    symlink(&outside, path.parent().unwrap()).unwrap();
    assert!(save(&root, "minimax", "fixture-key").is_err());
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn model_environment_concurrent_vendor_updates_do_not_lose_keys() {
    let root = fixture();
    std::thread::scope(|scope| {
        for vendor in VENDORS {
            let root = &root;
            scope.spawn(move || save(root, vendor, "fixture-key").unwrap());
        }
    });
    assert_eq!(read(&path(&root)).unwrap().len(), VENDORS.len());
    fs::remove_dir_all(root).unwrap();
}
