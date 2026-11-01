use super::*;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn installed_child_keeps_directory_arguments_environment_and_stderr_on_both_paths() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().canonicalize().unwrap();
    for parent in [None, Some("direct")] {
        let mut launch = ChildLaunch::legacy("/bin/sh");
        launch.installed = true;
        launch.working_directory = Some(cwd.clone());
        launch.environment.insert("HOME".into(), "".into());
        launch.args = [
            "-c",
            "printf '%s\\n' \"$PWD\" \"$HOME\" \"$@\"; printf diagnostic >&2",
            "fixture",
            "",
            "a b",
            "$(touch injected)",
            "'quoted'",
        ]
        .map(String::from)
        .into();
        let mut command = child_process_command_for_parent(&launch, parent, None).unwrap();
        command.kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(5), command.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut expected = cwd.as_os_str().as_encoded_bytes().to_vec();
        expected.extend_from_slice(b"\n\n\na b\n$(touch injected)\n'quoted'\n");
        assert_eq!(output.stdout, expected);
        assert_eq!(output.stderr, b"diagnostic");
        assert!(!cwd.join("injected").exists());
    }
}
