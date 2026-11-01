use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::process_invocation::ProcessInvocation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchFailure {
    InvalidProgram,
    InvalidArgument,
    InvalidWorkingDirectory,
    InvalidEnvironment,
    InvalidStdio,
}

impl LaunchFailure {
    pub(crate) fn error_code(self) -> &'static str {
        match self {
            Self::InvalidProgram => "process_launch_invalid_program",
            Self::InvalidArgument => "process_launch_invalid_argument",
            Self::InvalidWorkingDirectory => "process_launch_invalid_working_directory",
            Self::InvalidEnvironment => "process_launch_invalid_environment",
            Self::InvalidStdio => "process_launch_invalid_stdio",
        }
    }
}

// These are data handles, never the helper's own control stdin/stdout. None is
// explicitly null, not implicit inheritance. Do not add Debug/serialization.
pub(crate) struct PayloadIo {
    pub(crate) stdin: Option<File>,
    pub(crate) stdout: Option<File>,
    pub(crate) stderr: Option<File>,
}

pub(crate) struct ProcessLaunch {
    invocation: ProcessInvocation,
    cwd: File,
    environment: BTreeMap<OsString, OsString>,
    io: PayloadIo,
}

pub(crate) fn open_working_directory(cwd: &Path) -> Result<File, LaunchFailure> {
    if !cwd.is_absolute() || contains_nul(cwd.as_os_str()) {
        return Err(LaunchFailure::InvalidWorkingDirectory);
    }
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(cwd)
        .map_err(|_| LaunchFailure::InvalidWorkingDirectory)
}

impl ProcessLaunch {
    pub(crate) fn new(
        invocation: ProcessInvocation,
        cwd: PathBuf,
        environment: Vec<(OsString, OsString)>,
        io: PayloadIo,
    ) -> Result<Self, LaunchFailure> {
        let cwd = open_working_directory(&cwd)?;
        Self::with_working_directory(invocation, cwd, environment, io)
    }

    // The private handoff must transfer this admitted directory descriptor,
    // not reopen a path after validation or recover state from Command getters.
    pub(crate) fn with_working_directory(
        invocation: ProcessInvocation,
        cwd: File,
        environment: Vec<(OsString, OsString)>,
        io: PayloadIo,
    ) -> Result<Self, LaunchFailure> {
        if !cwd
            .metadata()
            .map_err(|_| LaunchFailure::InvalidWorkingDirectory)?
            .is_dir()
        {
            return Err(LaunchFailure::InvalidWorkingDirectory);
        }
        let flags = unsafe { libc::fcntl(cwd.as_raw_fd(), libc::F_GETFD) };
        if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
            return Err(LaunchFailure::InvalidWorkingDirectory);
        }
        let (program, args) = invocation.into_parts();
        if program.is_empty() || contains_nul(&program) {
            return Err(LaunchFailure::InvalidProgram);
        }
        if args.iter().any(|arg| contains_nul(arg)) {
            return Err(LaunchFailure::InvalidArgument);
        }
        let mut effective_environment = BTreeMap::new();
        for (key, value) in environment {
            if key.is_empty()
                || contains_nul(&key)
                || key.as_bytes().contains(&b'=')
                || contains_nul(&value)
                || effective_environment.insert(key, value).is_some()
            {
                return Err(LaunchFailure::InvalidEnvironment);
            }
        }
        validate_handle(io.stdin.as_ref(), false)?;
        validate_handle(io.stdout.as_ref(), true)?;
        validate_handle(io.stderr.as_ref(), true)?;
        let mut invocation = ProcessInvocation::new(program);
        for arg in args {
            invocation.arg(arg);
        }
        Ok(Self {
            invocation,
            cwd,
            environment: effective_environment,
            io,
        })
    }

    pub(crate) fn into_command(self) -> Command {
        let mut command = self.invocation.into_command();
        command
            .env_clear()
            .envs(self.environment)
            .stdin(data_stdio(self.io.stdin))
            .stdout(data_stdio(self.io.stdout))
            .stderr(data_stdio(self.io.stderr));
        let cwd = self.cwd;
        // fchdir is async-signal-safe; keep the descriptor owned by the hook
        // until exec, where CLOEXEC closes it. Never allocate/log in this hook.
        unsafe {
            command.pre_exec(move || {
                if libc::fchdir(cwd.as_raw_fd()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ProcessInvocation,
        File,
        Vec<(OsString, OsString)>,
        PayloadIo,
    ) {
        (
            self.invocation,
            self.cwd,
            self.environment.into_iter().collect(),
            self.io,
        )
    }
}

fn contains_nul(value: &std::ffi::OsStr) -> bool {
    value.as_bytes().contains(&0)
}

fn data_stdio(file: Option<File>) -> Stdio {
    file.map(Stdio::from).unwrap_or_else(Stdio::null)
}

fn validate_handle(file: Option<&File>, write: bool) -> Result<(), LaunchFailure> {
    let Some(file) = file else {
        return Ok(());
    };
    if file
        .metadata()
        .map_err(|_| LaunchFailure::InvalidStdio)?
        .is_dir()
    {
        return Err(LaunchFailure::InvalidStdio);
    }
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(LaunchFailure::InvalidStdio);
    }
    #[cfg(target_os = "linux")]
    if flags & libc::O_PATH != 0 {
        return Err(LaunchFailure::InvalidStdio);
    }
    let mode = flags & libc::O_ACCMODE;
    if (write && mode == libc::O_RDONLY) || (!write && mode == libc::O_WRONLY) {
        return Err(LaunchFailure::InvalidStdio);
    }
    Ok(())
}

#[cfg(test)]
#[path = "process_launch_tests.rs"]
mod tests;
