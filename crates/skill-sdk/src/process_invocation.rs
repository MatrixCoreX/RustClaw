use std::ffi::{OsStr, OsString};
use std::process::Command;

// Own the invocation before materializing a process command. Deliberately no
// Debug/Serialize or conversion from Command: arguments may contain secrets,
// and Command getters cannot recover hidden environment/pre-exec state.
pub struct ProcessInvocation {
    program: OsString,
    args: Vec<OsString>,
}

impl ProcessInvocation {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
        }
    }

    pub fn arg(&mut self, value: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(value.as_ref().to_owned());
        self
    }

    pub fn args<I, S>(&mut self, values: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for value in values {
            self.arg(value);
        }
        self
    }

    pub fn into_parts(self) -> (OsString, Vec<OsString>) {
        (self.program, self.args)
    }

    pub fn into_command(self) -> Command {
        let (program, args) = self.into_parts();
        let mut command = Command::new(program);
        command.args(args);
        command
    }
}

#[cfg(test)]
#[path = "process_invocation_tests.rs"]
mod tests;
