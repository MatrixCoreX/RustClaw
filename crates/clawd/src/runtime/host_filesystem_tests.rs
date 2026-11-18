use super::*;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("hf-{}", uuid::Uuid::new_v4().simple()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn directory(&self, relative: &str) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)
            .unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn private_directory_pins_are_keyless_cloexec_and_do_not_create_or_repair_state() {
    let fixture = Fixture::new();
    let private = fixture.directory("private");
    let child = fixture.directory("private/nested");
    let policy =
        HostFilesystemPolicy::pin_directories(&[private.clone(), child], &fixture.0).unwrap();
    assert_eq!(policy.process_directories().unwrap(), vec![private.clone()]);
    let fd = policy.directories[0].descriptor.as_raw_fd();
    assert_ne!(
        unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
    assert_eq!(std::fs::read_dir(&private).unwrap().count(), 1);
    assert_eq!(
        std::fs::read_dir(private.join("nested")).unwrap().count(),
        0
    );

    assert_eq!(
        policy
            .open_file(Some(&fixture.0), &private.join("not-present"))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    let missing = fixture.0.join("missing");
    assert!(HostFilesystemPolicy::pin_directories(&[missing.clone()], &fixture.0).is_err());
    assert!(!missing.exists());
    let link = fixture.0.join("alias");
    symlink(&private, &link).unwrap();
    assert!(HostFilesystemPolicy::pin_directories(&[link], &fixture.0).is_err());
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(HostFilesystemPolicy::pin_directories(&[private.clone()], &fixture.0).is_err());
    assert!(policy.process_directories().is_err());
    assert_eq!(std::fs::metadata(private).unwrap().mode() & 0o7777, 0o755);
}

#[test]
fn moved_private_inode_stays_unreadable_while_public_and_authorized_symlink_reads_work() {
    let fixture = Fixture::new();
    let private = fixture.directory("parent/private");
    std::fs::write(private.join("marker"), b"owned-private-marker").unwrap();
    let public = fixture.0.join("public");
    std::fs::write(&public, b"public-marker").unwrap();
    let policy = HostFilesystemPolicy::pin_directories(&[private.clone()], &fixture.0).unwrap();
    std::fs::rename(fixture.0.join("parent"), fixture.0.join("moved")).unwrap();
    fixture.directory("parent/private");
    std::fs::write(private.join("marker"), b"replacement-marker").unwrap();
    assert!(policy.process_directories().is_err());
    for path in [
        &private.join("marker"),
        &fixture.0.join("moved/private/marker"),
    ] {
        assert_eq!(
            policy
                .open_file(Some(&fixture.0), path)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    let alias = fixture.0.join("private-alias");
    symlink(fixture.0.join("moved/private"), &alias).unwrap();
    assert!(policy.open_file(None, &alias.join("marker")).is_err());
    let public_alias = fixture.0.join("public-alias");
    symlink(&public, &public_alias).unwrap();
    let (mut file, resolved) = policy.open_file(None, &public_alias).unwrap();
    assert_eq!(resolved, public);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"public-marker");
    assert!(policy.open_file(Some(&fixture.0), &public_alias).is_err());
    assert!(policy
        .open_directory(None, &fixture.0.join("moved/private"))
        .is_err());
    assert!(policy.open_directory(None, &alias).is_err());
}

#[test]
fn guarded_workspace_root_cannot_begin_inside_a_private_ancestor() {
    let fixture = Fixture::new();
    let private = fixture.directory("private/nested");
    std::fs::write(private.join("marker"), b"owned-private-marker").unwrap();
    let policy =
        HostFilesystemPolicy::pin_directories(&[fixture.0.join("private")], &fixture.0).unwrap();
    assert_eq!(
        policy
            .open_file(Some(&private), &private.join("marker"))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn returned_read_handle_and_path_survive_later_public_path_replacement() {
    let fixture = Fixture::new();
    let private = fixture.directory("private");
    let policy = HostFilesystemPolicy::pin_directories(&[private], &fixture.0).unwrap();
    let path = fixture.0.join("public");
    std::fs::write(&path, b"opened-public").unwrap();
    let (mut file, resolved) = policy.open_file(Some(&fixture.0), &path).unwrap();
    std::fs::rename(&path, fixture.0.join("original")).unwrap();
    std::fs::write(&path, b"replacement").unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"opened-public");
    assert_eq!(resolved, path);
    assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
}

#[test]
fn public_file_through_execute_only_ancestors_remains_readable() {
    let fixture = Fixture::new();
    let private = fixture.directory("private");
    let parent = fixture.directory("public-parent");
    let path = parent.join("public");
    std::fs::write(&path, b"public-marker").unwrap();
    let policy = HostFilesystemPolicy::pin_directories(&[private], &fixture.0).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o111)).unwrap();
    let ordinary = std::fs::read(&path);
    let guarded = policy.open_file(Some(&fixture.0), &path);
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(ordinary.unwrap(), b"public-marker");
    let (mut file, _) =
        guarded.expect("a readable file needs search, not directory listing rights");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"public-marker");
}

#[test]
fn directory_entries_and_metadata_stay_with_the_opened_directory_after_replacement() {
    use rustix::fs::FileType;
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let private = fixture.directory("private");
    let public = fixture.directory("public");
    std::fs::write(public.join("original"), b"abc").unwrap();
    let raw_name = std::ffi::OsString::from_vec(b"raw-\xff".to_vec());
    std::fs::write(public.join(&raw_name), b"raw-name").unwrap();
    symlink("does-not-exist", public.join("broken-link")).unwrap();
    let policy = HostFilesystemPolicy::pin_directories(&[private], &fixture.0).unwrap();
    let alias = fixture.0.join("public-alias");
    symlink(&public, &alias).unwrap();
    let directory = policy.open_directory(Some(&fixture.0), &alias).unwrap();
    assert_eq!(directory.canonical_path(), public);
    std::fs::rename(&public, fixture.0.join("moved-public")).unwrap();
    fixture.directory("public");
    std::fs::write(public.join("replacement"), b"replacement-only").unwrap();
    for _ in 0..2 {
        let entries = directory.entries().unwrap();
        assert_eq!(entries.len(), 3);
        let original = entries
            .iter()
            .find(|entry| entry.name == "original")
            .unwrap();
        assert_eq!(original.file_type, FileType::RegularFile);
        assert_eq!(original.size_bytes, Some(3));
        assert!(entries.iter().any(|entry| entry.name == raw_name));
        let link = entries
            .iter()
            .find(|entry| entry.name == "broken-link")
            .unwrap();
        assert_eq!(link.file_type, FileType::Symlink);
        assert_eq!(link.size_bytes, None);
        assert!(!entries.iter().any(|entry| entry.name == "replacement"));
    }
}

#[test]
fn directory_listing_traverses_execute_only_parents_but_requires_target_read_access() {
    let fixture = Fixture::new();
    let private = fixture.directory("private");
    let ancestor = fixture.directory("search-only");
    let public = fixture.directory("search-only/public");
    std::fs::write(public.join("marker"), b"public-marker").unwrap();
    let policy = HostFilesystemPolicy::pin_directories(&[private], &fixture.0).unwrap();
    std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o111)).unwrap();
    let readable = policy.open_directory(Some(&fixture.0), &public);
    let denied = policy.open_directory(Some(&fixture.0), &ancestor);
    std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(readable.unwrap().entries().unwrap()[0].name, "marker");
    assert_eq!(
        denied.err().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn authorized_outside_file_refuses_fifo_without_waiting_for_a_writer() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::mpsc;
    use std::time::Duration;
    let fixture = Fixture::new();
    let fifo = fixture.0.join("input.fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: the owned fixture path is a valid C string; no external FIFO is used.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let target = fifo.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = HostFilesystemPolicy::without_private_authority()
            .open_file(None, &target)
            .map(|_| ());
        sender.send(result).unwrap();
    });
    let early = receiver.recv_timeout(Duration::from_secs(1));
    // Release only the owned blocked FIFO reader so a failing test leaves no
    // detached thread behind. This is not a production-task timeout or kill.
    let rescue = early.is_err().then(|| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
            .unwrap()
    });
    let blocked = early.is_err();
    let result = early.unwrap_or_else(|_| receiver.recv().unwrap());
    worker.join().unwrap();
    drop(rescue);
    eprintln!(
        "HOST_FIFO blocked={blocked} refused={} llm_calls=0",
        result.is_err()
    );
    assert!(
        !blocked,
        "opening a file capability must not wait on an owned FIFO"
    );
    assert!(
        result.is_err(),
        "only regular files are admitted by this reader"
    );
}

#[test]
fn private_denial_is_typed_and_cannot_be_inferred_from_displayed_prose() {
    let fixture = Fixture::new();
    let private = fixture.directory("private");
    let policy = HostFilesystemPolicy::pin_directories(&[private.clone()], &fixture.0).unwrap();
    let actual = policy
        .open_file(Some(&fixture.0), &private.join("missing"))
        .err()
        .unwrap();
    assert!(HostFilesystemPolicy::is_private_access_denied(&actual));
    let same_text = io::Error::new(io::ErrorKind::PermissionDenied, actual.to_string());
    assert!(!HostFilesystemPolicy::is_private_access_denied(&same_text));
    assert!(!HostFilesystemPolicy::is_private_access_denied(
        &io::Error::from(io::ErrorKind::PermissionDenied)
    ));
}

#[test]
fn guarded_atomic_write_preserves_public_modes_and_refuses_moved_private_parents() {
    let fixture = Fixture::new();
    let private = fixture.directory("private");
    let policy = HostFilesystemPolicy::pin_directories(&[private.clone()], &fixture.0).unwrap();
    let public = fixture.0.join("public/nested/cache.json");
    policy
        .atomic_write_file(&fixture.0, &public, b"first")
        .unwrap();
    std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o640)).unwrap();
    policy
        .atomic_write_file(&fixture.0, &public, b"second")
        .unwrap();
    assert_eq!(std::fs::read(&public).unwrap(), b"second");
    assert_eq!(std::fs::metadata(&public).unwrap().mode() & 0o7777, 0o640);
    assert!(policy
        .atomic_write_file(&fixture.0, &private.join("cache.json"), b"not-applied")
        .is_err());
    std::fs::rename(&private, fixture.0.join("moved-private")).unwrap();
    assert!(policy
        .atomic_write_file(
            &fixture.0,
            &fixture.0.join("moved-private/new/cache.json"),
            b"not-applied"
        )
        .is_err());
    assert!(std::fs::read_dir(fixture.0.join("moved-private"))
        .unwrap()
        .next()
        .is_none());
    let outside = Fixture::new();
    let victim = outside.0.join("victim");
    std::fs::write(&victim, b"unchanged").unwrap();
    symlink(&victim, fixture.0.join("link")).unwrap();
    assert!(policy
        .atomic_write_file(&fixture.0, &fixture.0.join("link"), b"not-applied")
        .is_err());
    assert_eq!(std::fs::read(&victim).unwrap(), b"unchanged");
}

#[cfg(target_os = "linux")]
#[test]
fn private_directory_descriptor_alias_is_not_a_native_read_escape() {
    let fixture = Fixture::new();
    let private = fixture.directory("private");
    std::fs::write(private.join("marker"), b"owned-private-marker").unwrap();
    let policy = HostFilesystemPolicy::pin_directories(&[private], &fixture.0).unwrap();
    let fd = policy.directories[0].descriptor.as_raw_fd();
    let path = PathBuf::from(format!("/proc/self/fd/{fd}/marker"));
    assert_eq!(
        policy.open_file(None, &path).err().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
}
