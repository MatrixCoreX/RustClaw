use super::*;

fn boot_id() -> Uuid {
    Uuid::parse_str("a28934f0-8b76-4dfd-92a5-246fa89cee14").unwrap()
}

fn stat(pid: u32, command: &[u8], parent: u32, group: u32, ticks: u64) -> Vec<u8> {
    let mut bytes = format!("{pid} (").into_bytes();
    bytes.extend_from_slice(command);
    bytes.extend_from_slice(b") S ");
    let mut fields = vec![parent.to_string(), group.to_string()];
    fields.extend((0..16).map(|_| "0".to_string()));
    fields.push(ticks.to_string());
    bytes.extend_from_slice(fields.join(" ").as_bytes());
    bytes
}

#[test]
fn native_instance_linux_stat_parses_machine_fields_not_command_text() {
    for command in [
        b"normal".as_slice(),
        b"with spaces ) ) (\nextra",
        b"non-utf8\xff",
    ] {
        let observed = parse_linux_stat(42, boot_id(), &stat(42, command, 9, 21, 12345)).unwrap();
        assert_eq!(
            observed.birth,
            ProcessBirthIdentity {
                pid: 42,
                boot_id: boot_id(),
                started: ProcessStart::LinuxTicks { ticks: 12345 },
            }
        );
        assert_eq!((observed.parent_pid, observed.process_group), (9, 21));
    }
}

#[test]
fn native_instance_process_state_preserves_live_and_zombie_distinction() {
    for (state, expected) in [
        (b'R', ProcessRunState::Active),
        (b'S', ProcessRunState::Active),
        (b'D', ProcessRunState::Active),
        (b'T', ProcessRunState::Active),
        (b't', ProcessRunState::Active),
        (b'Z', ProcessRunState::Zombie),
        (b'X', ProcessRunState::Zombie),
        (b'x', ProcessRunState::Zombie),
    ] {
        let mut raw = stat(42, b"state", 9, 21, 123);
        let index = raw.iter().rposition(|byte| *byte == b')').unwrap() + 2;
        raw[index] = state;
        assert_eq!(
            parse_linux_stat(42, boot_id(), &raw).unwrap().state,
            expected
        );
    }
}

#[test]
fn native_instance_reparenting_and_group_changes_do_not_change_birth_identity() {
    let before = parse_linux_stat(42, boot_id(), &stat(42, b"same", 9, 21, 123)).unwrap();
    let after = parse_linux_stat(42, boot_id(), &stat(42, b"exec changed", 1, 42, 123)).unwrap();
    assert_eq!(before.birth, after.birth);
    assert_ne!(before.parent_pid, after.parent_pid);
    assert_ne!(before.process_group, after.process_group);
    let reused = parse_linux_stat(42, boot_id(), &stat(42, b"same", 1, 42, 124)).unwrap();
    assert_ne!(before.birth, reused.birth);
}

#[test]
fn native_instance_linux_stat_rejects_malformed_or_wrong_process_records() {
    let mut oversized = stat(42, b"same", 9, 21, 123);
    oversized.extend(vec![b' '; MAX_STAT_BYTES]);
    for raw in [
        b"".to_vec(),
        b"42 missing".to_vec(),
        b"42 (missing) S 1 2".to_vec(),
        stat(41, b"wrong-pid", 9, 21, 123),
        stat(42, b"bad-group", 9, u32::MAX, 123),
        stat(42, b"bad-parent", u32::MAX, 21, 123),
        oversized,
    ] {
        assert_eq!(
            parse_linux_stat(42, boot_id(), &raw),
            Err(ProcessProbeFailure::Malformed)
        );
    }
    for number in ["-1", "+1", "18446744073709551616", "1x"] {
        let mut raw = stat(42, b"bad-ticks", 9, 21, 123);
        raw.truncate(raw.len() - 3);
        raw.extend_from_slice(number.as_bytes());
        assert_eq!(
            parse_linux_stat(42, boot_id(), &raw),
            Err(ProcessProbeFailure::Malformed)
        );
    }
}

#[test]
fn native_instance_hidden_group_does_not_discard_visible_birth_identity() {
    let visible = parse_linux_stat(42, boot_id(), &stat(42, b"same", 9, 21, 123)).unwrap();
    let hidden = parse_linux_stat(42, boot_id(), &stat(42, b"same", 0, 0, 123)).unwrap();
    assert_eq!(visible.birth, hidden.birth);
    assert_eq!((hidden.parent_pid, hidden.process_group), (0, 0));
    assert!(!valid_target(hidden.process_group));
}

#[test]
fn native_instance_boot_identity_is_canonical_bounded_and_non_nil() {
    for raw in [
        b"a28934f0-8b76-4dfd-92a5-246fa89cee14\n".as_slice(),
        b"A28934F0-8B76-4DFD-92A5-246FA89CEE14\0",
    ] {
        assert_eq!(parse_boot_id(raw), Ok(boot_id()));
    }
    for raw in [
        b"".as_slice(),
        b"00000000-0000-0000-0000-000000000000",
        b"a28934f08b764dfd92a5246fa89cee14",
        b"a28934f0-8b76-4dfd-92a5-246fa89cee14\0\0",
    ] {
        assert_eq!(parse_boot_id(raw), Err(ProcessProbeFailure::Malformed));
    }
}

#[test]
fn native_instance_birth_records_round_trip_both_platforms() {
    for started in [
        ProcessStart::LinuxTicks { ticks: u64::MAX },
        ProcessStart::MacOsMicroseconds {
            seconds: 1780000000,
            microseconds: 999999,
        },
    ] {
        let birth = ProcessBirthIdentity {
            pid: 42,
            boot_id: boot_id(),
            started,
        };
        assert_eq!(
            decode_birth_record(&encode_birth_record(&birth).unwrap()),
            Ok(birth)
        );
    }
}

#[test]
fn native_instance_birth_record_rejects_unknown_contracts_and_invalid_values() {
    let birth = ProcessBirthIdentity {
        pid: 42,
        boot_id: boot_id(),
        started: ProcessStart::LinuxTicks { ticks: 123 },
    };
    let original: serde_json::Value =
        serde_json::from_slice(&encode_birth_record(&birth).unwrap()).unwrap();
    let mut future = original.clone();
    future["schema_version"] = 2.into();
    assert_eq!(
        decode_birth_record(&serde_json::to_vec(&future).unwrap()),
        Err(ProcessProbeFailure::Unsupported)
    );
    for (pointer, value) in [
        ("/birth/pid", serde_json::json!(0)),
        ("/birth/pid", serde_json::json!(1)),
        ("/birth/pid", serde_json::json!(2147483648_u64)),
        ("/birth/boot_id", serde_json::json!(Uuid::nil())),
        (
            "/birth/started",
            serde_json::json!({"kind":"mac_os_microseconds","seconds":1,"microseconds":1000000}),
        ),
        (
            "/birth/started",
            serde_json::json!({"kind":"future","ticks":1}),
        ),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(
            decode_birth_record(&serde_json::to_vec(&changed).unwrap()).is_err(),
            "pointer={pointer}"
        );
    }
    for pointer in ["", "/birth", "/birth/started"] {
        let mut changed = original.clone();
        changed
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unrecognized".into(), true.into());
        assert!(decode_birth_record(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    assert_eq!(
        decode_birth_record(&vec![b' '; MAX_BIRTH_RECORD_BYTES + 1]),
        Err(ProcessProbeFailure::Malformed)
    );
}

#[test]
fn native_instance_invalid_targets_do_not_probe_the_host() {
    for pid in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
        assert_eq!(
            observe_process(pid),
            Err(ProcessProbeFailure::InvalidTarget)
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn native_instance_current_process_is_stable_and_machine_only() {
    let first = observe_process(std::process::id()).unwrap();
    let second = observe_process(std::process::id()).unwrap();
    assert_eq!(first.birth, second.birth);
    let bytes = encode_birth_record(&first.birth).unwrap();
    assert_eq!(decode_birth_record(&bytes), Ok(first.birth));
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 2);
    assert!(value["birth"].get("command").is_none());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn native_instance_owned_child_has_distinct_identity_and_group() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut child = Command::new("/bin/sh")
        .args(["-c", "read -r value"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let observed = observe_process(child.id());
    drop(input);
    child.wait().unwrap();
    let observed = observed.unwrap();
    assert_eq!(observed.birth.pid, child.id());
    assert_eq!(observed.parent_pid, std::process::id());
    assert_eq!(observed.process_group, child.id());
    assert_ne!(
        observed.birth,
        observe_process(std::process::id()).unwrap().birth
    );
}

#[test]
fn native_instance_read_errors_are_not_process_identity_proof() {
    assert_eq!(
        map_read_error(std::io::Error::from(std::io::ErrorKind::NotFound)),
        ProcessProbeFailure::Missing
    );
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::Other,
    ] {
        assert_eq!(
            map_read_error(std::io::Error::from(kind)),
            ProcessProbeFailure::Unavailable
        );
    }
}

#[test]
fn native_instance_file_records_are_bounded_and_missing_is_explicit() {
    let root = RecordFixture::new();
    let path = root.0.join("record");
    assert_eq!(read_bounded(&path, 8), Err(ProcessProbeFailure::Missing));
    std::fs::write(&path, b"12345678").unwrap();
    assert_eq!(read_bounded(&path, 8).unwrap(), b"12345678");
    assert_eq!(read_bounded(&path, 7), Err(ProcessProbeFailure::Malformed));
    assert_eq!(read_bounded(&path, 0), Err(ProcessProbeFailure::Malformed));
    std::fs::write(&path, b"").unwrap();
    assert!(read_bounded(&path, 0).unwrap().is_empty());
}

struct RecordFixture(std::path::PathBuf);

impl RecordFixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-process-birth-record-{}",
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for RecordFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
