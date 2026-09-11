#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use claw_core::config::{ToolSandboxBackend, ToolSandboxMode};

    use super::super::prepare_runner_command;
    use crate::process_sandbox::{ProcessNetworkPolicy, ProcessSandboxRequest};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("durable-runner-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn request(root: &std::path::Path) -> ProcessSandboxRequest<'_> {
        ProcessSandboxRequest {
            mode: ToolSandboxMode::WorkspaceWrite,
            backend: ToolSandboxBackend::Auto,
            workspace_root: root,
            execution_root: root,
            network: ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
        }
    }

    fn bubblewrap_available() -> bool {
        std::path::Path::new("/usr/bin/bwrap").is_file()
            || std::path::Path::new("/bin/bwrap").is_file()
    }

    #[test]
    fn runner_lifetime_and_pid_visibility_are_independent() {
        if !bubblewrap_available() {
            return;
        }
        let root = Scratch::new();
        for durable in [false, true] {
            for host_process in [false, true] {
                let prepared =
                    prepare_runner_command("/bin/sh", request(&root.0), durable, host_process)
                        .unwrap();
                let args: Vec<_> = prepared.command.as_std().get_args().collect();
                let has = |value: &str| args.iter().any(|arg| *arg == value);
                assert_eq!(has("--die-with-parent"), !durable);
                assert_eq!(has("--unshare-pid"), !host_process);
                assert!(has("--new-session"));
                assert!(has("--unshare-net"));
                assert!(has("--unshare-ipc"));
                assert!(has("--unshare-uts"));
                assert!(has("--ro-bind"));
            }
        }
    }

    #[tokio::test]
    async fn durable_runner_parent_probe() {
        let Some(root) = std::env::var_os("DURABLE_RUNNER_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let durable = std::env::var("DURABLE_RUNNER_TEST_MODE").unwrap() == "durable";
        let mut prepared =
            prepare_runner_command("/bin/sh", request(&root), durable, false).unwrap();
        prepared
            .command
            .args(["-c", "printf ready > ready; sleep 3; printf done > done"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(false);
        assert!(prepared
            .command
            .spawn()
            .unwrap()
            .wait()
            .await
            .unwrap()
            .success());
    }

    fn check_parent_exit(durable: bool) {
        if !bubblewrap_available() {
            return;
        }
        let root = Scratch::new();
        let mut parent = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "skills::runner::sandbox::tests::linux::durable_runner_parent_probe",
                "--nocapture",
            ])
            .env("DURABLE_RUNNER_TEST_ROOT", &root.0)
            .env(
                "DURABLE_RUNNER_TEST_MODE",
                if durable { "durable" } else { "foreground" },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.0.join("ready").exists() {
            if parent.try_wait().unwrap().is_some() || Instant::now() >= deadline {
                let _ = parent.kill();
                let _ = parent.wait();
                panic!("sandbox probe did not become ready");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        parent.kill().unwrap();
        parent.wait().unwrap();
        std::thread::sleep(Duration::from_secs(4));
        assert_eq!(root.0.join("done").exists(), durable);
    }

    #[test]
    fn durable_runner_survives_real_parent_exit() {
        check_parent_exit(true);
    }

    #[test]
    fn foreground_runner_dies_with_real_parent_exit() {
        check_parent_exit(false);
    }
}
