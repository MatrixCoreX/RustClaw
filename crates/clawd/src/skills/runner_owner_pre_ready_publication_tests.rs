use crate::local_process_owner::package_test_support::*;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

fn wait_publication_refusal(watch: &rustix::fd::OwnedFd) {
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
                return;
            }
            Ok(_) => {}
            Err(rustix::io::Errno::WOULDBLOCK) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => panic!("pre_ready_publication_watch_failed:{error:?}"),
        }
        assert!(Instant::now() < end, "pre_ready_publication_not_observed");
    }
}

pub(super) fn scenario(crash_before_publication: bool) {
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
    let binding = [94; 32];
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
            deadline(),
        )
        .unwrap();
    let (child, channel, reference) = prepared.into_launch_parts();
    let mut child = OwnedPreparedHelper(child);
    let credential = reference.credential();
    let package = lease
        .retain_owner_custody(package_custody_binding(&credential, binding).unwrap())
        .unwrap();
    let directory = authority_directory(&fixture.keys);
    let outcome_path = directory.join(format!("{}.outcome", credential.job_id.simple()));
    let refusal = b"fixture-owned pre-ready publication refusal";
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&outcome_path)
        .unwrap()
        .write_all(refusal)
        .unwrap();
    let watch = rustix::fs::inotify::init(
        rustix::fs::inotify::CreateFlags::CLOEXEC | rustix::fs::inotify::CreateFlags::NONBLOCK,
    )
    .unwrap();
    rustix::fs::inotify::add_watch(&watch, &directory, rustix::fs::inotify::WatchFlags::DELETE)
        .unwrap();
    // Transfer only the real SDK capsule, then close without a launch frame.
    drop(
        channel
            .send_package_lease(Some(package), &credential, deadline())
            .unwrap(),
    );
    drop(lease);
    drop(resources);
    wait_publication_refusal(&watch);
    assert!(child.0.try_wait().unwrap().is_none());
    assert_eq!(fs::read(&outcome_path).unwrap(), refusal);
    assert!(!store.remove_installed_versions("lease_fixture").unwrap());
    assert!(version.is_dir());
    assert_eq!(fs::read_dir(&fixture.work).unwrap().count(), 0);
    if crash_before_publication {
        child.0.kill().unwrap();
        child.wait();
    }
    // Remove only this fixture's deliberately invalid blocking record.
    fs::remove_file(&outcome_path).unwrap();
    if !crash_before_publication {
        assert!(!child.wait().success());
    }
    let observed = authority
        .read_resources(fixture.directories(), expected, &reference, deadline())
        .unwrap();
    assert!(observed.budget().is_none());
    let fresh = crate::resource_scheduler::ResourceBroker::default();
    if crash_before_publication {
        assert!(observed.outcome().is_none());
        assert!(fresh
            .reconcile_owner_resources(&reference, &observed)
            .is_err());
        assert!(!store.remove_installed_versions("lease_fixture").unwrap());
        assert!(version.is_dir());
    } else {
        assert!(
            matches!(observed.outcome(), Some(crate::local_process_owner::DurableOwnerOutcome::NotCommitted(proof)) if proof.authorizes(&credential, &binding))
        );
        assert_eq!(
            fresh
                .reconcile_owner_resources(&reference, &observed)
                .unwrap()
                .restored,
            0
        );
        assert!(store.remove_installed_versions("lease_fixture").unwrap());
    }
    assert_eq!(broker.status().active_leases, 0);
    assert_eq!(fs::read_dir(&fixture.work).unwrap().count(), 0);
    let case = if crash_before_publication {
        "pre_ready_publication_crash"
    } else {
        "pre_ready_publication_retry"
    };
    println!("OWNER_PRE_READY_CASE case={case} actual_executable=true llm_calls=0 crash_before_publication={crash_before_publication}");
}
