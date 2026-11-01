use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use claw_core::config::{ToolSandboxBackend, ToolSandboxMode};

use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("command-launch-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn launch(&self, cwd: &Path, script: &Path) -> ProcessLaunch {
        prepare_async_command_launch(
            cwd,
            script,
            ToolSandboxMode::DangerFull,
            ToolSandboxBackend::Auto,
            &self.0,
            &[],
        )
        .expect("actual durable producer preparation")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

async fn run(launch: ProcessLaunch) {
    let mut command = Command::from(launch.into_command());
    command.kill_on_drop(true);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), command.status())
            .await
            .expect("bounded fixture command")
            .expect("fixture spawn")
            .success()
    );
}

#[tokio::test]
async fn async_launch_uses_admitted_directory_after_its_path_is_replaced() {
    let root = Fixture::new();
    let cwd = root.0.join("working directory");
    fs::create_dir(&cwd).unwrap();
    fs::write(cwd.join("marker"), b"admitted").unwrap();
    let script = root.0.join("run.sh");
    fs::write(&script, b"cat marker > ../observed").unwrap();
    let launch = root.launch(&cwd, &script);

    fs::rename(&cwd, root.0.join("original")).unwrap();
    fs::create_dir(&cwd).unwrap();
    fs::write(cwd.join("marker"), b"replacement").unwrap();

    run(launch).await;
    assert_eq!(fs::read(root.0.join("observed")).unwrap(), b"admitted");
    assert_eq!(fs::read(cwd.join("marker")).unwrap(), b"replacement");
}

#[tokio::test]
async fn async_launch_preserves_raw_script_argument_without_shell_interpolation() {
    let root = Fixture::new();
    let script = root.0.join(std::ffi::OsString::from_vec(
        b"script \xff ' $(touch injected).sh".to_vec(),
    ));
    fs::write(&script, b"printf exact-argument > observed").unwrap();
    run(root.launch(&root.0, &script)).await;
    assert_eq!(
        fs::read(root.0.join("observed")).unwrap(),
        b"exact-argument"
    );
    assert!(!root.0.join("injected").exists());
}

#[test]
fn async_launch_refuses_missing_relative_file_or_symlink_directory_before_spawn() {
    let root = Fixture::new();
    let script = root.0.join("run.sh");
    fs::write(&script, b"touch unexpected").unwrap();
    let file = root.0.join("file");
    fs::write(&file, b"not a directory").unwrap();
    let link = root.0.join("link");
    symlink(&root.0, &link).unwrap();

    for cwd in [
        root.0.join("missing"),
        PathBuf::from("relative"),
        file,
        link,
    ] {
        let error = prepare_async_command_launch(
            &cwd,
            &script,
            ToolSandboxMode::DangerFull,
            ToolSandboxBackend::Auto,
            &root.0,
            &[],
        )
        .err()
        .expect("invalid directory must not yield a launch");
        match error {
            RunSafeCommandError::Command(failure) => {
                assert_eq!(failure.kind, "async_job_launch_invalid");
                assert_eq!(failure.message, "process_launch_invalid_working_directory");
                assert!(failure.exit_code.is_none());
            }
            RunSafeCommandError::Policy(_) => panic!("unexpected direct-mode policy refusal"),
        }
    }
    assert!(!root.0.join("unexpected").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn host_private_policy_reaches_actual_durable_producer_preparation() {
    use std::os::unix::fs::DirBuilderExt;
    let root = Fixture::new();
    let private = root.0.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(private.join("marker"), b"owned-private-marker").unwrap();
    fs::write(root.0.join("public"), b"public-marker").unwrap();
    let script = root.0.join("run.sh");
    fs::write(&script, b"if cat private/marker >/dev/null 2>&1; then exit 41; fi; if (printf sandbox-only > private/child-output) 2>/dev/null; then exit 42; fi; cat public > observed").unwrap();
    let policy = crate::runtime::host_filesystem::HostFilesystemPolicy::pin_directories(
        &[private.clone()],
        &root.0,
    )
    .unwrap();
    let launch = prepare_async_command_launch(
        &root.0,
        &script,
        ToolSandboxMode::WorkspaceWrite,
        ToolSandboxBackend::Auto,
        &root.0,
        &policy.process_directories().unwrap(),
    )
    .unwrap();
    run(launch).await;
    assert_eq!(fs::read(root.0.join("observed")).unwrap(), b"public-marker");
    assert_eq!(
        fs::read(private.join("marker")).unwrap(),
        b"owned-private-marker"
    );
    assert!(!private.join("child-output").exists());
}

#[test]
fn async_launch_captures_raw_parent_environment_once() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "skills::builtin::builtin_run_cmd::launch_tests::raw_environment_child",
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "HOME",
            std::ffi::OsString::from_vec(b"/tmp/raw-\xff".to_vec()),
        )
        .env("APP_TEST_PARENT_SECRET", "private-before-capture")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "actual launch fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "isolated environment mutation, selected by its parent test"]
async fn raw_environment_child() {
    let root = Fixture::new();
    let script = root.0.join("run.sh");
    fs::write(
        &script,
        b"printf '%s\\n%s' \"$HOME\" \"${APP_TEST_PARENT_SECRET-unset}\" > observed",
    )
    .unwrap();
    let launch = root.launch(&root.0, &script);
    std::env::set_var("HOME", "changed-after-capture");
    std::env::set_var("APP_TEST_PARENT_SECRET", "private-after-capture");
    run(launch).await;
    assert_eq!(
        fs::read(root.0.join("observed")).unwrap(),
        b"/tmp/raw-\xff\nunset"
    );
}
