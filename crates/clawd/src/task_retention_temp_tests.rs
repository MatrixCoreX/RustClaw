use super::*;
use crate::task_result_retention::test_support::Fixture;
use crate::task_result_retention::{files, storage};

fn allowed() -> Vec<String> {
    TEMP_VARIABLES.into_iter().map(str::to_string).collect()
}
fn enrolled() -> (Fixture, SkillStorageDescriptor) {
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    let normal = fixture
        .state
        .core
        .skill_storage
        .directory_descriptor("fixture", 1)
        .unwrap();
    let descriptor = storage::descriptor_for_task(
        &fixture.state,
        &fixture.task_id,
        &fixture.payload(),
        Some(normal),
    )
    .unwrap()
    .unwrap();
    (fixture, descriptor)
}
fn values(command: &tokio::process::Command) -> BTreeMap<OsString, Option<OsString>> {
    command
        .as_std()
        .get_envs()
        .map(|(k, v)| (k.to_os_string(), v.map(std::ffi::OsStr::to_os_string)))
        .collect()
}

#[test]
fn ordinary_and_forged_payloads_keep_every_environment_entry_unchanged() {
    for kind in ["ask", "run_skill"] {
        let fixture = Fixture::new();
        fixture.insert(&fixture.task_id, kind, "ordinary", &fixture.payload());
        let descriptor = fixture
            .state
            .core
            .skill_storage
            .directory_descriptor("fixture", 1)
            .unwrap();
        let plan = prepare(
            &fixture.state,
            &fixture.task_id,
            &fixture.payload(),
            Some(&descriptor),
            &[],
            &BTreeMap::from([("TMPDIR".into(), "package default".into())]),
        )
        .unwrap();
        assert!(plan.is_none());
        let mut command = tokio::process::Command::new("unused");
        command
            .env("TMPDIR", "original value /tmp/with space")
            .env("TMP", "")
            .env("TEMP", "unchanged λ")
            .env("OTHER", "retained")
            .env_remove("REMOVED");
        let before = values(&command);
        apply(
            &mut command,
            plan.as_ref(),
            Some(Path::new("../irrelevant")),
        )
        .unwrap();
        assert_eq!(before, values(&command));
        assert!(
            !claw_core::workspace_state::workspace_artifacts_root(&fixture.root)
                .join("task-storage")
                .exists()
        );
    }
}

#[test]
fn owned_temporaries_use_linux_mapped_or_macos_canonical_storage_paths() {
    let (fixture, descriptor) = enrolled();
    let plan = prepare(
        &fixture.state,
        &fixture.task_id,
        &fixture.payload(),
        Some(&descriptor),
        &allowed(),
        &BTreeMap::new(),
    )
    .unwrap()
    .unwrap();
    assert!(plan.host_directory.is_dir());
    for mapped in [
        PathBuf::from("/run/agent-runtime-writable/3"),
        PathBuf::from(descriptor.directory_path.as_ref().unwrap())
            .canonicalize()
            .unwrap(),
    ] {
        let mut command = tokio::process::Command::new("unused");
        command
            .env("TMPDIR", "global old tmp")
            .env("OTHER", "unchanged");
        apply(&mut command, Some(&plan), Some(&mapped)).unwrap();
        let env = values(&command);
        for key in TEMP_VARIABLES {
            assert_eq!(
                env[std::ffi::OsStr::new(key)],
                Some(mapped.join(TEMP_DIRECTORY).into_os_string())
            );
        }
        assert_eq!(
            env[std::ffi::OsStr::new("OTHER")],
            Some(OsString::from("unchanged"))
        );
    }
    assert!(plan.mapped_values(None).is_err());
    assert!(plan.mapped_values(Some(Path::new("../tmp"))).is_err());
    assert!(plan.mapped_values(Some(Path::new("/run/../tmp"))).is_err());
}

#[test]
fn missing_receipt_forwarding_or_fixed_package_temp_fails_before_child_launch() {
    let (fixture, descriptor) = enrolled();
    let missing = vec!["TMPDIR".into(), "TEMP".into()];
    assert!(prepare(
        &fixture.state,
        &fixture.task_id,
        &fixture.payload(),
        Some(&descriptor),
        &missing,
        &BTreeMap::new()
    )
    .is_err());
    let fixed = BTreeMap::from([("TMPDIR".to_string(), "/global/package/tmp".to_string())]);
    assert!(prepare(
        &fixture.state,
        &fixture.task_id,
        &fixture.payload(),
        Some(&descriptor),
        &allowed(),
        &fixed
    )
    .is_err());
    assert!(!Path::new(descriptor.directory_path.as_ref().unwrap())
        .join(TEMP_DIRECTORY)
        .exists());
}

#[test]
fn interrupted_child_netlog_and_temp_cache_are_removed_with_the_owned_task() {
    let (fixture, descriptor) = enrolled();
    let plan = prepare(
        &fixture.state,
        &fixture.task_id,
        &fixture.payload(),
        Some(&descriptor),
        &allowed(),
        &BTreeMap::new(),
    )
    .unwrap()
    .unwrap();
    let chrome = plan.host_directory.join("browser-cancelled");
    std::fs::create_dir_all(chrome.join("profile/Cache")).unwrap();
    std::fs::write(chrome.join("netlog.json"), b"sensitive network evidence").unwrap();
    std::fs::write(chrome.join("profile/Cache/video"), b"media bytes").unwrap();
    let shared = fixture.root.join("ordinary-temp-cache");
    std::fs::write(&shared, b"ordinary cache stays").unwrap();
    let cleanup = files::prepare(&fixture.root, &fixture.task_id).unwrap();
    files::execute(&fixture.root, &cleanup).unwrap();
    assert!(!plan.host_directory.exists());
    assert_eq!(std::fs::read(shared).unwrap(), b"ordinary cache stays");
}

#[cfg(unix)]
#[test]
fn temporary_child_symlink_never_changes_shared_content_or_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let (fixture, descriptor) = enrolled();
    let shared = fixture.root.join("shared-temp");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o750)).unwrap();
    std::fs::write(shared.join("keep"), b"ordinary temp").unwrap();
    let before = std::fs::metadata(&shared).unwrap().permissions().mode();
    let temporary = Path::new(descriptor.directory_path.as_ref().unwrap()).join(TEMP_DIRECTORY);
    symlink(&shared, &temporary).unwrap();
    assert!(prepare(
        &fixture.state,
        &fixture.task_id,
        &fixture.payload(),
        Some(&descriptor),
        &allowed(),
        &BTreeMap::new()
    )
    .is_err());
    assert_eq!(
        std::fs::metadata(&shared).unwrap().permissions().mode(),
        before
    );
    assert_eq!(
        std::fs::read(shared.join("keep")).unwrap(),
        b"ordinary temp"
    );
}
