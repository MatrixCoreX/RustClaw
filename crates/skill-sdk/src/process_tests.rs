#[cfg(unix)]
#[test]
fn cancellation_terminates_the_complete_process_group() {
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    let trigger = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        signal.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30 & wait"]);
    let error = super::run_command_controlled(
        &mut command,
        None,
        Duration::from_secs(60),
        "build",
        Some(&cancelled),
    )
    .expect_err("cancellation must stop the process group");
    trigger.join().expect("cancel trigger");
    assert_eq!(error.code, "process_cancelled");
    assert!(started.elapsed() < Duration::from_secs(3));
}
