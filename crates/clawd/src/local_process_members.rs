use super::instance::{observe_process, ProcessProbeFailure, ProcessSnapshot};

const MAX_GROUP_MEMBERS: usize = 16_384;

pub(crate) fn group_members(group: u32) -> Result<Vec<ProcessSnapshot>, ProcessProbeFailure> {
    let candidates = group_candidates(group)?;
    observe_group_candidates(group, &candidates)
}

// Candidate enumeration and process observation are separate snapshots, not
// an atomic completion barrier. New children can appear between these calls.
pub(super) fn group_candidates(group: u32) -> Result<Vec<u32>, ProcessProbeFailure> {
    validate_group(group)?;
    platform_group_pids(group)
}

fn validate_group(group: u32) -> Result<(), ProcessProbeFailure> {
    if group <= 1 || group > i32::MAX as u32 {
        return Err(ProcessProbeFailure::InvalidTarget);
    }
    Ok(())
}

pub(super) fn observe_group_candidates(
    group: u32,
    candidates: &[u32],
) -> Result<Vec<ProcessSnapshot>, ProcessProbeFailure> {
    validate_group(group)?;
    if candidates.len() > MAX_GROUP_MEMBERS {
        return Err(ProcessProbeFailure::Unavailable);
    }
    let mut members = Vec::new();
    for &pid in candidates {
        if pid <= 1 || pid > i32::MAX as u32 {
            return Err(ProcessProbeFailure::InvalidTarget);
        }
        match observe_process(pid) {
            Ok(snapshot) if snapshot.process_group == group => {
                if members.len() == MAX_GROUP_MEMBERS {
                    return Err(ProcessProbeFailure::Unavailable);
                }
                members.push(snapshot);
            }
            Ok(_) | Err(ProcessProbeFailure::Missing) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(members)
}

#[cfg(target_os = "linux")]
fn platform_group_pids(group: u32) -> Result<Vec<u32>, ProcessProbeFailure> {
    let entries = std::fs::read_dir("/proc").map_err(|_| ProcessProbeFailure::Unavailable)?;
    let mut members = Vec::new();
    let mut scanned = 0_usize;
    for entry in entries {
        let entry = entry.map_err(|_| ProcessProbeFailure::Unavailable)?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid <= 1 || pid > i32::MAX as u32 {
            continue;
        }
        scanned += 1;
        if scanned > 131_072 {
            return Err(ProcessProbeFailure::Unavailable);
        }
        // The kernel supplies membership; never infer it from command text.
        let observed_group = unsafe { libc::getpgid(pid as i32) };
        if observed_group < 0 {
            match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ESRCH | libc::ENOENT) => continue,
                _ => return Err(ProcessProbeFailure::Unavailable),
            }
        }
        if observed_group as u32 == group {
            if members.len() == MAX_GROUP_MEMBERS {
                return Err(ProcessProbeFailure::Unavailable);
            }
            members.push(pid);
        }
    }
    Ok(members)
}

#[cfg(target_os = "macos")]
fn platform_group_pids(group: u32) -> Result<Vec<u32>, ProcessProbeFailure> {
    let mut pids = vec![0_i32; MAX_GROUP_MEMBERS + 1];
    // The public libproc wrapper returns a PID count, not a byte count.
    // Its underlying wrapper can report an error as zero, with errno set.
    let (count, error) = unsafe {
        *libc::__error() = 0;
        let count = libc::proc_listpgrppids(
            group as i32,
            pids.as_mut_ptr().cast(),
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        );
        (count, *libc::__error())
    };
    if count < 0 || error != 0 || count as usize >= pids.len() {
        return Err(ProcessProbeFailure::Unavailable);
    }
    pids.truncate(count as usize);
    pids.into_iter()
        .map(|pid| {
            u32::try_from(pid)
                .ok()
                .filter(|pid| *pid > 1)
                .ok_or(ProcessProbeFailure::Malformed)
        })
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_group_pids(_group: u32) -> Result<Vec<u32>, ProcessProbeFailure> {
    Err(ProcessProbeFailure::Unsupported)
}

#[cfg(test)]
#[path = "local_process_members_tests.rs"]
mod tests;
