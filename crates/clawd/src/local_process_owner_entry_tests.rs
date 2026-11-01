use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::*;

#[test]
fn descriptor_isolation_diagnostics_use_only_the_machine_code_and_optional_errno() {
    assert_eq!(
        EntryFailure::DescriptorIsolation { os_error: Some(38) }.to_string(),
        "process_owner_descriptor_isolation:Some(38)"
    );
    assert_eq!(
        EntryFailure::DescriptorIsolation { os_error: None }.to_string(),
        "process_owner_descriptor_isolation:None"
    );
}

#[cfg(target_os = "linux")]
#[path = "local_process_owner_entry_process_tests.rs"]
pub(in crate::local_process_owner) mod process;

pub(in crate::local_process_owner) fn executable() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("APP_OWNER_ENTRY_TEST_BINARY").expect("source-pinned executable required"),
    );
    let metadata = std::fs::symlink_metadata(&path).unwrap();
    assert!(path.is_absolute() && metadata.is_file() && metadata.permissions().mode() & 0o111 != 0);
    path
}

#[test]
fn helper_mode_is_an_exact_machine_invocation_not_a_normal_startup_fallback() {
    assert!(!helper_selected(Vec::<OsString>::new()).unwrap());
    assert!(!helper_selected(["--offline-bundled-bootstrap".into()]).unwrap());
    assert!(!helper_selected(["--process-owner-extra".into()]).unwrap());
    assert!(helper_selected([HELPER_FLAG.into()]).unwrap());
    assert!(matches!(
        helper_selected([HELPER_FLAG.into(), "extra".into()]),
        Err(EntryFailure::InvalidInvocation)
    ));
}

fn bootstrap() -> OwnerBootstrap {
    OwnerBootstrap {
        schema_version: 1,
        job_id: Uuid::new_v4(),
        authority_fingerprint: [1; 32],
        binding_digest: [2; 32],
        terminate_grace_millis: 100,
    }
}

#[test]
fn bootstrap_rejects_unknown_fields_invalid_identity_and_oversize_body() {
    let valid = serde_json::to_value(bootstrap()).unwrap();
    assert!(OwnerBootstrap::decode(&serde_json::to_vec(&valid).unwrap()).is_ok());
    for (key, value) in [
        ("schema_version", serde_json::json!(2)),
        ("job_id", serde_json::json!(Uuid::nil())),
        (
            "authority_fingerprint",
            serde_json::to_value([0_u8; 32]).unwrap(),
        ),
        ("binding_digest", serde_json::to_value([0_u8; 32]).unwrap()),
        ("terminate_grace_millis", serde_json::json!(-1)),
        ("command", serde_json::json!("not_admitted")),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = value;
        assert!(matches!(
            OwnerBootstrap::decode(&serde_json::to_vec(&invalid).unwrap()),
            Err(EntryFailure::InvalidBootstrap)
        ));
    }
    assert!(matches!(
        OwnerBootstrap::decode(&vec![b' '; 4097]),
        Err(EntryFailure::InvalidBootstrap)
    ));
}

#[test]
fn inherited_socket_roles_are_unix_typed_and_validation_does_not_modify_flags() {
    let (stream, _) = UnixStream::pair().unwrap();
    let datagram = UnixDatagram::unbound().unwrap();
    let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) };
    validate_unix_socket(stream.as_raw_fd(), libc::SOCK_STREAM).unwrap();
    validate_unix_socket(datagram.as_raw_fd(), libc::SOCK_DGRAM).unwrap();
    assert!(matches!(
        validate_unix_socket(stream.as_raw_fd(), libc::SOCK_DGRAM),
        Err(EntryFailure::InvalidDescriptor)
    ));
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    assert!(matches!(
        validate_unix_socket(tcp.as_raw_fd(), libc::SOCK_STREAM),
        Err(EntryFailure::InvalidDescriptor)
    ));
    assert_eq!(
        unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) },
        flags
    );
}

#[test]
fn entry_diagnostics_have_only_machine_codes_without_bootstrap_material() {
    assert_eq!(
        EntryFailure::InvalidBootstrap.to_string(),
        "process_owner_invalid_bootstrap"
    );
    assert_eq!(
        EntryFailure::Owner(OwnerFailure::CredentialMismatch).to_string(),
        "process_owner_runtime:CredentialMismatch"
    );
}

#[test]
#[ignore = "Requires the source-pinned non-test executable; selected explicitly by native acceptance"]
fn entry_executable_protocol_acceptance() {
    #[cfg(target_os = "linux")]
    process::run();
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new(executable())
            .arg(HELPER_FLAG)
            .env_clear()
            .current_dir(std::env::temp_dir())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("process_owner_privacy:Unsupported")
        );
        println!(
            "OWNER_ENTRY_CASE case=mac_native_scope_unsupported actual_executable=true llm_calls=0"
        );
    }
}
