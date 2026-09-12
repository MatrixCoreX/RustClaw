use super::{platform::Worker, windows_pipe};
use crate::Result;
use std::{
    mem::{size_of, zeroed},
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::{Command, Stdio},
    ptr::null,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{JobObjects::*, Threading::CREATE_NO_WINDOW},
};

pub struct Job(OwnedHandle);
pub fn spawn(mut command: Command) -> Result<Worker> {
    let (parent, child_pipe) = windows_pipe::pair()?;
    let input = parent
        .try_clone()
        .map_err(|_| "wallet_worker_unavailable")?;
    let child_output = child_pipe
        .try_clone()
        .map_err(|_| "wallet_worker_unavailable")?;
    let guard = create()?;
    command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::from(child_pipe))
        .stdout(Stdio::from(child_output));
    let mut child = command.spawn().map_err(|_| "wallet_worker_unavailable")?;
    // The worker waits for these exact job limits before accepting Open. No
    // secrets have crossed the pipe when it is assigned to the non-inherited job.
    if unsafe { AssignProcessToJobObject(guard.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("wallet_process_protection_unavailable".into());
    }
    Ok(Worker {
        child,
        input,
        output: parent,
        guard,
    })
}
fn create() -> Result<Job> {
    unsafe {
        let handle = CreateJobObjectW(null(), null());
        if handle.is_null() {
            return Err("wallet_process_protection_unavailable".into());
        }
        let job = Job(OwnedHandle::from_raw_handle(handle));
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        info.BasicLimitInformation.ActiveProcessLimit = 1;
        if SetInformationJobObject(
            job.0.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            return Err("wallet_process_protection_unavailable".into());
        }
        Ok(job)
    }
}
