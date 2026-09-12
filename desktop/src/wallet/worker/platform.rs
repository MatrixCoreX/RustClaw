//! Platform launch plumbing. A failed protection setup never opens a vault.
use crate::Result;
use std::{
    fs::File,
    path::Path,
    process::{Child, Command, Stdio},
};

pub struct Worker {
    pub child: Child,
    pub input: File,
    pub output: File,
    #[cfg(windows)]
    pub guard: super::windows_job::Job,
}

pub fn spawn(exe: &Path) -> Result<Worker> {
    let mut command = Command::new(exe);
    command
        .arg("--asset-vault-worker")
        .env_clear()
        .stderr(Stdio::null());
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "TMPDIR",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, OwnedFd};
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut child = command.spawn().map_err(|_| "wallet_worker_unavailable")?;
        let input = File::from(OwnedFd::from(child.stdin.take().unwrap()));
        let output = File::from(OwnedFd::from(child.stdout.take().unwrap()));
        let setup = super::transport::nonblocking(input.as_raw_fd())
            .and_then(|()| super::transport::nonblocking(output.as_raw_fd()));
        if let Err(e) = setup {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        Ok(Worker {
            child,
            input,
            output,
        })
    }
    #[cfg(windows)]
    {
        super::windows_job::spawn(command)
    }
}

pub fn stdio() -> Result<(File, File)> {
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        // SAFETY: worker mode exclusively owns unbuffered standard descriptors.
        unsafe { Ok((File::from_raw_fd(0), File::from_raw_fd(1))) }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::{
            Foundation::INVALID_HANDLE_VALUE,
            System::Console::{GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
        };
        // SAFETY: standard handles are exclusively owned in worker mode.
        unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            if input.is_null()
                || output.is_null()
                || input == INVALID_HANDLE_VALUE
                || output == INVALID_HANDLE_VALUE
            {
                return Err("wallet_worker_unavailable".into());
            }
            Ok((File::from_raw_handle(input), File::from_raw_handle(output)))
        }
    }
}
