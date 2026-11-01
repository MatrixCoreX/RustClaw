#[cfg(any(target_os = "linux", test))]
use std::{io::Read, path::Path};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(any(target_os = "linux", test))]
const MAX_STAT_BYTES: usize = 64 * 1024;
#[cfg(test)]
const MAX_BIRTH_RECORD_BYTES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ProcessStart {
    LinuxTicks { ticks: u64 },
    MacOsMicroseconds { seconds: u64, microseconds: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessBirthIdentity {
    pub(crate) pid: u32,
    pub(crate) boot_id: Uuid,
    pub(crate) started: ProcessStart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessSnapshot {
    pub(crate) birth: ProcessBirthIdentity,
    pub(crate) parent_pid: u32,
    // Zero means the group is outside this PID namespace, never a signal target.
    pub(crate) process_group: u32,
    pub(crate) state: ProcessRunState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessRunState {
    Active,
    Zombie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessProbeFailure {
    InvalidTarget,
    Missing,
    Unavailable,
    Malformed,
    #[cfg(any(test, not(target_os = "linux")))]
    Unsupported,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BirthRecord {
    schema_version: u32,
    birth: ProcessBirthIdentity,
}

fn valid_target(pid: u32) -> bool {
    pid > 1 && i32::try_from(pid).is_ok()
}

fn validate_birth(birth: &ProcessBirthIdentity) -> Result<(), ProcessProbeFailure> {
    if !valid_target(birth.pid)
        || birth.boot_id.is_nil()
        || matches!(birth.started, ProcessStart::MacOsMicroseconds { microseconds, .. } if microseconds >= 1_000_000)
    {
        return Err(ProcessProbeFailure::Malformed);
    }
    Ok(())
}

pub(crate) fn encode_birth_record(
    birth: &ProcessBirthIdentity,
) -> Result<Vec<u8>, ProcessProbeFailure> {
    validate_birth(birth)?;
    serde_json::to_vec(&BirthRecord {
        schema_version: 1,
        birth: birth.clone(),
    })
    .map_err(|_| ProcessProbeFailure::Malformed)
}

#[cfg(test)]
pub(crate) fn decode_birth_record(
    bytes: &[u8],
) -> Result<ProcessBirthIdentity, ProcessProbeFailure> {
    if bytes.len() > MAX_BIRTH_RECORD_BYTES {
        return Err(ProcessProbeFailure::Malformed);
    }
    let record: BirthRecord =
        serde_json::from_slice(bytes).map_err(|_| ProcessProbeFailure::Malformed)?;
    if record.schema_version != 1 {
        return Err(ProcessProbeFailure::Unsupported);
    }
    validate_birth(&record.birth)?;
    Ok(record.birth)
}

fn parse_boot_id(raw: &[u8]) -> Result<Uuid, ProcessProbeFailure> {
    let raw = raw.strip_suffix(b"\0").unwrap_or(raw);
    let value = std::str::from_utf8(raw)
        .map_err(|_| ProcessProbeFailure::Malformed)?
        .trim_matches(|character: char| character.is_ascii_whitespace());
    if value.len() != 36 {
        return Err(ProcessProbeFailure::Malformed);
    }
    Uuid::parse_str(value)
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or(ProcessProbeFailure::Malformed)
}

#[cfg(any(target_os = "linux", test))]
fn map_read_error(error: std::io::Error) -> ProcessProbeFailure {
    match error.kind() {
        std::io::ErrorKind::NotFound => ProcessProbeFailure::Missing,
        _ => ProcessProbeFailure::Unavailable,
    }
}

#[cfg(any(target_os = "linux", test))]
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, ProcessProbeFailure> {
    let file = std::fs::File::open(path).map_err(map_read_error)?;
    let mut bytes = Vec::new();
    file.take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(map_read_error)?;
    if bytes.len() > limit {
        return Err(ProcessProbeFailure::Malformed);
    }
    Ok(bytes)
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_stat(
    requested_pid: u32,
    boot_id: Uuid,
    raw: &[u8],
) -> Result<ProcessSnapshot, ProcessProbeFailure> {
    let invalid = ProcessProbeFailure::Malformed;
    if raw.len() > MAX_STAT_BYTES || !valid_target(requested_pid) || boot_id.is_nil() {
        return Err(invalid);
    }
    let open = raw.iter().position(|byte| *byte == b'(').ok_or(invalid)?;
    let close = raw.iter().rposition(|byte| *byte == b')').ok_or(invalid)?;
    if close <= open || raw.get(close + 1) != Some(&b' ') {
        return Err(invalid);
    }
    let recorded_pid = std::str::from_utf8(&raw[..open])
        .map_err(|_| invalid)?
        .trim()
        .parse::<u32>()
        .map_err(|_| invalid)?;
    if recorded_pid != requested_pid {
        return Err(invalid);
    }
    let fields: Vec<_> = raw[close + 1..]
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty())
        .collect();
    let unsigned = |index: usize| -> Result<u64, ProcessProbeFailure> {
        let field = fields
            .get(index)
            .filter(|field| !field.is_empty() && field.iter().all(u8::is_ascii_digit))
            .ok_or(invalid)?;
        std::str::from_utf8(field)
            .map_err(|_| invalid)?
            .parse()
            .map_err(|_| invalid)
    };
    let state = match fields.first().copied() {
        Some(b"Z" | b"X" | b"x") => ProcessRunState::Zombie,
        Some([value]) if value.is_ascii_alphabetic() => ProcessRunState::Active,
        _ => return Err(invalid),
    };
    let parent_pid = u32::try_from(unsigned(1)?).map_err(|_| invalid)?;
    let process_group = u32::try_from(unsigned(2)?).map_err(|_| invalid)?;
    if parent_pid > i32::MAX as u32 || process_group > i32::MAX as u32 {
        return Err(invalid);
    }
    Ok(ProcessSnapshot {
        birth: ProcessBirthIdentity {
            pid: requested_pid,
            boot_id,
            started: ProcessStart::LinuxTicks {
                ticks: unsigned(19)?,
            },
        },
        parent_pid,
        process_group,
        state,
    })
}

pub(crate) fn observe_process(pid: u32) -> Result<ProcessSnapshot, ProcessProbeFailure> {
    if !valid_target(pid) {
        return Err(ProcessProbeFailure::InvalidTarget);
    }
    #[cfg(target_os = "linux")]
    {
        let boot_id = parse_boot_id(&read_bounded(
            Path::new("/proc/sys/kernel/random/boot_id"),
            64,
        )?)?;
        let stat = read_bounded(Path::new(&format!("/proc/{pid}/stat")), MAX_STAT_BYTES)?;
        parse_linux_stat(pid, boot_id, &stat)
    }
    #[cfg(target_os = "macos")]
    {
        observe_macos_process(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(ProcessProbeFailure::Unsupported)
    }
}

#[cfg(target_os = "macos")]
fn observe_macos_process(pid: u32) -> Result<ProcessSnapshot, ProcessProbeFailure> {
    let mut boot = [0_u8; 64];
    let mut boot_len = boot.len();
    // Both calls use public SDK types; never substitute a private flavor/ABI.
    let boot_result = unsafe {
        libc::sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            boot.as_mut_ptr().cast(),
            &mut boot_len,
            std::ptr::null_mut(),
            0,
        )
    };
    if boot_result != 0 {
        return Err(ProcessProbeFailure::Unavailable);
    }
    let boot_id = parse_boot_id(boot.get(..boot_len).ok_or(ProcessProbeFailure::Malformed)?)?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let expected_size = std::mem::size_of::<libc::proc_bsdinfo>();
    let size = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            expected_size as i32,
        )
    };
    if size == 0 {
        return Err(
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                ProcessProbeFailure::Missing
            } else {
                ProcessProbeFailure::Unavailable
            },
        );
    }
    if size as usize != expected_size {
        return Err(ProcessProbeFailure::Malformed);
    }
    // proc_pidinfo initialized the complete public struct at the checked size.
    let info = unsafe { info.assume_init() };
    let birth = ProcessBirthIdentity {
        pid,
        boot_id,
        started: ProcessStart::MacOsMicroseconds {
            seconds: info.pbi_start_tvsec,
            microseconds: u32::try_from(info.pbi_start_tvusec)
                .map_err(|_| ProcessProbeFailure::Malformed)?,
        },
    };
    validate_birth(&birth)?;
    if info.pbi_pid != pid
        || info.pbi_ppid > i32::MAX as u32
        || info.pbi_pgid == 0
        || info.pbi_pgid > i32::MAX as u32
    {
        return Err(ProcessProbeFailure::Malformed);
    }
    Ok(ProcessSnapshot {
        birth,
        parent_pid: info.pbi_ppid,
        process_group: info.pbi_pgid,
        state: if info.pbi_status == libc::SZOMB {
            ProcessRunState::Zombie
        } else {
            ProcessRunState::Active
        },
    })
}

#[cfg(test)]
#[path = "local_process_instance_tests.rs"]
mod tests;
