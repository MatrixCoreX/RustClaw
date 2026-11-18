use super::*;
use std::os::fd::AsRawFd;

fn exclusive_available(path: &std::path::Path) -> bool {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return true;
    }
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EWOULDBLOCK)
    );
    false
}

fn scenario(case: &str) {
    let fixture = KeylessFixture::new();
    let authority = AuthorityHost::from_test_executable(File::open(executable()).unwrap()).unwrap();
    let expected = authority
        .initialize(fixture.directories(), deadline())
        .unwrap();
    let lease_path = fixture.root.join("version.lock");
    let lease = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&lease_path)
        .unwrap();
    assert_eq!(unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_SH) }, 0);
    let (mut child, channel) = helper_at(&fixture.work);
    let birth = crate::local_process_owner::instance::observe_process(child.0.id())
        .unwrap()
        .birth;
    let binding = [43_u8; 32];
    let job = Uuid::new_v4();
    // helper_at has observed this exact owned child's protected-ready barrier.
    let reference = unsafe {
        authority.publish_reference(
            fixture.directories(),
            expected,
            job,
            birth,
            binding,
            deadline(),
        )
    }
    .unwrap();
    let credential = reference.credential();
    let bootstrap = OwnerBootstrap {
        schema_version: 1,
        job_id: job,
        authority_fingerprint: *expected.as_bytes(),
        binding_digest: binding,
        terminate_grace_millis: 50,
    };
    let channel = channel
        .send_bootstrap(
            &serde_json::to_vec(&bootstrap).unwrap(),
            [
                File::open(&fixture.keys).unwrap().into(),
                fixture.control().into(),
            ],
            deadline(),
        )
        .unwrap();
    let channel = channel
        .send_package_lease(
            Some(
                crate::local_process_owner::package_test_support::custody_fixture(
                    lease.try_clone().unwrap(),
                    &fixture.root.join("custody.json"),
                    &credential,
                    binding,
                ),
            ),
            &credential,
            deadline(),
        )
        .unwrap();
    let failed = case == "startup_failure";
    let mut invocation = if failed {
        ProcessInvocation::new(fixture.work.join("absent-program"))
    } else {
        ProcessInvocation::new("/bin/sh")
    };
    if !failed {
        invocation.arg("-c").arg("for fd in /proc/self/fd/*; do case \"$(readlink \"$fd\" 2>/dev/null || :)\" in *.lock) exit 91;; esac; done; exec 3<&0; /bin/sh -c 'printf ready > child-ready; read -r _ || :' <&3 & printf '%s' \"$!\" > child-pid; printf x >> effects; exit 7");
    }
    let (input, release) = rustix::pipe::pipe().unwrap();
    for descriptor in [&input, &release] {
        rustix::io::fcntl_setfd(descriptor, rustix::io::FdFlags::CLOEXEC).unwrap();
    }
    let release = File::from(release);
    let launch = ProcessLaunch::new(
        invocation,
        fixture.work.clone(),
        vec![("PATH".into(), "/usr/bin:/bin".into())],
        PayloadIo {
            stdin: Some(File::from(input)),
            stdout: None,
            stderr: None,
        },
    )
    .unwrap();
    let channel = channel
        .send(launch, credential.clone(), deadline())
        .unwrap();
    let ready = HostPendingStart::new(channel, credential.clone(), binding)
        .unwrap()
        .wait_ready(deadline())
        .unwrap();
    drop(lease);
    assert!(!exclusive_available(&lease_path));
    assert!(!fixture.work.join("effects").exists());
    if case == "no_commit" {
        drop(ready);
        assert!(!child.wait().success());
        assert!(exclusive_available(&lease_path));
        assert!(matches!(authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap(), Some(DurableOwnerOutcome::NotCommitted(proof))
                if proof.authorizes(&credential, &binding)));
        println!("OWNER_ENTRY_CASE case=package_lease_{case} actual_executable=true llm_calls=0");
        return;
    }
    let committed = ready.commit(&reference, deadline()).unwrap();
    if case == "lost_ack" {
        drop(committed);
    } else {
        assert_eq!(
            committed.wait_started(deadline()),
            CommitObservation::Acknowledged(if failed {
                StartupAck::AttemptFailed {
                    error_code:
                        crate::local_process_owner::owner_protocol::OwnerFailure::SpawnFailed,
                }
            } else {
                StartupAck::Started {}
            })
        );
    }
    if failed {
        assert!(!child.wait().success());
        assert!(!fixture.work.join("effects").exists());
        assert!(exclusive_available(&lease_path));
        assert!(
            matches!(authority.read_outcome(fixture.directories(), expected, &reference, deadline()).unwrap(),
            Some(DurableOwnerOutcome::StartupFailed(failure)) if failure.authorizes(&credential, &binding))
        );
        println!("OWNER_ENTRY_CASE case=package_lease_{case} actual_executable=true llm_calls=0");
        return;
    }
    let end = deadline();
    while !fixture.work.join("child-ready").exists() {
        assert!(Instant::now() < end, "owned payload did not become ready");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut client = TestControlClient::new(&fixture);
    loop {
        let snapshot = client.request(&credential, OwnerAction::Inspect);
        if snapshot.root_exit.is_some() {
            assert_eq!(snapshot.phase, OwnerPhase::Running);
            assert!(snapshot.completion.is_none());
            let pid: u32 = fs::read_to_string(fixture.work.join("child-pid"))
                .unwrap()
                .parse()
                .unwrap();
            let descendant = crate::local_process_owner::instance::observe_process(pid).unwrap();
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
    assert!(!exclusive_available(&lease_path));
    assert!(authority
        .read_outcome(fixture.directories(), expected, &reference, deadline())
        .unwrap()
        .is_none());
    let directory = fixture.keys.join(AUTHORITY_DIRECTORY);
    if case == "terminal_retry" {
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
    }
    if case == "cancel" {
        client.request(&credential, OwnerAction::Cancel);
    }
    drop(release);
    if case == "terminal_retry" {
        loop {
            let snapshot = client.request(&credential, OwnerAction::Inspect);
            if snapshot.phase == OwnerPhase::Terminal {
                break;
            }
            assert!(Instant::now() < end, "owned scope did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(!exclusive_available(&lease_path));
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert!(child.wait().success());
    assert!(exclusive_available(&lease_path));
    assert_eq!(fs::read(fixture.work.join("effects")).unwrap(), b"x");
    match authority
        .read_outcome(fixture.directories(), expected, &reference, deadline())
        .unwrap()
        .unwrap()
    {
        DurableOwnerOutcome::Terminal(terminal) => {
            assert!(terminal.authorizes(&credential, &binding));
            assert_eq!(
                terminal.snapshot().completion,
                Some(if case == "cancel" {
                    OwnerCompletion::Cancelled
                } else {
                    OwnerCompletion::Completed
                })
            );
        }
        DurableOwnerOutcome::StartupFailed(_) => panic!("executed payload became startup failure"),
        DurableOwnerOutcome::NotCommitted(_) => panic!("executed payload became uncommitted"),
    }
    println!("OWNER_ENTRY_CASE case=package_lease_{case} actual_executable=true llm_calls=0");
}

pub(crate) fn run() {
    for case in [
        "held_child",
        "lost_ack",
        "cancel",
        "no_commit",
        "startup_failure",
        "terminal_retry",
    ] {
        scenario(case);
    }
}
