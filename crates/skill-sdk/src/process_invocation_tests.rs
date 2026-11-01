use super::*;

#[cfg(unix)]
#[test]
fn bulk_arguments_keep_empty_quoted_and_shell_syntax_as_data() {
    let mut invocation = ProcessInvocation::new("/bin/sh");
    invocation.args([
        "-c",
        "printf '%s\\n' \"$@\"",
        "fixture",
        "",
        "a b",
        "'quoted'",
        "$(not-executed)",
        "--flag=value",
    ]);
    let mut command = invocation.into_command();
    assert!(command.get_current_dir().is_none());
    assert_eq!(command.get_envs().count(), 0);
    let output = command.output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"\na b\n'quoted'\n$(not-executed)\n--flag=value\n"
    );
}

#[cfg(unix)]
#[test]
fn bulk_arguments_preserve_raw_os_bytes_without_lossy_conversion() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let raw = OsString::from_vec(b"value-\xff".to_vec());
    let mut invocation = ProcessInvocation::new("/bin/sh");
    invocation.args([
        OsStr::new("-c"),
        OsStr::new("printf '%s' \"$1\""),
        OsStr::new("fixture"),
        &raw,
    ]);
    let output = invocation.into_command().output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, raw.as_bytes());
}

#[test]
fn owned_parts_are_independent_of_later_source_changes() {
    let mut program = OsString::from("program");
    let mut argument = OsString::from("argument");
    let mut invocation = ProcessInvocation::new(&program);
    invocation.args([&argument]);
    program.push("-changed");
    argument.push("-changed");
    assert_eq!(
        invocation.into_parts(),
        (OsString::from("program"), vec![OsString::from("argument")])
    );
}
