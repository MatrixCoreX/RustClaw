use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::os::unix::ffi::OsStringExt;

use super::*;

pub(super) struct TestDirectory(pub(super) PathBuf);

impl TestDirectory {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!("launch-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

pub(super) fn null_io() -> PayloadIo {
    PayloadIo {
        stdin: None,
        stdout: None,
        stderr: None,
    }
}

fn invocation(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> ProcessInvocation {
    let mut invocation = ProcessInvocation::new(program);
    for arg in args {
        invocation.arg(arg);
    }
    invocation
}

#[test]
fn launch_uses_only_the_explicit_effective_environment_including_empty_values() {
    let root = TestDirectory::new();
    let output = root.0.join("output");
    let launch = ProcessLaunch::new(
        invocation("/usr/bin/env", &[]),
        root.0.clone(),
        vec![("ONLY_LAUNCH_TEST".into(), "".into())],
        PayloadIo {
            stdout: Some(File::create(&output).unwrap()),
            ..null_io()
        },
    )
    .unwrap();
    let mut command = launch.into_command();
    assert!(command.get_current_dir().is_none());
    assert!(command.status().unwrap().success());
    assert_eq!(fs::read(output).unwrap(), b"ONLY_LAUNCH_TEST=\n");
}

#[test]
fn launch_preserves_raw_environment_bytes_without_implicit_parent_values() {
    let root = TestDirectory::new();
    let output = root.0.join("output");
    let launch = ProcessLaunch::new(
        invocation("/usr/bin/env", &[]),
        root.0.clone(),
        vec![(
            OsString::from_vec(b"RAW_\xfe".to_vec()),
            OsString::from_vec(b"value-\xff".to_vec()),
        )],
        PayloadIo {
            stdout: Some(File::create(&output).unwrap()),
            ..null_io()
        },
    )
    .unwrap();
    assert!(launch.into_command().status().unwrap().success());
    assert_eq!(fs::read(output).unwrap(), b"RAW_\xfe=value-\xff\n");
}

#[test]
fn launch_keeps_working_directory_input_position_and_output_streams_distinct() {
    let root = TestDirectory::new();
    let cwd = root.0.join("directory with spaces");
    fs::create_dir(&cwd).unwrap();
    let input = root.0.join("input");
    fs::write(&input, b"skip:payload\n").unwrap();
    let mut input = File::open(input).unwrap();
    input.seek(SeekFrom::Start(5)).unwrap();
    let stdout = root.0.join("stdout");
    let stderr = root.0.join("stderr");
    let launch = ProcessLaunch::new(
        invocation(
            "/bin/sh",
            &[
                "-c",
                "pwd; IFS= read -r value; printf '%s' \"$value\"; printf 'diagnostic' >&2",
            ],
        ),
        cwd.clone(),
        vec![],
        PayloadIo {
            stdin: Some(input),
            stdout: Some(File::create(&stdout).unwrap()),
            stderr: Some(File::create(&stderr).unwrap()),
        },
    )
    .unwrap();
    let before = std::env::current_dir().unwrap();
    assert!(launch.into_command().status().unwrap().success());
    assert_eq!(std::env::current_dir().unwrap(), before);
    let mut expected = cwd.as_os_str().as_bytes().to_vec();
    expected.extend_from_slice(b"\npayload");
    assert_eq!(fs::read(stdout).unwrap(), expected);
    assert_eq!(fs::read(stderr).unwrap(), b"diagnostic");
}

#[test]
fn launch_keeps_open_output_identity_and_append_mode_instead_of_reopening_a_path() {
    let root = TestDirectory::new();
    let output = root.0.join("output");
    fs::write(&output, b"original:").unwrap();
    let file = OpenOptions::new().append(true).open(&output).unwrap();
    let launch = ProcessLaunch::new(
        invocation("/bin/sh", &["-c", "printf appended"]),
        root.0.clone(),
        vec![],
        PayloadIo {
            stdout: Some(file),
            ..null_io()
        },
    )
    .unwrap();
    let original = root.0.join("original");
    fs::rename(&output, &original).unwrap();
    fs::write(&output, b"replacement").unwrap();
    assert!(launch.into_command().status().unwrap().success());
    assert_eq!(fs::read(original).unwrap(), b"original:appended");
    assert_eq!(fs::read(output).unwrap(), b"replacement");
}

#[test]
fn launch_null_streams_do_not_inherit_the_helpers_control_streams() {
    let root = TestDirectory::new();
    let launch = ProcessLaunch::new(
        invocation(
            "/bin/sh",
            &[
                "-c",
                "read -r value && exit 9; printf ignored; printf ignored >&2",
            ],
        ),
        root.0.clone(),
        vec![],
        null_io(),
    )
    .unwrap();
    let output = launch.into_command().output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn launch_rejects_nul_empty_program_and_relative_working_directory_before_spawn() {
    let root = TestDirectory::new();
    for program in [
        OsString::new(),
        OsString::from_vec(b"/bin/sh\0bad".to_vec()),
    ] {
        assert_eq!(
            ProcessLaunch::new(invocation(program, &[]), root.0.clone(), vec![], null_io()).err(),
            Some(LaunchFailure::InvalidProgram)
        );
    }
    assert_eq!(
        ProcessLaunch::new(
            invocation("/bin/sh", &["bad\0arg"]),
            root.0.clone(),
            vec![],
            null_io()
        )
        .err(),
        Some(LaunchFailure::InvalidArgument)
    );
    for cwd in [
        PathBuf::from("relative"),
        PathBuf::from(OsString::from_vec(b"/tmp/bad\0path".to_vec())),
    ] {
        assert_eq!(
            ProcessLaunch::new(invocation("/bin/sh", &[]), cwd, vec![], null_io()).err(),
            Some(LaunchFailure::InvalidWorkingDirectory)
        );
    }
}

#[test]
fn launch_rejects_ambiguous_or_unrepresentable_environment_entries() {
    let root = TestDirectory::new();
    for entries in [
        vec![("".into(), "value".into())],
        vec![("BAD=KEY".into(), "value".into())],
        vec![("BAD\0KEY".into(), "value".into())],
        vec![("KEY".into(), "bad\0value".into())],
        vec![("KEY".into(), "one".into()), ("KEY".into(), "two".into())],
    ] {
        assert_eq!(
            ProcessLaunch::new(
                invocation("/bin/sh", &[]),
                root.0.clone(),
                entries,
                null_io()
            )
            .err(),
            Some(LaunchFailure::InvalidEnvironment)
        );
    }
}

#[test]
fn launch_refuses_incompatible_handle_directions_without_creating_a_child() {
    let root = TestDirectory::new();
    let data = root.0.join("data");
    fs::write(&data, []).unwrap();
    for io in [
        PayloadIo {
            stdin: Some(File::create(&data).unwrap()),
            ..null_io()
        },
        PayloadIo {
            stdout: Some(File::open(&data).unwrap()),
            ..null_io()
        },
        PayloadIo {
            stderr: Some(File::open(&data).unwrap()),
            ..null_io()
        },
    ] {
        assert_eq!(
            ProcessLaunch::new(invocation("/bin/sh", &[]), root.0.clone(), vec![], io).err(),
            Some(LaunchFailure::InvalidStdio)
        );
    }
}

#[test]
fn launch_pins_the_prepared_working_directory_across_a_path_replacement() {
    let root = TestDirectory::new();
    let cwd = root.0.join("workspace");
    fs::create_dir(&cwd).unwrap();
    let launch = ProcessLaunch::new(
        invocation("/bin/sh", &["-c", "printf pinned > result"]),
        cwd.clone(),
        vec![],
        null_io(),
    )
    .unwrap();
    let original = root.0.join("original");
    fs::rename(&cwd, &original).unwrap();
    fs::create_dir(&cwd).unwrap();
    assert!(launch.into_command().status().unwrap().success());
    assert_eq!(fs::read(original.join("result")).unwrap(), b"pinned");
    assert!(!cwd.join("result").exists());
}

#[test]
fn launch_refuses_a_regular_file_as_cwd_and_directories_as_data_streams() {
    let root = TestDirectory::new();
    let data = root.0.join("data");
    fs::write(&data, []).unwrap();
    assert_eq!(
        ProcessLaunch::new(invocation("/bin/sh", &[]), data, vec![], null_io()).err(),
        Some(LaunchFailure::InvalidWorkingDirectory)
    );
    assert_eq!(
        ProcessLaunch::new(
            invocation("/bin/sh", &[]),
            root.0.clone(),
            vec![],
            PayloadIo {
                stdin: Some(File::open(&root.0).unwrap()),
                ..null_io()
            }
        )
        .err(),
        Some(LaunchFailure::InvalidStdio)
    );
}

#[test]
fn launch_requires_an_admitted_close_on_exec_directory_handle() {
    use std::os::unix::fs::symlink;

    let root = TestDirectory::new();
    let alias = root.0.join("alias");
    symlink(&root.0, &alias).unwrap();
    assert_eq!(
        ProcessLaunch::new(invocation("/bin/sh", &[]), alias, vec![], null_io()).err(),
        Some(LaunchFailure::InvalidWorkingDirectory)
    );
    let data = root.0.join("data");
    fs::write(&data, []).unwrap();
    assert_eq!(
        ProcessLaunch::with_working_directory(
            invocation("/bin/sh", &[]),
            File::open(data).unwrap(),
            vec![],
            null_io()
        )
        .err(),
        Some(LaunchFailure::InvalidWorkingDirectory)
    );
    let directory = File::open(&root.0).unwrap();
    assert_eq!(
        unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_SETFD, 0) },
        0
    );
    assert_eq!(
        ProcessLaunch::with_working_directory(
            invocation("/bin/sh", &[]),
            directory,
            vec![],
            null_io()
        )
        .err(),
        Some(LaunchFailure::InvalidWorkingDirectory)
    );
}
