use super::*;

#[cfg(target_os = "linux")]
pub(crate) fn hold_resources_discard_reply(
    host: &AuthorityHost,
    directories: AuthorityDirectories,
    expected: AuthorityFingerprint,
    reference: &DurableOwnerReference,
    budget: crate::local_process_owner::OwnerResourceBudget,
    deadline: Instant,
) {
    let (channel, mut child) = host.spawn(HelperRole::Authority).unwrap();
    assert_eq!(channel.receive_control(deadline).unwrap(), BOOTSTRAP_READY);
    let bytes = serde_json::to_vec(&AuthorityBootstrap {
        schema_version: 1,
        action: AuthorityAction::HoldResources {
            authority_fingerprint: *expected.as_bytes(),
            credential: reference.credential(),
            binding_digest: reference.record().binding_digest,
            budget,
        },
    })
    .unwrap();
    let channel = channel
        .send_bootstrap(
            &bytes,
            [directories.keys.into(), directories.journal.into()],
            deadline,
        )
        .unwrap();
    // Actual pinned helper, but this caller never consumes its reply. A fresh
    // protected query in the SDK scenario must establish what was persisted.
    drop(channel);
    child.wait(deadline).unwrap();
}

#[test]
fn resource_response_binds_authority_reference_and_requested_budget() {
    use crate::local_process_owner::instance::ProcessStart;
    use crate::local_process_owner::owner_outcome::OwnerObservationRecord;
    use crate::local_process_owner::owner_reference::OwnerReference;
    use crate::local_process_owner::OwnerResourceBudget;
    let owner = ProcessBirthIdentity {
        pid: 123,
        boot_id: uuid::Uuid::new_v4(),
        started: ProcessStart::LinuxTicks { ticks: 10 },
    };
    let job = uuid::Uuid::new_v4();
    let budget = OwnerResourceBudget {
        memory_mib: 512,
        cpu_cores: 1,
        network_slots: 0,
        provider_slots: 0,
        browser_slots: 0,
        heavy: false,
        background: true,
    };
    let action = AuthorityAction::HoldResources {
        authority_fingerprint: [1; 32],
        credential: crate::local_process_owner::owner_protocol::OwnerCredential {
            job_id: job,
            owner: owner.clone(),
            nonce: uuid::Uuid::new_v4(),
        },
        binding_digest: [2; 32],
        budget: budget.clone(),
    };
    let response = || AuthorityResponse::Resources {
        schema_version: 1,
        authority_fingerprint: [1; 32],
        reference: OwnerReference::new(job, owner.clone(), [2; 32]).unwrap(),
        budget: Some(budget.clone()),
        observation: OwnerObservationRecord::Pending {},
    };
    assert!(accept_response(&action, response(), ExitStatus::from_raw(0)).is_ok());
    for field in ["epoch", "job", "binding", "budget", "missing"] {
        let mut changed = response();
        let AuthorityResponse::Resources {
            authority_fingerprint,
            reference,
            budget,
            ..
        } = &mut changed
        else {
            unreachable!()
        };
        match field {
            "epoch" => authority_fingerprint[0] ^= 1,
            "job" => reference.job_id = uuid::Uuid::new_v4(),
            "binding" => reference.binding_digest[0] ^= 1,
            "budget" => budget.as_mut().unwrap().memory_mib += 1,
            "missing" => *budget = None,
            _ => unreachable!(),
        }
        assert_eq!(
            accept_response(&action, changed, ExitStatus::from_raw(0))
                .err()
                .unwrap()
                .reason,
            AuthorityHostReason::OutcomeBindingMismatch
        );
    }
    assert_eq!(
        accept_response(
            &AuthorityAction::Initialize {},
            response(),
            ExitStatus::from_raw(0)
        )
        .err()
        .unwrap()
        .reason,
        AuthorityHostReason::ResponseKindMismatch
    );
    assert!(matches!(
        accept_response(&action, response(), ExitStatus::from_raw(1 << 8))
            .err()
            .unwrap()
            .reason,
        AuthorityHostReason::HelperExit(_)
    ));
}
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ah-{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        for name in ["keys", "journal"] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(root.join(name))
                .unwrap();
        }
        Self(root)
    }
    fn directories(&self) -> Result<AuthorityDirectories, AuthorityHostFailure> {
        // New isolated fixture with no managed payloads, not ordinary host
        // admission or proof that filesystem metadata establishes fencing.
        unsafe {
            AuthorityDirectories::from_fenced(
                File::open(self.0.join("keys")).unwrap(),
                File::open(self.0.join("journal")).unwrap(),
            )
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn current_helper_is_pinned_to_the_running_inode_and_cloexec() {
    let host = AuthorityHost::current().unwrap();
    let actual = host.executable.metadata().unwrap();
    let running = fs::metadata("/proc/self/exe").unwrap();
    assert_eq!((actual.dev(), actual.ino()), (running.dev(), running.ino()));
    assert_ne!(
        unsafe { libc::fcntl(host.executable.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
}

#[cfg(target_os = "linux")]
#[test]
fn expired_admission_cannot_send_or_initialize_anything() {
    let fixture = Fixture::new();
    let error = AuthorityHost::current()
        .unwrap()
        .initialize(fixture.directories().unwrap(), Instant::now())
        .unwrap_err();
    assert_eq!(error.phase, AuthorityHostPhase::Prepare);
    assert_eq!(error.reason, AuthorityHostReason::DeadlineElapsed);
    assert!(!error.request_may_have_been_sent());
    for name in ["keys", "journal"] {
        assert_eq!(fs::read_dir(fixture.0.join(name)).unwrap().count(), 0);
    }
}

#[test]
fn host_checks_private_directory_metadata_before_launch() {
    let fixture = Fixture::new();
    fs::set_permissions(fixture.0.join("journal"), fs::Permissions::from_mode(0o755)).unwrap();
    let error = fixture.directories().err().unwrap();
    assert_eq!(error.phase, AuthorityHostPhase::Prepare);
    assert!(matches!(
        error.reason,
        AuthorityHostReason::InvalidDirectory(_)
    ));
    assert!(!error.request_may_have_been_sent());
    assert!(!format!("{error:?}").contains(fixture.0.to_str().unwrap()));
    assert_eq!(fs::read_dir(fixture.0.join("keys")).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn host_does_not_accept_a_directory_or_text_file_as_its_executable() {
    let fixture = Fixture::new();
    let text = fixture.0.join("not-an-image");
    fs::write(&text, b"not executable image data").unwrap();
    fs::set_permissions(&text, fs::Permissions::from_mode(0o700)).unwrap();
    for path in [&fixture.0, &text] {
        let error = AuthorityHost::from_test_executable(File::open(path).unwrap())
            .err()
            .unwrap();
        assert_eq!(error.reason, AuthorityHostReason::InvalidExecutable);
        assert!(!error.request_may_have_been_sent());
    }
}

#[test]
fn responses_require_matching_resume_pin_and_confirmed_helper_exit() {
    let success = || AuthorityResponse::Ok {
        schema_version: 1,
        authority_fingerprint: [2; 32],
    };
    let initialize = AuthorityAction::Initialize {};
    let resume = AuthorityAction::Resume {
        authority_fingerprint: [1; 32],
    };
    let wrong_pin = accept_response(&resume, success(), ExitStatus::from_raw(0))
        .err()
        .unwrap();
    assert_eq!(
        wrong_pin.reason,
        AuthorityHostReason::ResumeFingerprintMismatch
    );
    assert!(wrong_pin.request_may_have_been_sent());
    let failed_exit = accept_response(&initialize, success(), ExitStatus::from_raw(1 << 8))
        .err()
        .unwrap();
    assert!(matches!(
        failed_exit.reason,
        AuthorityHostReason::HelperExit(_)
    ));
    assert!(failed_exit.request_may_have_been_sent());
    let refusal = || AuthorityResponse::Error {
        schema_version: 1,
        error: AuthorityAdmissionError::code(
            super::super::protocol::AuthorityAdmissionCode::AlreadyInitialized,
        ),
    };
    assert!(matches!(
        accept_response(&initialize, refusal(), ExitStatus::from_raw(0))
            .err()
            .unwrap()
            .reason,
        AuthorityHostReason::HelperExit(_)
    ));
    assert!(matches!(
        accept_response(&initialize, refusal(), ExitStatus::from_raw(1 << 8))
            .err()
            .unwrap()
            .reason,
        AuthorityHostReason::Admission(_)
    ));
}

#[test]
fn unknown_after_send_is_never_reclassified_as_not_submitted() {
    for phase in [
        AuthorityHostPhase::Transfer,
        AuthorityHostPhase::Outcome,
        AuthorityHostPhase::Exit,
    ] {
        let error = AuthorityHostFailure::new(phase, AuthorityHostReason::DeadlineElapsed);
        assert!(error.request_may_have_been_sent());
    }
    for phase in [
        AuthorityHostPhase::Prepare,
        AuthorityHostPhase::ProtectedReady,
    ] {
        let error = AuthorityHostFailure::new(phase, AuthorityHostReason::DeadlineElapsed);
        assert!(!error.request_may_have_been_sent());
    }
}

#[test]
fn private_reference_response_must_match_every_request_binding_and_exit() {
    use super::super::super::instance::{ProcessBirthIdentity, ProcessStart};
    use super::super::super::owner_protocol::OwnerCredential;
    let job_id = uuid::Uuid::new_v4();
    let owner = ProcessBirthIdentity {
        pid: 123,
        boot_id: uuid::Uuid::new_v4(),
        started: ProcessStart::LinuxTicks { ticks: 10 },
    };
    let credential = OwnerCredential {
        job_id,
        owner: owner.clone(),
        nonce: uuid::Uuid::new_v4(),
    };
    let action = AuthorityAction::PublishReference {
        authority_fingerprint: [1; 32],
        job_id,
        owner,
        binding_digest: [2; 32],
    };
    let response = || AuthorityResponse::Reference {
        schema_version: 1,
        authority_fingerprint: [1; 32],
        credential: credential.clone(),
        binding_digest: [2; 32],
    };
    assert!(accept_response(&action, response(), ExitStatus::from_raw(0)).is_ok());
    for field in ["authority", "job", "birth", "binding"] {
        let mut changed = response();
        let AuthorityResponse::Reference {
            authority_fingerprint,
            credential,
            binding_digest,
            ..
        } = &mut changed
        else {
            unreachable!();
        };
        match field {
            "authority" => authority_fingerprint[0] ^= 1,
            "job" => credential.job_id = uuid::Uuid::new_v4(),
            "birth" => credential.owner.boot_id = uuid::Uuid::new_v4(),
            "binding" => binding_digest[0] ^= 1,
            _ => unreachable!(),
        }
        let error = accept_response(&action, changed, ExitStatus::from_raw(0))
            .err()
            .unwrap();
        assert_eq!(error.reason, AuthorityHostReason::ReferenceBindingMismatch);
    }
    let error = accept_response(&action, response(), ExitStatus::from_raw(1 << 8))
        .err()
        .unwrap();
    assert!(matches!(error.reason, AuthorityHostReason::HelperExit(_)));
    let error = accept_response(
        &AuthorityAction::Initialize {},
        response(),
        ExitStatus::from_raw(0),
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, AuthorityHostReason::ResponseKindMismatch);
    let public = AuthorityResponse::Ok {
        schema_version: 1,
        authority_fingerprint: [1; 32],
    };
    let error = accept_response(&action, public, ExitStatus::from_raw(0))
        .err()
        .unwrap();
    assert_eq!(error.reason, AuthorityHostReason::ResponseKindMismatch);
}

#[test]
fn authenticated_outcome_reply_requires_exact_role_binding_and_successful_exit() {
    use super::super::super::instance::{ProcessBirthIdentity, ProcessStart};
    use super::super::super::owner_outcome::OwnerObservationRecord;
    use super::super::super::owner_reference::OwnerReference;
    let job_id = uuid::Uuid::new_v4();
    let owner = ProcessBirthIdentity {
        pid: 123,
        boot_id: uuid::Uuid::new_v4(),
        started: ProcessStart::LinuxTicks { ticks: 10 },
    };
    let record = OwnerReference::new(job_id, owner.clone(), [2; 32]).unwrap();
    let action = AuthorityAction::ReadOutcome {
        authority_fingerprint: [1; 32],
        credential: crate::local_process_owner::owner_protocol::OwnerCredential {
            job_id,
            owner: owner.clone(),
            nonce: uuid::Uuid::new_v4(),
        },
        binding_digest: [2; 32],
    };
    let response = || AuthorityResponse::Outcome {
        schema_version: 1,
        authority_fingerprint: [1; 32],
        reference: record.clone(),
        observation: OwnerObservationRecord::Pending {},
    };
    assert!(accept_response(&action, response(), ExitStatus::from_raw(0)).is_ok());
    for field in ["epoch", "job", "birth", "binding"] {
        let mut changed = response();
        let AuthorityResponse::Outcome {
            authority_fingerprint,
            reference,
            ..
        } = &mut changed
        else {
            unreachable!()
        };
        match field {
            "epoch" => authority_fingerprint[0] ^= 1,
            "job" => reference.job_id = uuid::Uuid::new_v4(),
            "birth" => reference.owner.boot_id = uuid::Uuid::new_v4(),
            "binding" => reference.binding_digest[0] ^= 1,
            _ => unreachable!(),
        }
        let error = accept_response(&action, changed, ExitStatus::from_raw(0))
            .err()
            .unwrap();
        assert_eq!(error.reason, AuthorityHostReason::OutcomeBindingMismatch);
    }
    let error = accept_response(&action, response(), ExitStatus::from_raw(1 << 8))
        .err()
        .unwrap();
    assert!(matches!(error.reason, AuthorityHostReason::HelperExit(_)));
    for wrong in [
        AuthorityAction::Initialize {},
        AuthorityAction::Resume {
            authority_fingerprint: [1; 32],
        },
        AuthorityAction::LoadReference {
            authority_fingerprint: [1; 32],
            job_id,
            owner: owner.clone(),
            binding_digest: [2; 32],
        },
        AuthorityAction::PublishReference {
            authority_fingerprint: [1; 32],
            job_id,
            owner,
            binding_digest: [2; 32],
        },
    ] {
        let error = accept_response(&wrong, response(), ExitStatus::from_raw(0))
            .err()
            .unwrap();
        assert_eq!(error.reason, AuthorityHostReason::ResponseKindMismatch);
    }
    let error = accept_response(
        &action,
        AuthorityResponse::Ok {
            schema_version: 1,
            authority_fingerprint: [1; 32],
        },
        ExitStatus::from_raw(0),
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, AuthorityHostReason::ResponseKindMismatch);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn unsupported_host_backend_does_not_attempt_a_linux_helper_path() {
    let error = AuthorityHost::current().err().unwrap();
    assert_eq!(error.reason, AuthorityHostReason::Unsupported);
    assert!(!error.request_may_have_been_sent());
}
