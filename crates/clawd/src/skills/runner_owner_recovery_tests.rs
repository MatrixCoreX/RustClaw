use crate::local_process_owner::owner_store::DurableOwnerReference;
use crate::local_process_owner::package_test_support::*;
use crate::resource_scheduler::ResourceBroker;
use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;

struct Job {
    child: OwnedHostHelper,
    release: Option<File>,
    reference: DurableOwnerReference,
}
impl Job {
    fn finish(&mut self) {
        drop(self.release.take());
        assert!(self.child.wait().success());
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        drop(self.release.take());
    }
}

fn control(fixture: &KeylessFixture, name: &str) -> UnixDatagram {
    let path = fixture.keys.join(name);
    let control = UnixDatagram::bind(&path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    control
}

fn start(
    fixture: &KeylessFixture,
    authority: &AuthorityHost,
    expected: AuthorityFingerprint,
    broker: &ResourceBroker,
    store: &skill_sdk::InstallReceiptStore,
    name: &str,
) -> Job {
    let resources = broker.try_acquire(None, 1).unwrap();
    let binding = [95; 32];
    let prepared = authority
        .prepare_owner(
            fixture.directories(),
            expected,
            OwnerPreparationRequest {
                job_id: uuid::Uuid::new_v4(),
                binding_digest: binding,
                terminate_grace_millis: 50,
            },
            control(fixture, name),
            deadline(),
        )
        .unwrap();
    let credential = prepared.reference().credential();
    let version = store
        .version_dir("lease_fixture", "1.0.0", &"a".repeat(64))
        .unwrap();
    fs::create_dir_all(&version).unwrap();
    let artifact = version.join("artifact");
    if artifact.exists() {
        assert_eq!(fs::read(&artifact).unwrap(), b"owned SDK version");
    } else {
        fs::write(&artifact, b"owned SDK version").unwrap();
    }
    let lease = store
        .acquire_version_lease("lease_fixture", &version)
        .unwrap();
    let package = lease
        .retain_owner_custody(package_custody_binding(&credential, binding).unwrap())
        .unwrap();
    let (input, release) = rustix::pipe::pipe().unwrap();
    for fd in [&input, &release] {
        rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::CLOEXEC).unwrap();
    }
    let mut invocation = ProcessInvocation::new("/bin/sh");
    invocation.arg("-c").arg("read -r _ || :");
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
        .ready_launch(launch, Some(package), resources, deadline())
        .unwrap();
    assert_eq!(
        ready
            .commit(
                authority,
                fixture.directories(),
                expected,
                &reference,
                deadline()
            )
            .unwrap()
            .wait_started(deadline()),
        CommitObservation::Acknowledged(StartupAck::Started {})
    );
    drop(lease);
    Job {
        child: OwnedHostHelper(child),
        release: Some(File::from(release)),
        reference,
    }
}

fn close_uncommitted(
    fixture: &KeylessFixture,
    authority: &AuthorityHost,
    expected: AuthorityFingerprint,
    broker: &ResourceBroker,
) {
    let resources = broker.try_acquire(None, 1).unwrap();
    let prepared = authority
        .prepare_owner(
            fixture.directories(),
            expected,
            OwnerPreparationRequest {
                job_id: uuid::Uuid::new_v4(),
                binding_digest: [96; 32],
                terminate_grace_millis: 50,
            },
            control(fixture, "b.sock"),
            deadline(),
        )
        .unwrap();
    let launch = ProcessLaunch::new(
        ProcessInvocation::new("/bin/true"),
        fixture.work.clone(),
        vec![],
        PayloadIo {
            stdin: None,
            stdout: None,
            stderr: None,
        },
    )
    .unwrap();
    let (child, ready, reference, _caller) = prepared
        .ready_launch(launch, None, resources, deadline())
        .unwrap();
    drop(ready);
    OwnedHostHelper(child).wait();
    assert!(matches!(
        authority
            .read_outcome(fixture.directories(), expected, &reference, deadline())
            .unwrap(),
        Some(crate::local_process_owner::DurableOwnerOutcome::NotCommitted(_))
    ));
}

fn assert_closed(broker: &ResourceBroker) {
    assert_eq!(
        broker.try_acquire(None, 1).err().unwrap().wait_reason,
        Some("owner_recovery_pending")
    );
    assert_eq!(broker.status().active_leases, 0);
}

fn complete_inventory() {
    let fixture = KeylessFixture::new();
    let authority = authority_host();
    let expected = authority
        .initialize(fixture.directories(), deadline())
        .unwrap();
    let old = ResourceBroker::default();
    let store = skill_sdk::InstallReceiptStore::new(fixture.root.join("packages"));
    let mut first = start(&fixture, &authority, expected, &old, &store, "a.sock");
    close_uncommitted(&fixture, &authority, expected, &old);
    let mut second = start(&fixture, &authority, expected, &old, &store, "c.sock");
    assert_eq!(old.status().active_leases, 2);
    assert!(!store.remove_installed_versions("lease_fixture").unwrap());
    drop(old);
    drop(authority);
    let resumed = authority_host();
    assert_eq!(
        resumed
            .resume(fixture.directories(), expected, deadline())
            .unwrap(),
        expected
    );
    let fresh = ResourceBroker::for_owner_recovery();
    assert_closed(&fresh);
    let inventory = resumed
        .inventory_resources(fixture.directories(), expected, deadline())
        .unwrap();
    assert_eq!(inventory.examined(), 3);
    assert_eq!(inventory.pending(), 2);
    assert_closed(&fresh);
    // This owned fixture performs no concurrent launches/holds; exclusive host
    // admission is explicit here, not inferred for the ordinary daemon.
    assert_eq!(
        unsafe { fresh.complete_owner_recovery(inventory) }.unwrap(),
        2
    );
    assert_eq!(fresh.status().active_leases, 2);
    assert_eq!(fresh.status().reserved_cpu_cores, 2);
    assert!(fresh.status().reserved_memory_mib >= 256);
    let again = resumed
        .inventory_resources(fixture.directories(), expected, deadline())
        .unwrap();
    assert!(unsafe { fresh.complete_owner_recovery(again) }.is_err());
    assert_eq!(fresh.status().active_leases, 2);
    for job in [&mut first, &mut second] {
        job.finish();
        let observed = resumed
            .read_resources(fixture.directories(), expected, &job.reference, deadline())
            .unwrap();
        assert_eq!(
            fresh
                .reconcile_owner_resources(&job.reference, &observed)
                .unwrap()
                .released,
            1
        );
    }
    assert_eq!(fresh.status().active_leases, 0);
    assert!(store.remove_installed_versions("lease_fixture").unwrap());
    println!("OWNER_RECOVERY_CASE case=complete_inventory actual_executable=true llm_calls=0 examined=3 restored=2 no_public_job_files=true");
}

fn rejected_inventory(case: &str) {
    let fixture = KeylessFixture::new();
    let authority = authority_host();
    let expected = authority
        .initialize(fixture.directories(), deadline())
        .unwrap();
    let old = ResourceBroker::default();
    let store = skill_sdk::InstallReceiptStore::new(fixture.root.join("packages"));
    let mut job = start(&fixture, &authority, expected, &old, &store, "a.sock");
    let path = authority_directory(&fixture.keys).join(format!(
        "{}.resources.json",
        job.reference.credential().job_id
    ));
    let original = fs::read(&path).unwrap();
    let supplied = match case {
        "missing_budget" => {
            fs::remove_file(&path).unwrap();
            expected
        }
        "corrupt_budget" => {
            fs::write(&path, b"fixture-owned invalid budget").unwrap();
            expected
        }
        "wrong_authority" => AuthorityFingerprint::from_slice(&[19; 32]).unwrap(),
        _ => unreachable!(),
    };
    drop(old);
    let fresh = ResourceBroker::for_owner_recovery();
    assert!(authority
        .inventory_resources(fixture.directories(), supplied, deadline())
        .is_err());
    assert_closed(&fresh);
    assert!(!store.remove_installed_versions("lease_fixture").unwrap());
    if case != "wrong_authority" {
        // Restore only this fixture's deliberately removed/corrupted record.
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut restored = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        restored.write_all(&original).unwrap();
        restored.sync_all().unwrap();
    }
    let inventory = authority
        .inventory_resources(fixture.directories(), expected, deadline())
        .unwrap();
    assert_eq!(inventory.pending(), 1);
    assert_eq!(
        unsafe { fresh.complete_owner_recovery(inventory) }.unwrap(),
        1
    );
    job.finish();
    let observed = authority
        .read_resources(fixture.directories(), expected, &job.reference, deadline())
        .unwrap();
    assert_eq!(
        fresh
            .reconcile_owner_resources(&job.reference, &observed)
            .unwrap()
            .released,
        1
    );
    assert!(store.remove_installed_versions("lease_fixture").unwrap());
    println!("OWNER_RECOVERY_CASE case={case} actual_executable=true llm_calls=0 refused_while_invalid=true explicit_fresh_retry=true");
}

fn empty_inventory() {
    let fixture = KeylessFixture::new();
    let authority = authority_host();
    let expected = authority
        .initialize(fixture.directories(), deadline())
        .unwrap();
    let fresh = ResourceBroker::for_owner_recovery();
    assert_closed(&fresh);
    let inventory = authority
        .inventory_resources(fixture.directories(), expected, deadline())
        .unwrap();
    assert_eq!(inventory.examined(), 0);
    assert_eq!(inventory.pending(), 0);
    assert_eq!(
        unsafe { fresh.complete_owner_recovery(inventory) }.unwrap(),
        0
    );
    if let Err(grant) = fresh.try_acquire(None, 1) {
        assert_ne!(grant.wait_reason, Some("owner_recovery_pending"));
    }
    println!("OWNER_RECOVERY_CASE case=authenticated_empty actual_executable=true llm_calls=0");
}

#[test]
#[ignore = "Requires the source-pinned non-test executable; selected explicitly by native acceptance"]
fn entry_executable_inventory_recovery_acceptance() {
    complete_inventory();
    for case in ["missing_budget", "corrupt_budget", "wrong_authority"] {
        rejected_inventory(case);
    }
    empty_inventory();
}
