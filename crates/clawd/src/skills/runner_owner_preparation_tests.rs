use crate::local_process_owner::package_test_support::*;
use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn exact_child_is_reaped(pid: u32) -> bool {
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD)
}

fn wait_for_exact_reaping(pid: u32) -> bool {
    let end = deadline();
    while Instant::now() < end {
        if exact_child_is_reaped(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
#[ignore = "Requires the source-pinned non-test executable; selected explicitly by native acceptance"]
fn entry_executable_ready_abort_acceptance() {
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
    let binding = [83; 32];
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
            deadline(),
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
    drop(lease);
    let mut invocation = ProcessInvocation::new("/bin/sh");
    invocation
        .arg("-c")
        .arg("printf unexpected > payload-started");
    let launch = ProcessLaunch::new(
        invocation,
        fixture.work.clone(),
        vec![("PATH".into(), "/usr/bin:/bin".into())],
        PayloadIo {
            stdin: None,
            stdout: None,
            stderr: None,
        },
    )
    .unwrap();
    let (entered, paused) = mpsc::channel();
    let (resume, blocked) = mpsc::channel();
    prepared.before_ready_for_test(move || {
        entered.send(()).unwrap();
        blocked.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    let (finished, completion) = mpsc::channel();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let task = runtime.spawn(async move {
        tokio::task::spawn_blocking(move || {
            let result = prepared.ready_launch(launch, Some(package), resources, deadline());
            assert!(result.is_ok());
            finished.send(()).unwrap();
            result
        })
        .await
    });
    runtime.block_on(async {
        loop {
            match paused.try_recv() {
                Ok(()) => break,
                Err(mpsc::TryRecvError::Empty) => {
                    tokio::time::sleep(Duration::from_millis(5)).await
                }
                Err(error) => panic!("actual_ready_pause_failed:{error:?}"),
            }
        }
    });
    assert!(!store.remove_installed_versions("lease_fixture").unwrap());
    assert!(!fixture.work.join("payload-started").exists());
    task.abort();
    assert!(matches!(runtime.block_on(task), Err(error) if error.is_cancelled()));
    resume.send(()).unwrap();
    completion.recv_timeout(Duration::from_secs(10)).unwrap();
    drop(runtime);
    // Let Tokio drop the unobserved blocking result; it contains the only
    // returned child handle. No fixture waiter substitutes for production here.
    let reaped = wait_for_exact_reaping(credential.owner.pid);
    if !reaped {
        let snapshot =
            crate::local_process_owner::instance::observe_process(credential.owner.pid).unwrap();
        assert_eq!(snapshot.birth, credential.owner);
        assert_eq!(
            snapshot.state,
            crate::local_process_owner::instance::ProcessRunState::Zombie
        );
        // Reap only the already-confirmed fixture zombie, before the assertion.
        assert_eq!(
            unsafe {
                libc::waitpid(
                    credential.owner.pid as i32,
                    std::ptr::null_mut(),
                    libc::WNOHANG,
                )
            },
            credential.owner.pid as i32
        );
    }
    let observation = authority
        .read_resources(fixture.directories(), expected, &reference, deadline())
        .unwrap();
    assert!(
        matches!(observation.outcome(), Some(crate::local_process_owner::DurableOwnerOutcome::NotCommitted(proof)) if proof.authorizes(&credential, &binding))
    );
    assert_eq!(broker.status().active_leases, 0);
    assert!(store.remove_installed_versions("lease_fixture").unwrap());
    assert!(!fixture.work.join("payload-started").exists());
    assert!(reaped, "async_ready_output_drop_left_owned_helper_unreaped");
    println!("OWNER_READY_ABORT_CASE actual_executable=true llm_calls=0 child_reaped=true");
}
