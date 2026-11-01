use std::fs::File;
use std::os::unix::ffi::OsStringExt;

use super::*;

fn null_io() -> PayloadIo {
    PayloadIo {
        stdin: None,
        stdout: None,
        stderr: None,
    }
}

fn invocation(program: &str, args: &[&str]) -> ProcessInvocation {
    let mut invocation = ProcessInvocation::new(program);
    for arg in args {
        invocation.arg(arg);
    }
    invocation
}

#[test]
fn effective_environment_is_explicit_last_writer_wins_and_removal_is_final() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().canonicalize().unwrap();
    let output = cwd.join("environment");
    let mut launch = RunnerLaunch::new(
        invocation("/usr/bin/env", &[]),
        &cwd,
        vec![
            ("ONLY".into(), "first".into()),
            ("DROP".into(), "secret".into()),
        ],
    )
    .unwrap();
    launch
        .envs([("ONLY", "second"), ("EMPTY", "")])
        .env_remove("DROP");
    let status = launch
        .finish(PayloadIo {
            stdout: Some(File::create(&output).unwrap()),
            ..null_io()
        })
        .unwrap()
        .into_command()
        .status()
        .unwrap();
    assert!(status.success());
    // Exact output also proves that HOME/PATH/provider secrets were not inherited.
    assert_eq!(std::fs::read(output).unwrap(), b"EMPTY=\nONLY=second\n");
}

#[test]
fn raw_arguments_and_environment_values_survive_the_real_process_boundary() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().canonicalize().unwrap();
    let output = cwd.join("raw");
    let raw = OsString::from_vec(b"value-\xff".to_vec());
    let mut call = invocation("/bin/sh", &["-c", "printf '%s' \"$1\"", "fixture"]);
    call.arg(&raw);
    let launch = RunnerLaunch::new(call, &cwd, vec![]).unwrap();
    assert!(launch
        .finish(PayloadIo {
            stdout: Some(File::create(&output).unwrap()),
            ..null_io()
        })
        .unwrap()
        .into_command()
        .status()
        .unwrap()
        .success());
    assert_eq!(std::fs::read(&output).unwrap(), b"value-\xff");

    let mut launch = RunnerLaunch::new(invocation("/usr/bin/env", &[]), &cwd, vec![]).unwrap();
    launch.env(OsString::from_vec(b"RAW_\xfe".to_vec()), raw);
    assert!(launch
        .finish(PayloadIo {
            stdout: Some(File::create(&output).unwrap()),
            ..null_io()
        })
        .unwrap()
        .into_command()
        .status()
        .unwrap()
        .success());
    assert_eq!(std::fs::read(output).unwrap(), b"RAW_\xfe=value-\xff\n");
}

#[test]
fn explicit_data_streams_remain_distinct_and_null_input_is_eof() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().canonicalize().unwrap();
    let input = cwd.join("input");
    let output = cwd.join("output");
    let error = cwd.join("error");
    std::fs::write(&input, b"payload\n").unwrap();
    let launch = RunnerLaunch::new(
        invocation(
            "/bin/sh",
            &[
                "-c",
                "IFS= read -r line; printf '%s' \"$line\"; printf diagnostic >&2",
            ],
        ),
        &cwd,
        vec![],
    )
    .unwrap();
    assert!(launch
        .finish(PayloadIo {
            stdin: Some(File::open(&input).unwrap()),
            stdout: Some(File::create(&output).unwrap()),
            stderr: Some(File::create(&error).unwrap()),
        })
        .unwrap()
        .into_command()
        .status()
        .unwrap()
        .success());
    assert_eq!(std::fs::read(output).unwrap(), b"payload");
    assert_eq!(std::fs::read(error).unwrap(), b"diagnostic");
    let launch = RunnerLaunch::new(
        invocation("/bin/sh", &["-c", "if IFS= read -r line; then exit 9; fi"]),
        &cwd,
        vec![],
    )
    .unwrap();
    assert!(launch
        .finish(null_io())
        .unwrap()
        .into_command()
        .status()
        .unwrap()
        .success());
}

#[test]
fn invalid_effective_environment_is_rejected_before_command_materialization() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().canonicalize().unwrap();
    for (key, value) in [("", "value"), ("A=B", "value"), ("KEY", "bad\0value")] {
        let mut launch = RunnerLaunch::new(invocation("/bin/sh", &[]), &cwd, vec![]).unwrap();
        launch.env(key, value);
        assert!(matches!(
            launch.finish(null_io()),
            Err(LaunchFailure::InvalidEnvironment)
        ));
    }
}

#[test]
fn non_directory_and_symlink_working_directories_are_not_admitted() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().canonicalize().unwrap();
    let file = cwd.join("file");
    let link = cwd.join("link");
    std::fs::write(&file, b"not a directory").unwrap();
    std::os::unix::fs::symlink(&cwd, &link).unwrap();
    for path in [&file, &link, Path::new("relative")] {
        assert!(matches!(
            RunnerLaunch::new(invocation("/bin/sh", &[]), path, vec![]),
            Err(LaunchFailure::InvalidWorkingDirectory)
        ));
    }
}
