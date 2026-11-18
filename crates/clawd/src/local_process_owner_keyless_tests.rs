use super::*;
use crate::local_process_owner::authority_entry::host::{
    AuthorityDirectories, AuthorityHost, AuthorityHostPhase, AuthorityHostReason,
    OwnerPreparationRequest,
};

pub(crate) struct KeylessFixture {
    pub(crate) root: PathBuf,
    pub(crate) keys: PathBuf,
    journal: PathBuf,
    pub(crate) work: PathBuf,
}

impl KeylessFixture {
    pub(crate) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ko-{}", Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let keys = root.join("keys");
        let journal = root.join("journal");
        let work = root.join("work");
        for directory in [&keys, &journal, &work] {
            fs::DirBuilder::new().mode(0o700).create(directory).unwrap();
        }
        Self {
            root,
            keys,
            journal,
            work,
        }
    }

    pub(crate) fn directories(&self) -> AuthorityDirectories {
        // Owned empty fixture, not admission of production managed payloads.
        unsafe {
            AuthorityDirectories::from_fenced(
                File::open(&self.keys).unwrap(),
                File::open(&self.journal).unwrap(),
            )
            .unwrap()
        }
    }

    pub(crate) fn control(&self) -> UnixDatagram {
        let path = self.keys.join("owner.sock");
        let socket = UnixDatagram::bind(&path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        socket
    }
}

impl Drop for KeylessFixture {
    fn drop(&mut self) {
        let directory = self.keys.join(AUTHORITY_DIRECTORY);
        if directory.is_dir() {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::remove_dir_all(&self.root).unwrap();
    }
}

pub(crate) struct TestControlClient {
    socket: UnixDatagram,
    endpoint: PathBuf,
    next_request: u64,
}

impl TestControlClient {
    pub(crate) fn new(fixture: &KeylessFixture) -> Self {
        let path = fixture.keys.join("client.sock");
        let socket = UnixDatagram::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        Self {
            socket,
            endpoint: fixture.keys.join("owner.sock"),
            next_request: 1,
        }
    }

    pub(crate) fn request(
        &mut self,
        credential: &OwnerCredential,
        action: OwnerAction,
    ) -> OwnerSnapshot {
        let request = OwnerRequest {
            schema_version: 1,
            credential: credential.clone(),
            request_id: self.next_request,
            action,
        };
        self.socket
            .send_to(&serde_json::to_vec(&request).unwrap(), &self.endpoint)
            .unwrap();
        let mut bytes = [0_u8; 4097];
        let (length, sender) = self.socket.recv_from(&mut bytes).unwrap();
        assert_eq!(sender.as_pathname(), Some(self.endpoint.as_path()));
        let reply =
            crate::local_process_owner::owner_protocol::decode_reply(&bytes[..length]).unwrap();
        assert_eq!(reply.request_id, self.next_request);
        self.next_request += 1;
        match reply.outcome {
            OwnerOutcome::Ok { snapshot } => snapshot,
            OwnerOutcome::Error { error_code } => panic!("owned control refused: {error_code:?}"),
        }
    }
}

fn scenario(case: &str) {
    let fixture = KeylessFixture::new();
    let authority = AuthorityHost::from_test_executable(File::open(executable()).unwrap()).unwrap();
    let expected = authority
        .initialize(fixture.directories(), deadline())
        .unwrap();
    let job = Uuid::new_v4();
    let binding = [31_u8; 32];
    let host_dumpable = unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) };
    let prepared = authority
        .prepare_owner(
            fixture.directories(),
            expected,
            OwnerPreparationRequest {
                job_id: job,
                binding_digest: binding,
                terminate_grace_millis: 50,
            },
            fixture.control(),
            deadline(),
        )
        .unwrap_or_else(|error| panic!("keyless publication failed: {error:?}"));
    let birth = prepared.reference().credential().owner;
    let (child, channel, reference) = prepared.into_launch_parts();
    let mut child = OwnedHelper(child);
    assert_eq!(
        unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) },
        host_dumpable
    );
    assert_eq!(
        File::open(format!("/proc/{}/mem", child.0.id()))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied,
    );
    let credential = reference.credential();
    assert!(reference.authorizes(&credential, &binding));
    let attempt_failed = matches!(case, "startup_failure" | "lost_failure_ack");
    let mut invocation = if attempt_failed {
        ProcessInvocation::new(fixture.work.join("absent-program"))
    } else {
        ProcessInvocation::new("/bin/sh")
    };
    if !attempt_failed {
        invocation.arg("-c").arg(match case {
            "cancel" => "printf x >> effects; sleep 2; exit 7",
            "root_exit_child_live" => "exec 3<&0; /bin/sh -c 'printf ready > child-ready; read -r _' <&3 & printf '%s' \"$!\" > child-pid; printf x >> effects; exit 7",
            _ => "printf x >> effects; sleep 0.2; exit 7",
        });
    }
    let (release, input) = if case == "root_exit_child_live" {
        let (input, release) = rustix::pipe::pipe().unwrap();
        for descriptor in [&input, &release] {
            rustix::io::fcntl_setfd(descriptor, rustix::io::FdFlags::CLOEXEC).unwrap();
        }
        (Some(File::from(release)), Some(File::from(input)))
    } else {
        (None, None)
    };
    let launch = ProcessLaunch::new(
        invocation,
        fixture.work.clone(),
        vec![("PATH".into(), "/usr/bin:/bin".into())],
        PayloadIo {
            stdin: input,
            stdout: None,
            stderr: None,
        },
    )
    .unwrap();
    let channel = channel
        .send_package_lease(None, &credential, deadline())
        .unwrap()
        .send(launch, credential.clone(), deadline())
        .unwrap();
    let ready = HostPendingStart::new(channel, credential.clone(), binding)
        .unwrap()
        .wait_ready(deadline())
        .unwrap();
    assert!(!fixture.work.join("effects").exists());
    assert!(authority
        .read_outcome(fixture.directories(), expected, &reference, deadline())
        .unwrap()
        .is_none());
    let committed = ready.commit(&reference, deadline()).unwrap();
    if matches!(case, "lost_ack" | "lost_failure_ack") {
        drop(committed);
    } else {
        assert_eq!(
            committed.wait_started(deadline()),
            CommitObservation::Acknowledged(if attempt_failed {
                StartupAck::AttemptFailed {
                    error_code:
                        crate::local_process_owner::owner_protocol::OwnerFailure::SpawnFailed,
                }
            } else {
                StartupAck::Started {}
            })
        );
    }
    if attempt_failed {
        assert!(!child.wait().success());
        assert!(!fixture.work.join("effects").exists());
        let outcome = authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap()
            .expect("authenticated startup failure missing");
        match outcome {
            DurableOwnerOutcome::StartupFailed(failure) => {
                assert!(failure.authorizes(&credential, &binding));
                assert_eq!(
                    failure.error_code(),
                    crate::local_process_owner::owner_protocol::OwnerFailure::SpawnFailed
                );
            }
            DurableOwnerOutcome::Terminal(_) => panic!("startup failure became scope terminal"),
            DurableOwnerOutcome::NotCommitted(_) => panic!("startup attempt became uncommitted"),
        }
        println!("OWNER_ENTRY_CASE case=keyless_{case} actual_executable=true llm_calls=0");
        return;
    }
    let end = deadline();
    while !fixture.work.join("effects").exists() {
        assert!(Instant::now() < end, "owned payload did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
    if case == "cancel" {
        let snapshot = TestControlClient::new(&fixture).request(&credential, OwnerAction::Cancel);
        assert!(matches!(
            snapshot.phase,
            OwnerPhase::Cancelling | OwnerPhase::Terminal
        ));
    }
    if case == "root_exit_child_live" {
        while !fixture.work.join("child-ready").exists() {
            assert!(
                Instant::now() < end,
                "owned descendant did not become ready"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut client = TestControlClient::new(&fixture);
        loop {
            let snapshot = client.request(&credential, OwnerAction::Inspect);
            if snapshot.root_exit.is_some() {
                assert_eq!(snapshot.phase, OwnerPhase::Running);
                assert!(snapshot.completion.is_none());
                let child_pid: u32 = fs::read_to_string(fixture.work.join("child-pid"))
                    .unwrap()
                    .parse()
                    .unwrap();
                let descendant =
                    crate::local_process_owner::instance::observe_process(child_pid).unwrap();
                assert_eq!(descendant.parent_pid, child.0.id());
                assert_eq!(
                    descendant.state,
                    crate::local_process_owner::instance::ProcessRunState::Active
                );
                break;
            }
            assert!(Instant::now() < end, "owned root did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap()
            .is_none());
    }
    drop(release);
    assert!(child.wait().success());
    assert_eq!(fs::read(fixture.work.join("effects")).unwrap(), b"x");
    // Raw bytes are diagnostic only; proof below must come from protected lookup.
    let outcome = fixture
        .keys
        .join(AUTHORITY_DIRECTORY)
        .join(format!("{}.outcome", job.simple()));
    let bytes = fs::read(outcome).unwrap();
    if case == "corrupt_outcome" {
        let path = fixture
            .keys
            .join(AUTHORITY_DIRECTORY)
            .join(format!("{}.outcome", job.simple()));
        let mut forged: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        forged["authenticator"][0] =
            serde_json::json!(forged["authenticator"][0].as_u64().unwrap() ^ 1);
        let changed = serde_json::to_vec(&forged).unwrap();
        fs::write(&path, &changed).unwrap();
        let error = authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .err()
            .expect("tampered outcome was accepted");
        assert_eq!(error.phase, AuthorityHostPhase::Outcome);
        assert!(matches!(error.reason, AuthorityHostReason::Admission(_)));
        assert_eq!(fs::read(path).unwrap(), changed);
        assert_eq!(fs::read(fixture.work.join("effects")).unwrap(), b"x");
        println!("OWNER_ENTRY_CASE case=keyless_{case} actual_executable=true llm_calls=0");
        return;
    }
    let authenticated = authority
        .read_outcome(fixture.directories(), expected, &reference, deadline())
        .unwrap()
        .expect("authenticated terminal missing");
    match authenticated {
        DurableOwnerOutcome::Terminal(terminal) => {
            assert!(terminal.authorizes(&credential, &binding));
            let snapshot = terminal.snapshot();
            assert_eq!(snapshot.phase, OwnerPhase::Terminal);
            assert_eq!(
                snapshot.completion,
                Some(if case == "cancel" {
                    OwnerCompletion::Cancelled
                } else {
                    OwnerCompletion::Completed
                })
            );
            if case != "cancel" {
                assert_eq!(snapshot.root_exit.unwrap().code, Some(7));
            }
        }
        DurableOwnerOutcome::StartupFailed(_) => panic!("executed payload became startup failure"),
        DurableOwnerOutcome::NotCommitted(_) => panic!("executed payload became uncommitted"),
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["kind"], "terminal");
    assert_eq!(value["receipt"]["reference"]["job_id"], job.to_string());
    assert_eq!(value["receipt"]["snapshot"]["phase"], "terminal");
    assert_eq!(
        value["receipt"]["snapshot"]["completion"],
        if case == "cancel" {
            "cancelled"
        } else {
            "completed"
        }
    );
    assert!(!String::from_utf8_lossy(&bytes).contains(&credential.nonce.to_string()));
    // Reloading the same durable reference after owner exit is not a new spawn
    // or a claim that the saved owner remains alive.
    let reloaded = authority
        .load_reference(
            fixture.directories(),
            expected,
            job,
            birth,
            binding,
            deadline(),
        )
        .unwrap_or_else(|error| panic!("keyless reload failed: {error:?}"));
    assert!(reloaded.authorizes(&credential, &binding));
    assert!(
        matches!(authority.read_outcome(fixture.directories(), expected, &reloaded, deadline()).unwrap(),
        Some(DurableOwnerOutcome::Terminal(terminal)) if terminal.authorizes(&credential, &binding))
    );
    assert_eq!(fs::read(fixture.work.join("effects")).unwrap(), b"x");
    println!("OWNER_ENTRY_CASE case=keyless_{case} actual_executable=true llm_calls=0");
}

pub(super) fn run() {
    abandoned_preparation();
    for case in [
        "completed",
        "lost_ack",
        "cancel",
        "startup_failure",
        "lost_failure_ack",
        "root_exit_child_live",
        "corrupt_outcome",
    ] {
        scenario(case);
    }
}

fn abandoned_preparation() {
    let fixture = KeylessFixture::new();
    let owned_image = fixture.root.join("pinned-image");
    fs::copy(executable(), &owned_image).unwrap();
    let authority = AuthorityHost::from_test_executable(File::open(&owned_image).unwrap()).unwrap();
    fs::remove_file(&owned_image).unwrap();
    assert!(!owned_image.exists());
    let host_dumpable = unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) };
    let expected = authority
        .initialize(fixture.directories(), deadline())
        .unwrap();
    let prepared = authority
        .prepare_owner(
            fixture.directories(),
            expected,
            OwnerPreparationRequest {
                job_id: Uuid::new_v4(),
                binding_digest: [37; 32],
                terminate_grace_millis: 50,
            },
            fixture.control(),
            deadline(),
        )
        .unwrap();
    let reference = authority
        .load_reference(
            fixture.directories(),
            expected,
            prepared.reference().credential().job_id,
            prepared.reference().credential().owner,
            [37; 32],
            deadline(),
        )
        .unwrap();
    let birth = reference.credential().owner;
    assert_eq!(
        unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) },
        host_dumpable
    );
    assert_eq!(
        crate::local_process_owner::instance::observe_process(birth.pid)
            .unwrap()
            .birth,
        birth,
    );
    drop(prepared);
    if let Ok(observed) = crate::local_process_owner::instance::observe_process(birth.pid) {
        assert_ne!(observed.birth, birth);
    }
    assert!(matches!(
        authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap(),
        Some(DurableOwnerOutcome::NotCommitted(proof))
            if proof.authorizes(&reference.credential(), &[37; 32])
    ));
    assert_eq!(fs::read_dir(&fixture.work).unwrap().count(), 0);
    println!(
        "OWNER_ENTRY_CASE case=keyless_abandoned_preparation actual_executable=true llm_calls=0"
    );
}

#[path = "local_process_owner_package_entry_tests.rs"]
pub(super) mod package;
