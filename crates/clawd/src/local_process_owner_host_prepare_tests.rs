use super::*;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let suffix = uuid::Uuid::new_v4().as_u128() as u64;
        let root = std::env::temp_dir().join(format!("hp-{suffix:016x}"));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        for name in ["keys", "journal"] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(root.join(name))
                .unwrap();
        }
        Self(root)
    }

    fn directories(&self) -> AuthorityDirectories {
        // Owned empty fixture, never production host fencing from metadata.
        unsafe {
            AuthorityDirectories::from_fenced(
                File::open(self.0.join("keys")).unwrap(),
                File::open(self.0.join("journal")).unwrap(),
            )
            .unwrap()
        }
    }

    fn control(&self, directory: &str) -> UnixDatagram {
        let path = self.0.join(directory).join("control.sock");
        let socket = UnixDatagram::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        socket
    }
}

#[cfg(target_os = "linux")]
#[test]
fn unbound_or_unfenced_control_is_rejected_without_mutating_its_flags() {
    use std::os::fd::AsRawFd;
    let fixture = Fixture::new();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(fixture.0.join("unfenced"))
        .unwrap();
    let host = AuthorityHost::current().unwrap();
    for control in [
        UnixDatagram::unbound().unwrap(),
        fixture.control("unfenced"),
    ] {
        let retained = control.try_clone().unwrap();
        let before = unsafe { libc::fcntl(retained.as_raw_fd(), libc::F_GETFL) };
        let error = host
            .prepare_owner(
                fixture.directories(),
                AuthorityFingerprint::from_slice(&[1; 32]).unwrap(),
                OwnerPreparationRequest {
                    job_id: uuid::Uuid::new_v4(),
                    binding_digest: [7; 32],
                    terminate_grace_millis: 50,
                },
                control,
                Instant::now() + Duration::from_secs(1),
            )
            .err()
            .unwrap();
        assert_eq!(error.phase, AuthorityHostPhase::Prepare);
        assert_eq!(error.reason, AuthorityHostReason::InvalidRequest);
        assert_eq!(
            unsafe { libc::fcntl(retained.as_raw_fd(), libc::F_GETFL) },
            before
        );
        for name in ["keys", "journal"] {
            assert_eq!(fs::read_dir(fixture.0.join(name)).unwrap().count(), 0);
        }
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn unsupported_owner_host_cannot_fall_back_to_an_unprotected_process() {
    let fixture = Fixture::new();
    let error = AuthorityHost {}
        .prepare_owner(
            fixture.directories(),
            AuthorityFingerprint::from_slice(&[1; 32]).unwrap(),
            OwnerPreparationRequest {
                job_id: uuid::Uuid::new_v4(),
                binding_digest: [7; 32],
                terminate_grace_millis: 50,
            },
            fixture.control("keys"),
            Instant::now() + Duration::from_secs(1),
        )
        .err()
        .unwrap();
    assert_eq!(error.reason, AuthorityHostReason::Unsupported);
    assert_eq!(fs::read_dir(fixture.0.join("journal")).unwrap().count(), 0);
    assert!(!fixture
        .0
        .join("keys")
        .join(super::super::super::super::owner_authority::DIRECTORY)
        .exists());
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn expired_and_invalid_owner_preparation_cannot_spawn_or_publish() {
    let fixture = Fixture::new();
    let host = AuthorityHost::current().unwrap();
    for case in ["expired", "job", "binding"] {
        let request = OwnerPreparationRequest {
            job_id: if case == "job" {
                uuid::Uuid::nil()
            } else {
                uuid::Uuid::new_v4()
            },
            binding_digest: if case == "binding" { [0; 32] } else { [7; 32] },
            terminate_grace_millis: 50,
        };
        let deadline = if case == "expired" {
            Instant::now()
        } else {
            Instant::now() + Duration::from_secs(1)
        };
        let error = host
            .prepare_owner(
                fixture.directories(),
                AuthorityFingerprint::from_slice(&[1; 32]).unwrap(),
                request,
                UnixDatagram::unbound().unwrap(),
                deadline,
            )
            .err()
            .unwrap();
        assert_eq!(error.phase, AuthorityHostPhase::Prepare);
        assert_eq!(
            error.reason,
            if case == "expired" {
                AuthorityHostReason::DeadlineElapsed
            } else {
                AuthorityHostReason::InvalidRequest
            }
        );
        assert!(!error.request_may_have_been_sent());
        for name in ["keys", "journal"] {
            assert_eq!(fs::read_dir(fixture.0.join(name)).unwrap().count(), 0);
        }
    }
}
