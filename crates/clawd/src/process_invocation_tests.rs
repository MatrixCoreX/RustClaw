use super::*;
use std::ffi::{OsStr, OsString};

#[test]
fn invocation_materialization_preserves_argument_boundaries_without_shell_parsing() {
    let mut invocation = ProcessInvocation::new("/bin/sh");
    for value in [
        "-c",
        "printf '%s\\n' \"$@\"",
        "fixture",
        "",
        "a b",
        "'quoted'",
        "$(not-executed)",
        "--flag=value",
    ] {
        invocation.arg(value);
    }
    let mut command = invocation.into_command();
    assert_eq!(command.get_program(), OsStr::new("/bin/sh"));
    assert!(command.get_current_dir().is_none());
    assert_eq!(command.get_envs().count(), 0);
    let output = command.output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        b"\na b\n'quoted'\n$(not-executed)\n--flag=value\n"
    );
}

#[cfg(unix)]
#[test]
fn invocation_keeps_non_utf8_program_and_argument_bytes_exact() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let program = OsString::from_vec(b"/tmp/program-\xff".to_vec());
    let argument = OsString::from_vec(b"value-\xfe".to_vec());
    let mut invocation = ProcessInvocation::new(&program);
    invocation.arg(&argument);
    let (owned_program, owned_args) = invocation.into_parts();
    assert_eq!(owned_program.as_bytes(), program.as_bytes());
    assert_eq!(owned_args, vec![argument.clone()]);
    let mut printable = ProcessInvocation::new("/bin/sh");
    for value in [
        OsStr::new("-c"),
        OsStr::new("printf '%s' \"$1\""),
        OsStr::new("fixture"),
        &argument,
    ] {
        printable.arg(value);
    }
    let output = printable.into_command().output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, argument.as_bytes());
}

#[test]
fn invocation_returns_owned_parts_without_retaining_a_second_mutable_command() {
    let mut program = OsString::from("program");
    let mut argument = OsString::from("value");
    let mut invocation = ProcessInvocation::new(&program);
    invocation.arg(&argument);
    program.push("-changed");
    argument.push("-changed");
    assert_eq!(
        invocation.into_parts(),
        (OsString::from("program"), vec![OsString::from("value")])
    );
}
