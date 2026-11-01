use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::super::host::{
    AuthorityDirectories, AuthorityHost, AuthorityHostPhase, AuthorityHostReason,
};
use super::super::protocol::{AuthorityAdmissionCode as Code, AuthorityResponse};
use super::super::{BOOTSTRAP_READY, HELPER_FLAG};
use crate::local_process_owner::launch_channel::LaunchChannel;
use crate::local_process_owner::owner_reference::AuthorityFingerprint;

struct Fixture {
    root: PathBuf,
    keys: PathBuf,
    journal: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ab-{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let keys = root.join("keys");
        let journal = root.join("journal");
        for path in [&keys, &journal] {
            fs::DirBuilder::new().mode(0o700).create(path).unwrap();
        }
        Self {
            root,
            keys,
            journal,
        }
    }
    fn key(&self) -> PathBuf {
        self.keys
            .join(crate::local_process_owner::owner_authority::DIRECTORY)
            .join("authority.key")
    }
    fn database(&self) -> PathBuf {
        self.journal.join("authority.sqlite")
    }
    fn sequence(&self) -> i64 {
        rusqlite::Connection::open_with_flags(
            self.database(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
        .query_row(
            "SELECT admission_sequence FROM local_process_authority_bootstrap",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

struct OwnedHelper(Child);
impl OwnedHelper {
    fn wait(&mut self) -> std::process::ExitStatus {
        let end = deadline();
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < end, "owned authority helper did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for OwnedHelper {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn initialize() -> serde_json::Value {
    serde_json::json!({"schema_version":1,"action":{"kind":"initialize"}})
}
fn resume(pin: [u8; 32]) -> serde_json::Value {
    serde_json::json!({"schema_version":1,"action":{"kind":"resume","authority_fingerprint":pin}})
}

fn helper(fixture: &Fixture) -> (OwnedHelper, LaunchChannel) {
    let program = PathBuf::from(
        std::env::var_os("APP_OWNER_ENTRY_TEST_BINARY").expect("source-pinned executable required"),
    );
    assert!(program.is_absolute() && fs::symlink_metadata(&program).unwrap().is_file());
    let (host, peer) = UnixStream::pair().unwrap();
    let fd = unsafe { libc::fcntl(peer.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 6) };
    assert!(fd >= 6);
    let descriptor = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut command = Command::new(program);
    command
        .arg(HELPER_FLAG)
        .env_clear()
        .current_dir(&fixture.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(
                    fixture
                        .root
                        .join(format!("stderr-{}", uuid::Uuid::new_v4().simple())),
                )
                .unwrap(),
        );
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(descriptor.as_raw_fd(), 3) != 3 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = OwnedHelper(command.spawn().unwrap());
    drop(command);
    drop(peer);
    let host = LaunchChannel::new(host).unwrap();
    assert_eq!(host.receive_control(deadline()).unwrap(), BOOTSTRAP_READY);
    assert_eq!(
        File::open(format!("/proc/{}/mem", child.0.id()))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    (child, host)
}

fn invoke_with(
    fixture: &Fixture,
    request: serde_json::Value,
    keys: File,
    journal: File,
) -> AuthorityResponse {
    let (mut child, host) = helper(fixture);
    let host = host
        .send_bootstrap(
            &serde_json::to_vec(&request).unwrap(),
            [keys.into(), journal.into()],
            deadline(),
        )
        .unwrap();
    let bytes = host.receive_control(deadline()).unwrap();
    drop(host);
    let status = child.wait();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schema_version"], 1);
    let response: AuthorityResponse = serde_json::from_value(value).unwrap();
    assert_eq!(status.success(), response.is_success());
    response
}
fn invoke(fixture: &Fixture, request: serde_json::Value) -> AuthorityResponse {
    invoke_with(
        fixture,
        request,
        File::open(&fixture.keys).unwrap(),
        File::open(&fixture.journal).unwrap(),
    )
}
fn pin(response: AuthorityResponse) -> [u8; 32] {
    match response {
        AuthorityResponse::Ok {
            authority_fingerprint,
            ..
        } => authority_fingerprint,
        AuthorityResponse::Error { error, .. } => panic!("authority fixture refused: {error:?}"),
        AuthorityResponse::Reference { .. }
        | AuthorityResponse::Outcome { .. }
        | AuthorityResponse::Resources { .. } => {
            panic!("unexpected bound job response")
        }
    }
}
fn error(response: AuthorityResponse, code: Code) {
    match response {
        AuthorityResponse::Error { error, .. } => assert_eq!(error.error_code, code),
        AuthorityResponse::Ok { .. }
        | AuthorityResponse::Reference { .. }
        | AuthorityResponse::Outcome { .. }
        | AuthorityResponse::Resources { .. } => {
            panic!("authority fixture unexpectedly admitted")
        }
    }
}
fn record(case: &str) {
    println!("AUTHORITY_ENTRY_CASE case={case} actual_executable=true llm_calls=0");
}

#[path = "local_process_authority_publication_tests.rs"]
mod publication;

#[path = "local_process_authority_outcome_process_tests.rs"]
mod outcome;

fn host() -> AuthorityHost {
    let executable =
        std::env::var_os("APP_OWNER_ENTRY_TEST_BINARY").expect("source-pinned executable required");
    AuthorityHost::from_test_executable(File::open(executable).unwrap()).unwrap()
}

fn directories(fixture: &Fixture) -> AuthorityDirectories {
    // Fresh isolated fixture without managed payloads. This is not ordinary
    // host private-root or pre-cutover survivor admission evidence.
    unsafe {
        AuthorityDirectories::from_fenced(
            File::open(&fixture.keys).unwrap(),
            File::open(&fixture.journal).unwrap(),
        )
        .unwrap()
    }
}

fn host_cases() {
    {
        let fixture = Fixture::new();
        let host = host();
        let expected = host.initialize(directories(&fixture), deadline()).unwrap();
        let inode = fs::metadata(fixture.key()).unwrap().ino();
        assert_eq!(fixture.sequence(), 1);
        assert_eq!(
            host.resume(directories(&fixture), expected, deadline())
                .unwrap(),
            expected
        );
        assert_eq!(fixture.sequence(), 2);
        record("host_initialize_and_reopen");
        let mut wrong = *expected.as_bytes();
        wrong[0] ^= 1;
        let refused = host
            .resume(
                directories(&fixture),
                AuthorityFingerprint::from_slice(&wrong).unwrap(),
                deadline(),
            )
            .unwrap_err();
        assert!(refused.request_may_have_been_sent());
        assert!(
            matches!(refused.reason, AuthorityHostReason::Admission(error)
            if error.error_code == Code::AuthorityMismatch)
        );
        assert_eq!(fixture.sequence(), 2);
        let repeated = host
            .initialize(directories(&fixture), deadline())
            .unwrap_err();
        assert!(
            matches!(repeated.reason, AuthorityHostReason::Admission(error)
            if error.error_code == Code::AlreadyInitialized)
        );
        assert_eq!(fs::metadata(fixture.key()).unwrap().ino(), inode);
        assert_eq!(fixture.sequence(), 2);
        record("host_wrong_pin_and_duplicate_no_reinitialize");
        let lock = rusqlite::Connection::open(fixture.database()).unwrap();
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        let uncertain = host
            .resume(
                directories(&fixture),
                expected,
                Instant::now() + Duration::from_secs(2),
            )
            .unwrap_err();
        assert_eq!(uncertain.phase, AuthorityHostPhase::Outcome);
        assert!(uncertain.request_may_have_been_sent());
        assert!(matches!(
            uncertain.reason,
            AuthorityHostReason::Transport(_)
        ));
        lock.execute_batch("ROLLBACK").unwrap();
        drop(lock);
        assert_eq!(fixture.sequence(), 2);
        assert_eq!(fs::metadata(fixture.key()).unwrap().ino(), inode);
        assert_eq!(
            host.resume(directories(&fixture), expected, deadline())
                .unwrap(),
            expected
        );
        assert_eq!(fixture.sequence(), 3);
        record("host_uncertain_deadline_no_automatic_retry");
    }
    {
        let fixture = Fixture::new();
        let refused = host()
            .resume(
                directories(&fixture),
                AuthorityFingerprint::from_slice(&[1; 32]).unwrap(),
                deadline(),
            )
            .unwrap_err();
        assert!(refused.request_may_have_been_sent());
        assert!(
            matches!(refused.reason, AuthorityHostReason::Admission(error)
            if error.error_code == Code::MissingJournal)
        );
        assert!(!fixture.database().exists());
        assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
        record("host_missing_journal_no_initialize_fallback");
    }
    {
        let fixture = Fixture::new();
        let source = std::env::var_os("APP_OWNER_ENTRY_TEST_BINARY").unwrap();
        let path = fixture.root.join("pinned-helper");
        fs::copy(source, &path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let pinned = AuthorityHost::from_test_executable(File::open(&path).unwrap()).unwrap();
        fs::rename(&path, fixture.root.join("original-image")).unwrap();
        fs::write(&path, b"not the admitted executable").unwrap();
        pinned
            .initialize(directories(&fixture), deadline())
            .unwrap();
        assert_eq!(fixture.sequence(), 1);
        assert_eq!(fs::read(&path).unwrap(), b"not the admitted executable");
        record("host_pinned_executable_survives_path_replacement");
    }
}

pub(super) fn run() {
    {
        let fixture = Fixture::new();
        let (mut observer, peer) = UnixStream::pair().unwrap();
        let descriptor = unsafe { libc::fcntl(peer.as_raw_fd(), libc::F_DUPFD, 64) };
        assert!(descriptor >= 64);
        let inherited = unsafe { OwnedFd::from_raw_fd(descriptor) };
        assert_eq!(
            unsafe { libc::fcntl(inherited.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        drop(peer);
        let (mut child, channel) = helper(&fixture);
        observer
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let control = observer.read(&mut [0]);
        assert!(matches!(
            control.as_ref().err().map(std::io::Error::kind),
            Some(std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
        ));
        drop(inherited);
        let isolated = observer.read(&mut [0]);
        // No private directory has crossed the channel. Reap the owned helper
        // before asserting the observation, including on a regression.
        drop(channel);
        assert!(!child.wait().success());
        assert_eq!(
            isolated.unwrap(),
            0,
            "unrelated inherited descriptor survived protected-ready"
        );
        assert!(!fixture.database().exists());
        record("unrelated_inherited_descriptor_closed_before_ready");
    }
    host_cases();
    publication::run();
    outcome::run();
    {
        let fixture = Fixture::new();
        let expected = pin(invoke(&fixture, initialize()));
        let inode = fs::metadata(fixture.key()).unwrap().ino();
        assert_eq!(fixture.sequence(), 1);
        assert_eq!(pin(invoke(&fixture, resume(expected))), expected);
        assert_eq!(fixture.sequence(), 2);
        assert_eq!(fs::metadata(fixture.key()).unwrap().ino(), inode);
        record("initialize_and_reopen");
        let mut wrong = expected;
        wrong[0] ^= 1;
        error(invoke(&fixture, resume(wrong)), Code::AuthorityMismatch);
        assert_eq!(fixture.sequence(), 2);
        error(invoke(&fixture, initialize()), Code::AlreadyInitialized);
        assert_eq!(fixture.sequence(), 2);
        assert_eq!(fs::metadata(fixture.key()).unwrap().ino(), inode);
        record("wrong_pin_and_duplicate_initialize");
        fs::remove_file(fixture.key()).unwrap();
        error(
            invoke(&fixture, resume(expected)),
            Code::AuthorityUnavailable,
        );
        assert!(!fixture.key().exists());
        record("missing_key_no_reinitialize");
    }
    {
        let fixture = Fixture::new();
        error(invoke(&fixture, resume([1; 32])), Code::MissingJournal);
        assert!(!fixture.database().exists());
        assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
        record("missing_journal_no_create");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(fixture.database())
            .unwrap();
        error(invoke(&fixture, resume([1; 32])), Code::InvalidJournal);
        error(invoke(&fixture, initialize()), Code::AlreadyInitialized);
        assert_eq!(fs::metadata(fixture.database()).unwrap().len(), 0);
        assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
        record("partial_first_create_retained");
    }
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let fixture = Fixture::new();
        let public = fixture.root.join("public-marker");
        fs::write(&public, b"unchanged").unwrap();
        fs::set_permissions(&public, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(
            &public,
            fixture.journal.join(format!("authority.sqlite{suffix}")),
        )
        .unwrap();
        error(invoke(&fixture, initialize()), Code::InvalidJournal);
        assert_eq!(fs::read(public).unwrap(), b"unchanged");
        assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
        record(&format!("journal_symlink_{suffix}"));
    }
    {
        let fixture = Fixture::new();
        let public = fixture.root.join("public-marker");
        fs::write(&public, b"unchanged").unwrap();
        fs::set_permissions(&public, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&public, fixture.database()).unwrap();
        error(invoke(&fixture, initialize()), Code::InvalidJournal);
        assert_eq!(fs::read(public).unwrap(), b"unchanged");
        record("journal_hardlink_refused");
    }
    {
        let fixture = Fixture::new();
        fs::set_permissions(&fixture.journal, fs::Permissions::from_mode(0o755)).unwrap();
        error(invoke(&fixture, initialize()), Code::InvalidDirectory);
        assert!(!fixture.database().exists());
        assert_eq!(
            fs::metadata(&fixture.journal).unwrap().mode() & 0o777,
            0o755
        );
        record("nonprivate_directory_not_repaired");
    }
    {
        let fixture = Fixture::new();
        error(
            invoke(
                &fixture,
                serde_json::json!({"schema_version":1,"job_id":"wrong_role"}),
            ),
            Code::InvalidRequest,
        );
        error(
            invoke(
                &fixture,
                serde_json::json!({"schema_version":2,"action":{"kind":"initialize"}}),
            ),
            Code::InvalidRequest,
        );
        assert!(!fixture.database().exists());
        record("invalid_and_cross_role_metadata");
    }
    {
        let fixture = Fixture::new();
        let journal = File::open(&fixture.journal).unwrap();
        let moved = fixture.root.join("admitted-journal");
        fs::rename(&fixture.journal, &moved).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&fixture.journal)
            .unwrap();
        pin(invoke_with(
            &fixture,
            initialize(),
            File::open(&fixture.keys).unwrap(),
            journal,
        ));
        assert!(moved.join("authority.sqlite").is_file());
        assert!(!fixture.database().exists());
        record("journal_descriptor_survives_path_replacement");
    }
    {
        let fixture = Fixture::new();
        let expected = pin(invoke(&fixture, initialize()));
        let inode = fs::metadata(fixture.key()).unwrap().ino();
        fs::write(fixture.key(), [0; 40]).unwrap();
        error(
            invoke(&fixture, resume(expected)),
            Code::AuthorityUnavailable,
        );
        assert_eq!(fs::metadata(fixture.key()).unwrap().ino(), inode);
        assert_eq!(fs::metadata(fixture.key()).unwrap().len(), 40);
        record("corrupt_key_not_replaced");
    }
    {
        let fixture = Fixture::new();
        let (mut child, host) = helper(&fixture);
        let host = host
            .send_bootstrap(
                &serde_json::to_vec(&initialize()).unwrap(),
                [
                    File::open(&fixture.keys).unwrap().into(),
                    File::open(&fixture.journal).unwrap().into(),
                ],
                deadline(),
            )
            .unwrap();
        drop(host);
        assert!(!child.wait().success());
        assert_eq!(fixture.sequence(), 1);
        assert_eq!(fs::metadata(fixture.key()).unwrap().len(), 40);
        // No automatic initialize retry, pin adoption, or business execution.
        record("lost_response_keeps_committed_state");
    }
}
