use crate::{wallet::windows_security, Result};
use std::{
    fs::File,
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*, Security::SECURITY_ATTRIBUTES, Storage::FileSystem::*, System::Pipes::*,
};

pub fn pair() -> Result<(File, File)> {
    let name: Vec<u16> = format!(r"\\.\pipe\agent-runtime-wallet-{}", uuid::Uuid::new_v4())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let descriptor = windows_security::private_descriptor(false)?;
    let security = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: fresh random name, first-instance guard, one local client, user-only
    // ACL. Both ends connect before spawning; no name or credential is sent out.
    unsafe {
        let server = CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            262144,
            262144,
            0,
            &security,
        );
        if server == INVALID_HANDLE_VALUE {
            return Err("wallet_worker_unavailable".into());
        }
        let server = File::from_raw_handle(server);
        let client = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        );
        if client == INVALID_HANDLE_VALUE {
            return Err("wallet_worker_unavailable".into());
        }
        let client = File::from_raw_handle(client);
        if ConnectNamedPipe(server.as_raw_handle(), null_mut()) == 0
            && GetLastError() != ERROR_PIPE_CONNECTED
        {
            return Err("wallet_worker_unavailable".into());
        }
        nonblocking(client.as_raw_handle())?;
        Ok((server, client))
    }
}
pub fn nonblocking(handle: HANDLE) -> Result<()> {
    let mode = PIPE_READMODE_BYTE | PIPE_NOWAIT;
    if unsafe { SetNamedPipeHandleState(handle, &mode, null(), null()) } == 0 {
        return Err("wallet_worker_unavailable".into());
    }
    Ok(())
}
pub fn ready(handle: HANDLE, write: bool, timeout: Duration) -> Result<bool> {
    // Deliberate synchronous polling, not an overlapped-I/O emulation. NOWAIT
    // bounds WriteFile too; partial writes are retried under the frame deadline.
    if write {
        return Ok(true);
    }
    let end = Instant::now() + timeout;
    loop {
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                handle,
                null_mut(),
                0,
                null_mut(),
                &mut available,
                null_mut(),
            )
        } == 0
        {
            return Err("wallet_worker_unavailable".into());
        }
        if available > 0 {
            return Ok(true);
        }
        let Some(left) = end.checked_duration_since(Instant::now()) else {
            return Ok(false);
        };
        std::thread::sleep(left.min(Duration::from_millis(5)));
    }
}
