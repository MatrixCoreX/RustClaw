use std::ffi::OsStr;

use crate::process_sandbox::{self, PreparedSandboxInvocation, ProcessSandboxRequest};

pub(super) fn prepare_runner_invocation(
    program: impl AsRef<OsStr>,
    request: ProcessSandboxRequest<'_>,
    durable: bool,
    host_process: bool,
) -> Result<PreparedSandboxInvocation, &'static str> {
    match (durable, host_process) {
        (true, true) => process_sandbox::prepare_durable_host_process_invocation(program, request),
        (true, false) => process_sandbox::prepare_durable_process_invocation(program, request),
        (false, true) => process_sandbox::prepare_host_process_invocation(program, request),
        (false, false) => process_sandbox::prepare_process_invocation(program, request),
    }
}

#[cfg(test)]
#[path = "runner_sandbox_tests.rs"]
mod tests;
