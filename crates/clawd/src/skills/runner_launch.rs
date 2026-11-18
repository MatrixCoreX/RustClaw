use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::Path;

use crate::local_process_owner::process_launch::{
    open_working_directory, LaunchFailure, PayloadIo, ProcessLaunch,
};
use crate::process_invocation::ProcessInvocation;

// Keep authority-projected values explicit until the actual spawn boundary.
// The environment can contain credentials, so never derive Debug/Serialize.
pub(super) struct RunnerLaunch {
    invocation: ProcessInvocation,
    cwd: File,
    environment: BTreeMap<OsString, OsString>,
}

impl RunnerLaunch {
    pub(super) fn new(
        invocation: ProcessInvocation,
        cwd: &Path,
        environment: Vec<(OsString, OsString)>,
    ) -> Result<Self, LaunchFailure> {
        Ok(Self {
            invocation,
            cwd: open_working_directory(cwd)?,
            environment: environment.into_iter().collect(),
        })
    }

    pub(super) fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.environment
            .insert(key.as_ref().to_owned(), value.as_ref().to_owned());
        self
    }

    pub(super) fn envs<I, K, V>(&mut self, pairs: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        for (key, value) in pairs {
            self.env(key, value);
        }
        self
    }

    pub(super) fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.environment.remove(key.as_ref());
        self
    }

    pub(super) fn finish(self, io: PayloadIo) -> Result<ProcessLaunch, LaunchFailure> {
        ProcessLaunch::with_working_directory(
            self.invocation,
            self.cwd,
            self.environment.into_iter().collect(),
            io,
        )
    }
}

pub(super) fn launch_error(skill: &str, error: LaunchFailure) -> String {
    super::runner_host_pre_dispatch_error(
        skill,
        "skill_runner_launch_invalid",
        "repair_runtime_installation",
        serde_json::json!({"launch_error_code": error.error_code()}),
    )
}

#[cfg(test)]
#[path = "runner_launch_tests.rs"]
mod tests;
