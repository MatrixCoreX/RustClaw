use super::*;
use crate::secrets::{EnvFileSecretsBroker, SecretsBroker};

struct OwnedFixture(PathBuf);
impl Drop for OwnedFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn model_environment_panicked_publication_restores_credentials_and_unlocks_next_save() {
    let fixture = OwnedFixture(fixture());
    save(&fixture.0, "custom", "fixture-original").unwrap();
    let original = fs::read(path(&fixture.0)).unwrap();
    let result = std::panic::catch_unwind(|| {
        with_saved(
            &fixture.0,
            "custom",
            "fixture-replacement",
            || -> Result<(), ()> {
                panic!("fixture-only publication panic");
            },
        )
    });
    assert!(result.is_err());
    assert_eq!(fs::read(path(&fixture.0)).unwrap(), original);
    save(&fixture.0, "custom", "fixture-after-panic").unwrap();
    assert_eq!(
        lookup_at(&path(&fixture.0), "custom")
            .unwrap()
            .unwrap()
            .expose(),
        "fixture-after-panic"
    );
}

#[test]
fn model_environment_failed_associated_publication_restores_exact_original_bytes() {
    let fixture = OwnedFixture(fixture());
    save(&fixture.0, "custom", "fixture-original").unwrap();
    let original = fs::read(path(&fixture.0)).unwrap();
    let result = with_saved(&fixture.0, "custom", "fixture-replacement", || {
        Err::<(), _>("fixture config failure")
    });
    assert!(matches!(
        result,
        Err(PublicationError::Operation("fixture config failure"))
    ));
    assert_eq!(fs::read(path(&fixture.0)).unwrap(), original);
}

#[test]
fn model_environment_failed_first_publication_removes_only_its_own_new_file() {
    let fixture = OwnedFixture(fixture());
    let result = with_saved(&fixture.0, "custom", "fixture-replacement", || {
        Err::<(), _>(())
    });
    assert!(matches!(result, Err(PublicationError::Operation(()))));
    assert!(!path(&fixture.0).exists());
}

#[test]
fn model_environment_recovery_does_not_clobber_independent_credentials() {
    let fixture = OwnedFixture(fixture());
    let independent = b"CUSTOM_API_KEY=fixture-independent\n";
    let result = with_saved(&fixture.0, "custom", "fixture-replacement", || {
        fs::write(path(&fixture.0), independent).unwrap();
        Err::<(), _>(())
    });
    assert!(matches!(result, Err(PublicationError::Recovery(_))));
    assert_eq!(fs::read(path(&fixture.0)).unwrap(), independent);
}

#[test]
fn model_environment_success_keeps_credentials_and_callback_result() {
    let fixture = OwnedFixture(fixture());
    assert_eq!(
        with_saved(&fixture.0, "custom", "fixture-replacement", || Ok::<_, ()>(
            42
        ))
        .unwrap(),
        42
    );
    assert_eq!(
        lookup_at(&path(&fixture.0), "custom")
            .unwrap()
            .unwrap()
            .expose(),
        "fixture-replacement"
    );
}

#[test]
fn model_environment_invalid_credential_never_runs_associated_publication() {
    let fixture = OwnedFixture(fixture());
    let mut called = false;
    let result = with_saved(&fixture.0, "custom", "invalid\nvalue", || {
        called = true;
        Ok::<_, ()>(())
    });
    assert!(matches!(result, Err(PublicationError::Environment(_))));
    assert!(!called);
    assert!(!path(&fixture.0).exists());
}

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
