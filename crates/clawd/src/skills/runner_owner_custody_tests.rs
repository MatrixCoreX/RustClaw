use crate::local_process_owner::package_test_support::OwnedHostHelper as OwnedHelper;
use crate::local_process_owner::package_test_support::*;
use std::fs::{self, File};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn wait_marker(path: &Path) {
    let end = deadline();
    while !path.is_file() {
        assert!(Instant::now() < end, "owned payload did not progress");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_failed_publication(watch: &rustix::fd::OwnedFd) {
    use rustix::fs::inotify;
    let mut buffer = [std::mem::MaybeUninit::uninit(); 4096];
    let mut events = inotify::Reader::new(watch, &mut buffer);
    let end = deadline();
    loop {
        match events.next() {
            Ok(event)
                if event.events().contains(inotify::ReadFlags::DELETE)
                    && event.file_name().is_some_and(|name| {
                        name.to_bytes().starts_with(b".owner-ref-")
                            && name.to_bytes().ends_with(b".tmp")
                    }) =>
            {
                return
            }
            Ok(_) => {}
            Err(rustix::io::Errno::WOULDBLOCK) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => panic!("no_commit_publication_watch_failed:{error:?}"),
        }
        assert!(
            Instant::now() < end,
            "no_commit_publication_attempt_not_observed"
        );
    }
}

fn scenario(case: &str) {
    let fixture = KeylessFixture::new();
    let resources = crate::resource_scheduler::ResourceBroker::default();
    let resource_lease = resources.try_acquire(None, 1).expect("real host admission");
    let admitted_budget = crate::local_process_owner::HostResourceLease::budget(&resource_lease);
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
    let binding = [71_u8; 32];
    let job = Uuid::new_v4();
    let admission_deadline = deadline();
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
            admission_deadline,
        )
        .unwrap();
    let credential = prepared.reference().credential();
    let package = lease
        .retain_owner_custody(package_custody_binding(&credential, binding).unwrap())
        .unwrap();
    let (input, release) = rustix::pipe::pipe().unwrap();
    for descriptor in [&input, &release] {
        rustix::io::fcntl_setfd(descriptor, rustix::io::FdFlags::CLOEXEC).unwrap();
    }
    let release = File::from(release);
    let failed = matches!(case, "startup_failure" | "lost_failure_ack");
    let mut invocation = if failed {
        ProcessInvocation::new(version.join("absent-program"))
    } else {
        ProcessInvocation::new("/bin/sh")
    };
    if !failed {
        invocation.arg("-c").arg("for fd in /proc/self/fd/*; do case \"$(readlink \"$fd\" 2>/dev/null || :)\" in */owner-leases/*|*.lock) exit 91;; esac; done; exec 3<&0; /bin/sh -c 'printf ready > child-ready; read -r _ || :; printf done > child-done' <&3 & printf '%s' \"$!\" > child-pid; exit 7");
    }
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
    let (child, ready, reference, _caller) = prepared
        .ready_launch(launch, Some(package), resource_lease, deadline())
        .unwrap();
    let mut child = OwnedHelper(child);
    assert!(!store.remove_installed_versions("lease_fixture").unwrap());
    drop(lease);
    assert!(version.is_dir());
    if matches!(
        case,
        "budget_abandoned"
            | "budget_expired_after_hold"
            | "budget_lost_reply"
            | "no_commit_persist_retry"
            | "no_commit_persist_crash"
    ) {
        let missing = authority
            .read_resources(fixture.directories(), expected, &reference, deadline())
            .unwrap();
        assert!(missing.budget().is_none() && missing.outcome().is_none());
        assert!(crate::resource_scheduler::ResourceBroker::default()
            .reconcile_owner_resources(&reference, &missing)
            .is_err());
        if case == "budget_lost_reply" {
            hold_resources_discard_reply(
                &authority,
                fixture.directories(),
                expected,
                &reference,
                admitted_budget,
                deadline(),
            );
        } else {
            authority
                .hold_resources(
                    fixture.directories(),
                    expected,
                    &reference,
                    admitted_budget,
                    deadline(),
                )
                .unwrap();
        }
        let pending = authority
            .read_resources(fixture.directories(), expected, &reference, deadline())
            .unwrap();
        assert!(pending.budget().is_some() && pending.outcome().is_none());
        let outcome_path = authority_directory(&fixture.keys)
            .join(format!("{}.outcome", credential.job_id.simple()));
        let watch = if case.starts_with("no_commit_persist_") {
            use std::io::Write;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&outcome_path)
                .unwrap()
                .write_all(b"fixture-owned no-commit refusal")
                .unwrap();
            let watch = rustix::fs::inotify::init(
                rustix::fs::inotify::CreateFlags::CLOEXEC
                    | rustix::fs::inotify::CreateFlags::NONBLOCK,
            )
            .unwrap();
            rustix::fs::inotify::add_watch(
                &watch,
                authority_directory(&fixture.keys),
                rustix::fs::inotify::WatchFlags::DELETE,
            )
            .unwrap();
            Some(watch)
        } else {
            None
        };
        if case == "budget_expired_after_hold" {
            std::thread::sleep(
                admission_deadline.saturating_duration_since(Instant::now())
                    + Duration::from_millis(10),
            );
            assert!(ready
                .commit(
                    &authority,
                    fixture.directories(),
                    expected,
                    &reference,
                    deadline()
                )
                .is_err());
        } else {
            drop(ready);
        }
        if let Some(watch) = &watch {
            wait_failed_publication(watch);
            assert!(child.0.try_wait().unwrap().is_none());
            assert_eq!(
                fs::read(&outcome_path).unwrap(),
                b"fixture-owned no-commit refusal"
            );
            assert!(!store.remove_installed_versions("lease_fixture").unwrap());
            assert!(version.is_dir());
            if case == "no_commit_persist_crash" {
                child.0.kill().unwrap();
                child.wait();
            }
            // Remove only this fixture's deliberately invalid refusal, not an
            // authenticated record or production evidence.
            fs::remove_file(&outcome_path).unwrap();
        }
        if case == "no_commit_persist_crash" {
            let unknown = authority
                .read_resources(fixture.directories(), expected, &reference, deadline())
                .unwrap();
            assert!(unknown.outcome().is_none());
            assert!(!store.remove_installed_versions("lease_fixture").unwrap());
            let recovered = crate::resource_scheduler::ResourceBroker::default();
            assert_eq!(
                recovered
                    .reconcile_owner_resources(&reference, &unknown)
                    .unwrap()
                    .restored,
                1
            );
            assert_eq!(recovered.status().active_leases, 1);
            assert!(!fixture.work.join("child-ready").exists());
            println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
            return;
        }
        assert!(!child.wait().success());
        assert!(!fixture.work.join("child-ready").exists());
        assert_eq!(resources.status().active_leases, 0);
        assert!(store.remove_installed_versions("lease_fixture").unwrap());
        let observation = authority
            .read_resources(fixture.directories(), expected, &reference, deadline())
            .unwrap();
        let recovered = crate::resource_scheduler::ResourceBroker::default();
        let result = recovered
            .reconcile_owner_resources(&reference, &observation)
            .unwrap();
        assert_eq!(
            result.restored, 0,
            "uncommitted_persisted_budget_restored_as_active"
        );
        assert!(observation.outcome().is_some());
        assert!(
            matches!(observation.outcome(), Some(crate::local_process_owner::DurableOwnerOutcome::NotCommitted(proof)) if proof.authorizes(&credential, &binding))
        );
        assert_eq!(recovered.status().active_leases, 0);
        assert_eq!(
            recovered
                .reconcile_owner_resources(&reference, &pending)
                .unwrap()
                .restored,
            0
        );
        assert_eq!(recovered.status().active_leases, 0);
        println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
        return;
    }
    if matches!(
        case,
        "no_commit" | "wrong_reference" | "expired_commit" | "expired_preparation"
    ) {
        match case {
            "wrong_reference" => with_other_reference(binding, |other| {
                assert!(ready
                    .commit(
                        &authority,
                        fixture.directories(),
                        expected,
                        other,
                        deadline()
                    )
                    .is_err());
            }),
            "expired_commit" => assert!(ready
                .commit(
                    &authority,
                    fixture.directories(),
                    expected,
                    &reference,
                    Instant::now()
                )
                .is_err()),
            "expired_preparation" => {
                std::thread::sleep(
                    admission_deadline.saturating_duration_since(Instant::now())
                        + Duration::from_millis(10),
                );
                assert!(ready
                    .commit(
                        &authority,
                        fixture.directories(),
                        expected,
                        &reference,
                        deadline()
                    )
                    .is_err());
            }
            _ => drop(ready),
        }
        assert!(!child.wait().success());
        assert!(matches!(authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap(), Some(crate::local_process_owner::DurableOwnerOutcome::NotCommitted(proof))
                if proof.authorizes(&credential, &binding)));
        assert!(store.remove_installed_versions("lease_fixture").unwrap());
        assert_eq!(resources.status().active_leases, 0);
        let missing = authority
            .read_resources(fixture.directories(), expected, &reference, deadline())
            .unwrap();
        assert!(missing.budget().is_none());
        assert_eq!(
            resources
                .reconcile_owner_resources(&reference, &missing)
                .unwrap()
                .restored,
            0
        );
        println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
        return;
    }
    if case == "commit_transport_loss" {
        child.0.kill().unwrap();
        child.wait();
        let committed = ready
            .commit(
                &authority,
                fixture.directories(),
                expected,
                &reference,
                deadline(),
            )
            .unwrap();
        assert!(matches!(
            committed.wait_started(deadline()),
            CommitObservation::Uncertain(_)
        ));
        assert_eq!(resources.status().active_leases, 1);
        assert!(!store.remove_installed_versions("lease_fixture").unwrap());
        assert!(authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap()
            .is_none());
        assert!(!fixture.work.join("child-ready").exists());
        println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
        return;
    }
    let committed = ready
        .commit(
            &authority,
            fixture.directories(),
            expected,
            &reference,
            deadline(),
        )
        .unwrap();
    assert_eq!(resources.status().active_leases, 1);
    if matches!(case, "lost_ack" | "lost_failure_ack") {
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
        let outcome = authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap()
            .unwrap();
        settle_resources(&resources, &reference, &binding, &outcome);
        assert!(is_startup_failure(outcome, &credential, &binding));
        assert!(store.remove_installed_versions("lease_fixture").unwrap());
        println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
        return;
    }
    wait_marker(&fixture.work.join("child-ready"));
    let mut client = TestControlClient::new(&fixture);
    let end = deadline();
    loop {
        let snapshot = client.request(&credential, OwnerAction::Inspect);
        if snapshot.root_exit.is_some() {
            assert_eq!(snapshot.phase, OwnerPhase::Running);
            assert!(snapshot.completion.is_none());
            break;
        }
        assert!(Instant::now() < end, "owned root did not exit");
        std::thread::sleep(Duration::from_millis(5));
    }
    if matches!(case, "broker_restart" | "late_pending_recovery") {
        assert_eq!(resources.status().active_leases, 1);
        let previous = resources.status();
        let pid: u32 = fs::read_to_string(fixture.work.join("child-pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            crate::local_process_owner::instance::observe_process(pid)
                .unwrap()
                .state,
            crate::local_process_owner::instance::ProcessRunState::Active
        );
        drop(resources);
        let recovered = crate::resource_scheduler::ResourceBroker::default();
        assert_eq!(recovered.status().active_leases, 0);
        let observation = authority
            .read_resources(fixture.directories(), expected, &reference, deadline())
            .unwrap();
        assert!(observation.outcome().is_none());
        let budget = observation.budget().unwrap().clone();
        assert_eq!(budget.memory_mib, previous.reserved_memory_mib);
        assert_eq!(budget.cpu_cores, previous.reserved_cpu_cores);
        let first = recovered
            .reconcile_owner_resources(&reference, &observation)
            .unwrap();
        assert_eq!((first.restored, first.released), (1, 0));
        assert_other_authority_rejected(&reference, budget.clone(), binding);
        let repeated = recovered
            .reconcile_owner_resources(&reference, &observation)
            .unwrap();
        assert_eq!((repeated.restored, repeated.released), (0, 0));
        with_other_reference(binding, |other| {
            assert!(recovered
                .reconcile_owner_resources(other, &observation)
                .is_err());
        });
        let mut conflict = budget.clone();
        conflict.memory_mib += 1;
        assert!(authority
            .hold_resources(
                fixture.directories(),
                expected,
                &reference,
                conflict,
                deadline()
            )
            .is_err());
        assert_eq!(
            authority
                .read_resources(fixture.directories(), expected, &reference, deadline())
                .unwrap()
                .budget(),
            Some(&budget)
        );
        let restored = recovered.status().active_leases;
        drop(release);
        wait_marker(&fixture.work.join("child-done"));
        assert!(child.wait().success());
        assert!(store.remove_installed_versions("lease_fixture").unwrap());
        assert_eq!(restored, 1, "protected_owner_resource_recovery_missing");
        let closed = authority
            .read_resources(fixture.directories(), expected, &reference, deadline())
            .unwrap();
        let settled = recovered
            .reconcile_owner_resources(&reference, &closed)
            .unwrap();
        assert_eq!((settled.restored, settled.released), (0, 1));
        assert_eq!(recovered.status().active_leases, 0);
        let empty = crate::resource_scheduler::ResourceBroker::default();
        assert_eq!(
            empty
                .reconcile_owner_resources(&reference, &closed)
                .unwrap()
                .restored,
            0
        );
        assert_eq!(empty.status().active_leases, 0);
        if case == "late_pending_recovery" {
            let stale = recovered
                .reconcile_owner_resources(&reference, &observation)
                .unwrap();
            assert_eq!(
                stale.restored, 0,
                "late_pending_recovery_resurrected_closed_owner"
            );
            assert_eq!(recovered.status().active_leases, 0);
            let stale = empty
                .reconcile_owner_resources(&reference, &observation)
                .unwrap();
            assert_eq!(stale.restored, 0);
            assert_eq!(empty.status().active_leases, 0);
            let concurrent = crate::resource_scheduler::ResourceBroker::default();
            std::thread::scope(|scope| {
                let pending =
                    scope.spawn(|| concurrent.reconcile_owner_resources(&reference, &observation));
                let terminal =
                    scope.spawn(|| concurrent.reconcile_owner_resources(&reference, &closed));
                pending.join().unwrap().unwrap();
                terminal.join().unwrap().unwrap();
            });
            assert_eq!(concurrent.status().active_leases, 0);
            let lease = recovered.try_acquire(None, 1).unwrap();
            assert!(crate::local_process_owner::HostResourceLease::escrow(
                lease, &reference, &binding
            )
            .is_err());
            assert_eq!(recovered.status().active_leases, 0);
        }
        println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
        return;
    }
    if case == "resource_tamper" {
        let path = authority_directory(&fixture.keys)
            .join(format!("{}.resources.json", credential.job_id));
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let memory = value["record"]["budget"]["memory_mib"].as_u64().unwrap();
        value["record"]["budget"]["memory_mib"] = serde_json::json!(memory + 1);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(authority
            .read_resources(fixture.directories(), expected, &reference, deadline())
            .is_err());
        assert_eq!(resources.status().active_leases, 1);
        drop(release);
        wait_marker(&fixture.work.join("child-done"));
        assert!(child.wait().success());
        let outcome = authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap()
            .unwrap();
        settle_resources(&resources, &reference, &binding, &outcome);
        assert!(store.remove_installed_versions("lease_fixture").unwrap());
        println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
        return;
    }
    if case == "owner_crash" {
        child.0.kill().unwrap();
        child.wait();
        let pid: u32 = fs::read_to_string(fixture.work.join("child-pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            crate::local_process_owner::instance::observe_process(pid)
                .unwrap()
                .state,
            crate::local_process_owner::instance::ProcessRunState::Active
        );
        let removed = store.remove_installed_versions("lease_fixture").unwrap();
        let retained = version.is_dir();
        assert!(authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap()
            .is_none());
        let resource_custody_retained = resources.status().active_leases;
        drop(release);
        wait_marker(&fixture.work.join("child-done"));
        assert!(
            !removed && retained,
            "SDK collected a package while the crashed owner's payload was active"
        );
        // An external observation that the child later exited is still not an authenticated outcome.
        assert!(!store.remove_installed_versions("lease_fixture").unwrap());
        assert_eq!(
            resource_custody_retained, 1,
            "caller_drop_released_resources_while_owned_payload_was_active"
        );
        assert_eq!(resources.status().active_leases, 1);
        println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
        return;
    }
    let directory = authority_directory(&fixture.keys);
    if case.starts_with("terminal_retry") {
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
    }
    if case == "cancel" {
        client.request(&credential, OwnerAction::Cancel);
    }
    drop(release);
    if case.starts_with("terminal_retry") {
        loop {
            if client.request(&credential, OwnerAction::Inspect).phase == OwnerPhase::Terminal {
                break;
            }
            assert!(Instant::now() < end, "owned scope did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(!store.remove_installed_versions("lease_fixture").unwrap());
        assert_eq!(resources.status().active_leases, 1);
        if case == "terminal_retry_crash" {
            child.0.kill().unwrap();
            child.wait();
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(authority
                .read_outcome(fixture.directories(), expected, &reference, deadline())
                .unwrap()
                .is_none());
            assert!(!store.remove_installed_versions("lease_fixture").unwrap());
            assert_eq!(resources.status().active_leases, 1);
            println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
            return;
        }
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert!(child.wait().success());
    let outcome = authority
        .read_outcome(fixture.directories(), expected, &reference, deadline())
        .unwrap()
        .unwrap();
    settle_resources(&resources, &reference, &binding, &outcome);
    assert!(is_terminal(outcome, &credential, &binding));
    assert!(store.remove_installed_versions("lease_fixture").unwrap());
    assert!(!version.exists());
    println!("SDK_CUSTODY_CASE case={case} actual_executable=true llm_calls=0");
}

fn assert_other_authority_rejected(
    reference: &crate::local_process_owner::owner_store::DurableOwnerReference,
    budget: crate::local_process_owner::OwnerResourceBudget,
    binding: [u8; 32],
) {
    let fixture = KeylessFixture::new();
    let authority = authority_host();
    let expected = authority
        .initialize(fixture.directories(), deadline())
        .unwrap();
    let credential = reference.credential();
    // The exact independently execed owner already passed protected-ready and
    // remains Active under the first caller's owned child/resource custody.
    let other = unsafe {
        authority.publish_reference(
            fixture.directories(),
            expected,
            credential.job_id,
            credential.owner.clone(),
            binding,
            deadline(),
        )
    }
    .unwrap();
    assert!(other.credential() != credential);
    authority
        .hold_resources(fixture.directories(), expected, &other, budget, deadline())
        .unwrap();
    assert!(authority
        .read_resources(fixture.directories(), expected, reference, deadline())
        .is_err());
    assert!(authority
        .read_outcome(fixture.directories(), expected, reference, deadline())
        .is_err());
}

fn with_other_reference(
    binding: [u8; 32],
    check: impl FnOnce(&crate::local_process_owner::owner_store::DurableOwnerReference),
) {
    let fixture = KeylessFixture::new();
    let authority = authority_host();
    let expected = authority
        .initialize(fixture.directories(), deadline())
        .unwrap();
    let prepared = authority
        .prepare_owner(
            fixture.directories(),
            expected,
            OwnerPreparationRequest {
                job_id: Uuid::new_v4(),
                binding_digest: binding,
                terminate_grace_millis: 50,
            },
            fixture.control(),
            deadline(),
        )
        .unwrap();
    check(prepared.reference());
}

fn settle_resources(
    resources: &crate::resource_scheduler::ResourceBroker,
    reference: &crate::local_process_owner::owner_store::DurableOwnerReference,
    binding: &[u8; 32],
    outcome: &crate::local_process_owner::DurableOwnerOutcome,
) {
    assert_eq!(resources.status().active_leases, 1);
    assert!(resources
        .settle_owner_reservations(reference, &[19; 32], outcome)
        .is_err());
    with_other_reference(*binding, |other| {
        assert!(resources
            .settle_owner_reservations(other, binding, outcome)
            .is_err());
    });
    assert_eq!(resources.status().active_leases, 1);
    assert_eq!(
        resources
            .settle_owner_reservations(reference, binding, outcome)
            .unwrap(),
        1
    );
    assert_eq!(
        resources
            .settle_owner_reservations(reference, binding, outcome)
            .unwrap(),
        0
    );
    assert_eq!(resources.status().active_leases, 0);
}

#[test]
#[ignore = "Requires the source-pinned non-test executable; selected explicitly by native acceptance"]
fn entry_executable_sdk_custody_acceptance() {
    for case in [
        "budget_abandoned",
        "budget_expired_after_hold",
        "budget_lost_reply",
        "no_commit_persist_retry",
        "no_commit_persist_crash",
        "late_pending_recovery",
        "broker_restart",
        "resource_tamper",
        "owner_crash",
        "completed",
        "lost_ack",
        "cancel",
        "no_commit",
        "wrong_reference",
        "expired_commit",
        "expired_preparation",
        "commit_transport_loss",
        "startup_failure",
        "lost_failure_ack",
        "terminal_retry",
        "terminal_retry_crash",
    ] {
        scenario(case);
    }
}
