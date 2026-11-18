use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use super::*;
use crate::local_process_owner::owner_admission::OwnerAuthorityJournal;
use crate::local_process_owner::owner_host_handshake::{CommitObservation, HostPendingStart};
use crate::local_process_owner::owner_outcome::DurableOwnerOutcome;
use crate::local_process_owner::owner_protocol::{
    OwnerAction, OwnerCompletion, OwnerCredential, OwnerOutcome, OwnerRequest, OwnerSnapshot,
};
use crate::local_process_owner::owner_store::{DurableOwnerReference, OwnerReferenceStore};
use crate::local_process_owner::process_launch::{PayloadIo, ProcessLaunch};
use crate::process_invocation::ProcessInvocation;

const AUTHORITY_DIRECTORY: &str = crate::local_process_owner::owner_authority::DIRECTORY;

struct Fixture {
    root: PathBuf,
    private: PathBuf,
    workspace: PathBuf,
    store: OwnerReferenceStore,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("oe-{}", Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let private = root.join("private");
        let workspace = root.join("work");
        for path in [&private, &workspace] {
            fs::DirBuilder::new().mode(0o700).create(path).unwrap();
        }
        let db = rusqlite::Connection::open(private.join("journal.sqlite")).unwrap();
        db.pragma_update(None, "journal_mode", "WAL").unwrap();
        let mut journal = OwnerAuthorityJournal::from_connection(db).unwrap();
        let broker =
            OwnerAuthorityBroker::from_admitted_directory(File::open(&private).unwrap()).unwrap();
        let store = journal.initialize(&broker).unwrap();
        Self {
            root,
            private,
            workspace,
            store,
        }
    }

    fn bootstrap(&self) -> OwnerBootstrap {
        OwnerBootstrap {
            schema_version: 1,
            job_id: Uuid::new_v4(),
            authority_fingerprint: *self.store.authority_fingerprint().unwrap().as_bytes(),
            binding_digest: [17; 32],
            terminate_grace_millis: 50,
        }
    }

    fn resume_store(&self) -> OwnerReferenceStore {
        let db = rusqlite::Connection::open(self.private.join("journal.sqlite")).unwrap();
        let mut journal = OwnerAuthorityJournal::from_connection(db).unwrap();
        let broker =
            OwnerAuthorityBroker::from_admitted_directory(File::open(&self.private).unwrap())
                .unwrap();
        journal
            .resume(&broker, &self.store.authority_fingerprint().unwrap())
            .unwrap()
    }

    fn controls(&self) -> UnixDatagram {
        let path = self.private.join("owner.sock");
        let socket = UnixDatagram::bind(&path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        socket
    }

    fn launch(&self, script: &str) -> ProcessLaunch {
        let mut invocation = ProcessInvocation::new("/bin/sh");
        invocation.arg("-c").arg(script);
        self.launch_invocation(invocation)
    }

    fn launch_invocation(&self, invocation: ProcessInvocation) -> ProcessLaunch {
        ProcessLaunch::new(
            invocation,
            self.workspace.clone(),
            vec![("PATH".into(), "/usr/bin:/bin".into())],
            PayloadIo {
                stdin: None,
                stdout: Some(
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(self.workspace.join("stdout"))
                        .unwrap(),
                ),
                stderr: None,
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::set_permissions(
            self.private.join(AUTHORITY_DIRECTORY),
            fs::Permissions::from_mode(0o700),
        );
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(crate) struct OwnedHelper(pub(crate) Child);
impl OwnedHelper {
    pub(crate) fn wait(&mut self) -> std::process::ExitStatus {
        let end = deadline();
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < end, "owned helper did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for OwnedHelper {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            // The fixture owns this unreaped immediate child. Its payload is
            // independently self-terminating; never signal a numeric group.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub(crate) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

fn helper(fixture: &Fixture) -> (OwnedHelper, LaunchChannel) {
    helper_at(&fixture.workspace)
}

pub(crate) fn helper_at(workspace: &std::path::Path) -> (OwnedHelper, LaunchChannel) {
    let (host, peer) = UnixStream::pair().unwrap();
    let descriptor = unsafe { libc::fcntl(peer.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 6) };
    assert!(descriptor >= 6);
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    let mut command = Command::new(executable());
    command
        .arg(HELPER_FLAG)
        .env_clear()
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(workspace.join("helper.stderr"))
                .unwrap(),
        );
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(descriptor.as_raw_fd(), BOOTSTRAP_FD) != BOOTSTRAP_FD {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = OwnedHelper(command.spawn().unwrap());
    drop(command);
    drop(peer);
    let host = LaunchChannel::new(host).unwrap();
    let ready = host.receive_control(deadline()).unwrap_or_else(|error| {
        panic!(
            "protected ready failed: {error:?}; {}",
            fs::read_to_string(workspace.join("helper.stderr")).unwrap()
        );
    });
    assert_eq!(ready, BOOTSTRAP_READY);
    #[cfg(target_os = "linux")]
    assert_eq!(
        File::open(format!("/proc/{}/mem", child.0.id()))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    (child, host)
}

fn wait_effect(fixture: &Fixture) {
    let end = deadline();
    while !fixture.workspace.join("effects").exists() {
        assert!(Instant::now() < end, "payload did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_terminal(
    fixture: &Fixture,
    reference: &DurableOwnerReference,
) -> super::super::super::owner_protocol::OwnerSnapshot {
    let end = deadline();
    loop {
        if let Some(terminal) = fixture.store.load_terminal(reference).unwrap() {
            assert!(terminal.authorizes(&reference.credential(), &[17; 32]));
            return terminal.snapshot();
        }
        assert!(
            Instant::now() < end,
            "authenticated terminal record missing"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct ControlClient {
    socket: UnixDatagram,
    endpoint: PathBuf,
    next_request: u64,
}

impl ControlClient {
    fn new(fixture: &Fixture) -> Self {
        let path = fixture.private.join("client.sock");
        let socket = UnixDatagram::bind(&path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        Self {
            socket,
            endpoint: fixture.private.join("owner.sock"),
            next_request: 1,
        }
    }

    fn request(&mut self, credential: &OwnerCredential, action: OwnerAction) -> OwnerSnapshot {
        let request = OwnerRequest {
            schema_version: 1,
            credential: credential.clone(),
            request_id: self.next_request,
            action,
        };
        self.socket
            .send_to(&serde_json::to_vec(&request).unwrap(), &self.endpoint)
            .unwrap();
        let mut bytes = vec![0; 4096];
        let (length, sender) = self.socket.recv_from(&mut bytes).unwrap();
        assert_eq!(sender.as_pathname(), Some(self.endpoint.as_path()));
        let reply =
            crate::local_process_owner::owner_protocol::decode_reply(&bytes[..length]).unwrap();
        assert_eq!(reply.request_id, self.next_request);
        self.next_request += 1;
        match reply.outcome {
            OwnerOutcome::Ok { snapshot } => snapshot,
            OwnerOutcome::Error { error_code } => panic!("owned control failed: {error_code:?}"),
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_case(case: &str) {
    let fixture = Fixture::new();
    let bootstrap = fixture.bootstrap();
    let (mut child, host) = helper(&fixture);
    let owner = crate::local_process_owner::instance::observe_process(child.0.id())
        .unwrap()
        .birth;
    let credential = fixture
        .store
        .credential_for(bootstrap.job_id, owner, bootstrap.binding_digest)
        .unwrap();
    let mut bytes = serde_json::to_value(&bootstrap).unwrap();
    if case == "wrong_fingerprint" {
        bytes["authority_fingerprint"] = serde_json::to_value([3_u8; 32]).unwrap();
    }
    if case == "invalid_bootstrap" {
        bytes["schema_version"] = serde_json::json!(99);
    }
    let control: OwnedFd = if case == "invalid_control" {
        File::open(&fixture.workspace).unwrap().into()
    } else {
        fixture.controls().into()
    };
    let host = host
        .send_bootstrap(
            &serde_json::to_vec(&bytes).unwrap(),
            [File::open(&fixture.private).unwrap().into(), control],
            deadline(),
        )
        .unwrap();
    if matches!(
        case,
        "wrong_fingerprint" | "invalid_bootstrap" | "invalid_control"
    ) {
        drop(host);
        assert!(!child.wait().success());
        assert!(!fixture.workspace.join("effects").exists());
        assert!(fixture.store.load(bootstrap.job_id).unwrap().is_none());
        return;
    }
    let script = if case == "cancel" {
        "printf x >> effects; sleep 2"
    } else {
        "for fd in /proc/self/fd/*; do case ${fd##*/} in 0|1|2) continue;; esac; if [ -S \"$fd\" ] || [ -d \"$fd\" ]; then exit 92; fi; done; printf x >> effects; sleep 0.25; exit 7"
    };
    let launch = if matches!(
        case,
        "attempt_failed" | "lost_failure_ack" | "failure_retry"
    ) {
        fixture.launch_invocation(ProcessInvocation::new(
            fixture.workspace.join("absent-program"),
        ))
    } else {
        fixture.launch(script)
    };
    let reference = fixture
        .store
        .publish(&credential, bootstrap.binding_digest)
        .unwrap();
    let host = host
        .send_package_lease(None, &credential, deadline())
        .unwrap()
        .send(launch, credential.clone(), deadline())
        .unwrap();
    let ready = HostPendingStart::new(host, credential.clone(), bootstrap.binding_digest)
        .unwrap()
        .wait_ready(deadline())
        .unwrap();
    assert!(!fixture.workspace.join("effects").exists());
    if case == "no_commit" {
        drop(ready);
        assert!(!child.wait().success());
        assert!(!fixture.workspace.join("effects").exists());
        assert!(matches!(fixture.store.load_outcome(&reference).unwrap(),
            Some(DurableOwnerOutcome::NotCommitted(proof))
                if proof.authorizes(&credential, &bootstrap.binding_digest)));
        return;
    }
    let outcome_path = fixture
        .private
        .join(AUTHORITY_DIRECTORY)
        .join(format!("{}.outcome", bootstrap.job_id.simple()));
    if case == "failure_retry" {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&outcome_path)
            .unwrap()
            .write_all(b"fixture-owned refusal")
            .unwrap();
    }
    let committed = ready.commit(&reference, deadline()).unwrap();
    if matches!(
        case,
        "attempt_failed" | "lost_failure_ack" | "failure_retry"
    ) {
        if case == "lost_failure_ack" {
            drop(committed);
        } else {
            assert_eq!(
                committed.wait_started(deadline()),
                CommitObservation::Acknowledged(StartupAck::AttemptFailed {
                    error_code:
                        crate::local_process_owner::owner_protocol::OwnerFailure::SpawnFailed,
                })
            );
        }
        if case == "failure_retry" {
            std::thread::sleep(Duration::from_millis(150));
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "storage refusal dropped failed owner"
            );
            assert_eq!(fs::read(&outcome_path).unwrap(), b"fixture-owned refusal");
            let program = fixture.workspace.join("absent-program");
            fs::write(&program, b"#!/bin/sh\nprintf replayed >> effects\n").unwrap();
            fs::set_permissions(program, fs::Permissions::from_mode(0o700)).unwrap();
            // Remove only this fixture's deliberate refusal, never a stored outcome.
            fs::remove_file(&outcome_path).unwrap();
        }
        assert!(!child.wait().success());
        assert!(!fixture.workspace.join("effects").exists());
        let reopened = fixture.resume_store();
        let restored = reopened.load(bootstrap.job_id).unwrap().unwrap();
        assert!(restored.authorizes(&credential, &bootstrap.binding_digest));
        let recovered = reopened
            .load_outcome(&restored)
            .unwrap()
            .expect("failed startup lost its recoverable outcome");
        match recovered {
            DurableOwnerOutcome::StartupFailed(failure) => {
                assert!(failure.authorizes(&credential, &bootstrap.binding_digest));
                assert_eq!(
                    failure.error_code(),
                    crate::local_process_owner::owner_protocol::OwnerFailure::SpawnFailed
                );
            }
            DurableOwnerOutcome::Terminal(_) => panic!("startup failure became scope completion"),
            DurableOwnerOutcome::NotCommitted(_) => panic!("startup attempt became uncommitted"),
        }
        assert!(reopened.load_terminal(&restored).is_err());
        return;
    }
    if case == "lost_ack" {
        drop(committed);
    } else {
        assert_eq!(
            committed.wait_started(deadline()),
            CommitObservation::Acknowledged(StartupAck::Started {})
        );
    }
    wait_effect(&fixture);
    if case == "cancel" {
        ControlClient::new(&fixture).request(&credential, OwnerAction::Cancel);
    }
    if case == "terminal_retry" {
        let directory = fixture.private.join(AUTHORITY_DIRECTORY);
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
        let mut client = ControlClient::new(&fixture);
        let end = deadline();
        while client.request(&credential, OwnerAction::Inspect).phase != OwnerPhase::Terminal {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        }
        // More replies than one bounded pump can issue prove that the real
        // helper returned through its terminal-persistence branch while the
        // directory was invalid, rather than merely waiting for its payload.
        for _ in 0..=crate::local_process_owner::owner_transport::MAX_PACKETS_PER_PUMP {
            assert_eq!(
                client.request(&credential, OwnerAction::Inspect).phase,
                OwnerPhase::Terminal
            );
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "persistence refusal dropped the owner"
        );
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let terminal = wait_terminal(&fixture, &reference);
    assert_eq!(
        terminal.completion,
        Some(if case == "cancel" {
            OwnerCompletion::Cancelled
        } else {
            OwnerCompletion::Completed
        })
    );
    if case != "cancel" {
        assert_eq!(terminal.root_exit.unwrap().code, Some(7));
    }
    assert!(child.wait().success());
    assert_eq!(fs::read(fixture.workspace.join("effects")).unwrap(), b"x");
}

#[cfg(target_os = "linux")]
#[path = "local_process_owner_keyless_tests.rs"]
pub(in crate::local_process_owner) mod keyless;

pub(super) fn run() {
    #[cfg(target_os = "linux")]
    keyless::run();
    #[cfg(target_os = "linux")]
    for case in [
        "completed",
        "lost_ack",
        "cancel",
        "terminal_retry",
        "no_commit",
        "wrong_fingerprint",
        "invalid_bootstrap",
        "invalid_control",
        "attempt_failed",
        "lost_failure_ack",
        "failure_retry",
    ] {
        linux_case(case);
        println!("OWNER_ENTRY_CASE case={case} actual_executable=true llm_calls=0");
    }
    keyless::package::run();
}
