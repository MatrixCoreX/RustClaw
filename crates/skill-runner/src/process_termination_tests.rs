use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_CASE: &str = "process_termination_tests::owned_group_termination_fixture";
const CHILD_ENV: &str = "APP_TERMINATION_TEST_CHILD";

#[test]
fn timeout_termination_keeps_the_callers_group_alive() {
    // The legacy kill CLI may target its caller group on some hosts. Execute
    // it only in a new owned group, never in the Cargo/CI/operator group.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD_CASE, "--ignored", "--nocapture"])
        .env(CHILD_ENV, "1")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("owned termination fixture deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        status.success(),
        "owned termination fixture failed: {status}"
    );
}

#[tokio::test]
#[ignore = "run only as the guarded subprocess of the parent regression"]
async fn owned_group_termination_fixture() {
    use std::os::unix::process::ExitStatusExt;

    assert_eq!(std::env::var(CHILD_ENV).as_deref(), Ok("1"));
    let mut target = tokio::process::Command::new("/bin/sh")
        .args(["-c", "read -r _ || :"])
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    // A killed/panicking helper closes this writer, so a surviving fixture
    // child exits on EOF rather than leaking an unbounded background process.
    let _input = target.stdin.take().unwrap();
    let pid = target.id().unwrap();
    assert!(pid > 1);
    super::terminate_child_process_group(Some(pid)).await;
    let status = tokio::time::timeout(Duration::from_secs(3), target.wait())
        .await
        .expect("owned target termination deadline")
        .unwrap();
    assert_eq!(status.signal(), Some(9));
}
