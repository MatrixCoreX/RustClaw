use crate::local_process_owner::package_test_support::OwnedHostHelper as OwnedHelper;
use crate::local_process_owner::package_test_support::*;
use std::fs::{self, File};
use std::sync::mpsc;
use std::time::Duration;

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
    let binding = [79; 32];
    let admission_deadline = deadline();
    let prepared = authority
        .prepare_owner(
            fixture.directories(),
            expected,
            OwnerPreparationRequest {
                job_id: uuid::Uuid::new_v4(),
                binding_digest: binding,
                terminate_grace_millis: 50,
            },
            fixture.control(),
            admission_deadline,
        )
        .unwrap();
    let credential = prepared.reference().credential();
    let package = lease
        .retain_owner_custody(package_custody_binding(&credential, binding).unwrap())
        .unwrap();
    let (input, release) = rustix::pipe::pipe().unwrap();
    for fd in [&input, &release] {
        rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::CLOEXEC).unwrap();
    }
    let release = File::from(release);
    let mut invocation = ProcessInvocation::new("/bin/sh");
    invocation
        .arg("-c")
        .arg("printf ready > child-ready; read -r _ || :; printf done > child-done");
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
    let (child, mut ready, reference, caller) = prepared
        .ready_launch(launch, Some(package), resources, admission_deadline)
        .unwrap();
    let mut child = OwnedHelper(child);
    drop(lease);
    let recovery_reference = authority
        .load_reference(
            fixture.directories(),
            expected,
            credential.job_id,
            credential.owner.clone(),
            binding,
            deadline(),
        )
        .unwrap();
    let (entered, blocked) = mpsc::channel();
    let (resume, release_block) = mpsc::channel();
    let pause = move || {
        entered.send(()).unwrap();
        release_block.recv_timeout(Duration::from_secs(20)).unwrap();
    };
    let before_hold = if case == "abort_before_hold" {
        Some(pause)
    } else {
        if case == "abort_after_barrier" {
            ready.after_submit_for_test(pause);
        } else {
            ready.before_submit_for_test(pause);
        }
        None
    };
    let (finished, completion) = mpsc::channel();
    let directories = fixture.directories();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let task = runtime.spawn(async move {
        // This guard belongs to the actual awaiting future, not spawn_blocking.
        let _caller = caller;
        tokio::task::spawn_blocking(move || {
            if let Some(pause) = before_hold {
                pause();
            }
            let result = ready.commit(&authority, directories, expected, &reference, deadline());
            let started = result.map(|committed| committed.wait_started(deadline()));
            finished.send(started).unwrap();
        })
        .await
        .unwrap();
    });
    runtime.block_on(async {
        loop {
            match blocked.try_recv() {
                Ok(()) => break,
                Err(mpsc::TryRecvError::Empty) => {
                    tokio::time::sleep(Duration::from_millis(5)).await
                }
                Err(error) => panic!("actual_submit_pause_failed:{error:?}"),
            }
        }
    });
    let pending = authority_host()
        .read_resources(
            fixture.directories(),
            expected,
            &recovery_reference,
            deadline(),
        )
        .unwrap();
    assert_eq!(pending.budget().is_some(), case != "abort_before_hold");
    assert!(pending.outcome().is_none());
    assert!(!store.remove_installed_versions("lease_fixture").unwrap());
    assert!(!fixture.work.join("child-ready").exists());
    let task = if matches!(case, "uncanceled" | "deadline_after_hold") {
        Some(task)
    } else {
        task.abort();
        assert!(runtime.block_on(task).unwrap_err().is_cancelled());
        None
    };
    assert!(child.0.try_wait().unwrap().is_none());
    if case == "deadline_after_hold" {
        std::thread::sleep(
            admission_deadline.saturating_duration_since(std::time::Instant::now())
                + Duration::from_millis(20),
        );
    }
    resume.send(()).unwrap();
    let result = completion.recv_timeout(Duration::from_secs(10)).unwrap();
    if let Some(task) = task {
        runtime.block_on(task).unwrap();
    }
    // Finish/reap this fixture's owned process before any behavior assertion.
    drop(release);
    let status = child.wait();
    let ran = fixture.work.join("child-ready").exists();
    let observation = authority_host()
        .read_resources(
            fixture.directories(),
            expected,
            &recovery_reference,
            deadline(),
        )
        .unwrap();
    let should_run = matches!(case, "abort_after_barrier" | "uncanceled");
    assert_eq!(ran, should_run, "async_caller_submission_barrier_violated");
    assert_eq!(result.is_ok(), should_run);
    assert_eq!(status.success(), should_run);
    if should_run {
        assert!(matches!(
            result.unwrap(),
            CommitObservation::Acknowledged(StartupAck::Started {})
        ));
        assert!(
            matches!(observation.outcome(), Some(crate::local_process_owner::DurableOwnerOutcome::Terminal(terminal)) if terminal.authorizes(&credential, &binding))
        );
        assert_eq!(broker.status().active_leases, 1);
        assert_eq!(
            broker
                .reconcile_owner_resources(&recovery_reference, &observation)
                .unwrap()
                .released,
            1
        );
    } else {
        let error = result.unwrap_err();
        if case == "deadline_after_hold" {
            assert!(matches!(error, HostLaunchFailure::DeadlineElapsed));
        } else {
            assert!(matches!(error, HostLaunchFailure::CallerAbandoned));
        }
        assert!(
            matches!(observation.outcome(), Some(crate::local_process_owner::DurableOwnerOutcome::NotCommitted(proof)) if proof.authorizes(&credential, &binding))
        );
        assert_eq!(
            crate::resource_scheduler::ResourceBroker::default()
                .reconcile_owner_resources(&recovery_reference, &observation)
                .unwrap()
                .restored,
            0
        );
    }
    assert_eq!(broker.status().active_leases, 0);
    assert!(store.remove_installed_versions("lease_fixture").unwrap());
    println!("OWNER_ASYNC_SUBMIT_CASE case={case} actual_executable=true llm_calls=0");
}

#[test]
#[ignore = "Requires the source-pinned non-test executable; selected explicitly by native acceptance"]
fn entry_executable_async_submit_acceptance() {
    for case in [
        "abort_after_hold",
        "abort_before_hold",
        "abort_after_barrier",
        "uncanceled",
        "deadline_after_hold",
    ] {
        scenario(case);
    }
}
