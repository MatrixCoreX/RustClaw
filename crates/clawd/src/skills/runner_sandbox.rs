use std::ffi::OsStr;

use crate::process_sandbox::{self, PreparedProcessCommand, ProcessSandboxRequest};

pub(super) fn prepare_runner_command(
    program: impl AsRef<OsStr>,
    request: ProcessSandboxRequest<'_>,
    durable: bool,
    host_process: bool,
) -> Result<PreparedProcessCommand, &'static str> {
    match (durable, host_process) {
        (true, true) => process_sandbox::prepare_durable_host_process_command(program, request),
        (true, false) => process_sandbox::prepare_durable_process_command(program, request),
        (false, true) => process_sandbox::prepare_host_process_command(program, request),
        (false, false) => process_sandbox::prepare_process_command(program, request),
    }
}

#[cfg(test)]
#[path = "runner_sandbox_tests.rs"]
mod tests;
