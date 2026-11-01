use crate::local_process_owner::package_test_support::*;
use std::fs::{self, File};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

#[path = "runner_owner_pre_ready_publication_tests.rs"]
mod publication;

fn wait_reaped(pid: u32) {
    let end = deadline();
    loop {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
            return;
        }
        assert!(Instant::now() < end, "pre_ready_helper_not_reaped");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn scenario(case: &str) {
    let fixture = KeylessFixture::new();
    let broker = crate::resource_scheduler::ResourceBroker::default();
    let resources = broker.try_acquire(None, 1).unwrap();
    let authority = authority_host();
    let expected = authority
        .initialize(fixture.directories(), deadline())
        .unwrap();
    let store = skill_sdk::InstallReceiptStore::new(fixture.root.join("packages"));
    let version = store
        .version_dir("lease_fixture", "1.0.0", &"a".repeat(64))
        .unwrap();
    fs::create_dir_all(&version).unwrap();
    fs::write(version.join("artifact"), b"owned SDK version").unwrap();
    let lease = store
        .acquire_version_lease("lease_fixture", &version)
        .unwrap();
    let binding = [89; 32];
    let preparation_deadline = if case == "original_deadline_expired" {
        Instant::now() + Duration::from_secs(2)
    } else {
        deadline()
    };
    let mut prepared = authority
        .prepare_owner(
            fixture.directories(),
            expected,
            OwnerPreparationRequest {
                job_id: uuid::Uuid::new_v4(),
                binding_digest: binding,
                terminate_grace_millis: 50,
            },
            fixture.control(),
            preparation_deadline,
        )
        .unwrap();
    let credential = prepared.reference().credential();
    let reference = authority
        .load_reference(
            fixture.directories(),
            expected,
            credential.job_id,
            credential.owner.clone(),
            binding,
            deadline(),
        )
        .unwrap();
    let package = lease
        .retain_owner_custody(package_custody_binding(&credential, binding).unwrap())
        .unwrap();
    let (socket, _peer) = UnixStream::pair().unwrap();
    let stdin = if case == "launch_refused" {
        let descriptor: OwnedFd = socket.into();
        Some(File::from(descriptor))
    } else {
        None
    };
    let mut invocation = ProcessInvocation::new("/bin/sh");
    invocation
        .arg("-c")
        .arg("printf unexpected > payload-started");
    let launch = ProcessLaunch::new(
        invocation,
        fixture.work.clone(),
        vec![("PATH".into(), "/usr/bin:/bin".into())],
        PayloadIo {
            stdin,
            stdout: None,
            stderr: None,
        },
    )
    .unwrap();
    if case == "registration_refused" {
        prepared.refuse_child_registration_for_test();
    }
    if case == "original_deadline_expired" {
        std::thread::sleep(preparation_deadline.saturating_duration_since(Instant::now()));
    }
    let admission = if case == "expired_before_ready" {
        Instant::now()
    } else {
        deadline()
    };
    let error = prepared
        .ready_launch(launch, Some(package), resources, admission)
        .err()
        .expect("pre_ready_failure_missing");
    match case {
        "launch_refused" => assert!(matches!(error, HostLaunchFailure::Transfer(_))),
        "expired_before_ready" | "original_deadline_expired" => {
            assert!(matches!(error, HostLaunchFailure::DeadlineElapsed))
        }
        "registration_refused" => {
            assert!(matches!(error, HostLaunchFailure::ChildCustodyUnavailable))
        }
        _ => unreachable!(),
    }
    drop(lease);
    wait_reaped(credential.owner.pid);
    assert!(!fixture.work.join("payload-started").exists());
    assert_eq!(broker.status().active_leases, 0);
    let observation = authority
        .read_resources(fixture.directories(), expected, &reference, deadline())
        .unwrap();
    assert!(observation.budget().is_none());
    let no_commit = matches!(observation.outcome(), Some(crate::local_process_owner::DurableOwnerOutcome::NotCommitted(proof)) if proof.authorizes(&credential, &binding));
    let collectible = store.remove_installed_versions("lease_fixture").unwrap();
    println!("OWNER_PRE_READY_CASE case={case} actual_executable=true llm_calls=0 no_commit={no_commit} sdk_collectible={collectible}");
    assert!(no_commit, "pre_ready_missing_authenticated_fate");
    assert!(collectible, "pre_ready_custody_not_collectible");
    assert_eq!(
        crate::resource_scheduler::ResourceBroker::default()
            .reconcile_owner_resources(&reference, &observation)
            .unwrap()
            .restored,
        0
    );
}

#[test]
#[ignore = "Requires the source-pinned non-test executable; selected explicitly by native acceptance"]
fn entry_executable_pre_ready_acceptance() {
    for case in [
        "launch_refused",
        "expired_before_ready",
        "original_deadline_expired",
        "registration_refused",
    ] {
        scenario(case);
    }
    for crash_before_publication in [false, true] {
        publication::scenario(crash_before_publication);
    }
}
