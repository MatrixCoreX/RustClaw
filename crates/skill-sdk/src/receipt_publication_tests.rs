use super::atomic_write;
use std::fs;
use std::path::Path;

fn entries(directory: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

#[cfg(unix)]
fn replace_and_check_mode(mode: u32) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    fs::write(&target, b"old pointer").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(mode)).unwrap();
    let before = fs::symlink_metadata(&target).unwrap();
    atomic_write(&target, b"new pointer").unwrap();
    let after = fs::symlink_metadata(&target).unwrap();
    assert_eq!(
        (after.uid(), after.gid(), after.mode() & 0o7777),
        (before.uid(), before.gid(), before.mode() & 0o7777),
        "replacement must not broaden or lose an existing file's metadata"
    );
    assert_eq!(fs::read(&target).unwrap(), b"new pointer");
    assert_eq!(
        entries(root.path()),
        vec![target.file_name().unwrap().to_os_string()]
    );
}

#[cfg(unix)]
#[test]
fn sdk_publication_preserves_private_existing_mode() {
    replace_and_check_mode(0o600);
}

#[cfg(unix)]
#[test]
fn sdk_publication_preserves_read_only_existing_mode() {
    replace_and_check_mode(0o400);
}

#[cfg(unix)]
#[test]
fn sdk_publication_preserves_executable_existing_mode() {
    replace_and_check_mode(0o740);
}

#[cfg(unix)]
#[test]
fn sdk_publication_preserves_existing_owner_and_group() {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::os::unix::process::CommandExt;
    const CHILD: &str = "APP_SDK_OWNER_FAILURE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        assert_ne!(unsafe { libc::geteuid() }, 0);
        let target = Path::new("foreign.json");
        let before = fs::metadata(target).unwrap();
        assert_eq!(before.uid(), 0);
        let original_entries = entries(Path::new("."));
        let error = atomic_write(target, b"must not publish").unwrap_err();
        assert_eq!(error.code, "atomic_write_metadata_failed");
        let after = fs::metadata(target).unwrap();
        assert_eq!(
            (after.uid(), after.gid(), after.mode()),
            (before.uid(), before.gid(), before.mode())
        );
        assert_eq!(fs::read(target).unwrap(), b"root-owned original");
        assert_eq!(entries(Path::new(".")), original_entries);
        eprintln!("sdk_publication_unpreservable_owner_rejected=true");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    fs::write(&target, b"old pointer").unwrap();
    // The same exact fixture also runs under native CI sudo; never run Cargo as root.
    let privileged = unsafe { libc::geteuid() } == 0;
    if privileged {
        let file = fs::File::open(&target).unwrap();
        assert_eq!(unsafe { libc::fchown(file.as_raw_fd(), 65534, 65534) }, 0);
    }
    let before = fs::metadata(&target).unwrap();
    atomic_write(&target, b"new pointer").unwrap();
    let after = fs::metadata(&target).unwrap();
    assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
    assert_eq!(fs::read(&target).unwrap(), b"new pointer");
    if privileged {
        let foreign = root.path().join("foreign.json");
        fs::write(&foreign, b"root-owned original").unwrap();
        fs::set_permissions(&foreign, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "receipt::publication_tests::sdk_publication_preserves_existing_owner_and_group",
                "--exact",
                "--nocapture",
            ])
            .current_dir(root.path())
            .env(CHILD, "1")
            .uid(65534)
            .gid(65534)
            .output()
            .unwrap();
        eprintln!(
            "sdk_publication_child_status={} stdout={:?} stderr={:?}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("sdk_publication_unpreservable_owner_rejected=true"));
    }
    eprintln!("sdk_publication_metadata_preserved=true privileged_owner_check={privileged} unpreservable_owner_rejected={privileged}");
}

#[cfg(unix)]
#[test]
fn sdk_publication_rejects_leaf_links_without_modifying_either_target() {
    use std::os::unix::fs::symlink;
    for dangling in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = outside.path().join("outside.json");
        if !dangling {
            fs::write(&source, b"outside protected").unwrap();
        }
        let target = root.path().join("current.json");
        symlink(&source, &target).unwrap();
        assert!(atomic_write(&target, b"replacement").is_err());
        assert_eq!(fs::read_link(&target).unwrap(), source);
        assert_eq!(source.exists(), !dangling);
        if !dangling {
            assert_eq!(fs::read(&source).unwrap(), b"outside protected");
        }
        assert_eq!(
            entries(root.path()),
            vec![target.file_name().unwrap().to_os_string()]
        );
    }
}

#[cfg(unix)]
#[test]
fn sdk_publication_rejects_sockets_without_replacing_them() {
    use std::os::unix::fs::FileTypeExt;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    let _listener = std::os::unix::net::UnixListener::bind(&target).unwrap();
    assert!(atomic_write(&target, b"replacement").is_err());
    assert!(fs::symlink_metadata(&target)
        .unwrap()
        .file_type()
        .is_socket());
    assert_eq!(
        entries(root.path()),
        vec![target.file_name().unwrap().to_os_string()]
    );
}

#[test]
fn sdk_publication_directory_failure_keeps_original_and_cleans_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("current.json");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), b"protected").unwrap();
    assert!(atomic_write(&target, b"replacement").is_err());
    assert_eq!(fs::read(target.join("keep")).unwrap(), b"protected");
    assert_eq!(
        entries(root.path()),
        vec![target.file_name().unwrap().to_os_string()]
    );
}

#[test]
fn sdk_publication_preserves_new_file_creation_policy_and_complete_bytes() {
    let root = tempfile::tempdir().unwrap();
    let reference = root.path().join("reference");
    fs::write(&reference, b"creation mode reference").unwrap();
    let target = root.path().join("nested/current.json");
    let bytes = vec![b'x'; 2 * 1024 * 1024 + 17];
    atomic_write(&target, &bytes).unwrap();
    assert_eq!(fs::read(&target).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            fs::metadata(&target).unwrap().mode() & 0o7777,
            fs::metadata(&reference).unwrap().mode() & 0o7777
        );
    }
    assert_eq!(
        entries(target.parent().unwrap()),
        vec![target.file_name().unwrap().to_os_string()]
    );
}

#[path = "receipt_publication_failure_tests.rs"]
mod failures;
