use super::{WarmPoolCheckout, WarmRunnerKey, WarmRunnerPool};
use futures_util::StreamExt;

fn key(generation: u64) -> WarmRunnerKey {
    WarmRunnerKey {
        scope_token: "sample".to_string(),
        version_pin: skill_sdk::SkillVersionPin {
            skill_name: "sample".to_string(),
            version: "1.0.0".to_string(),
            adapter: skill_sdk::BuildAdapter::Cargo,
            progress_frames: false,
            execution_profile: skill_sdk::ExecutionProfile::StatelessReadonly,
            sandbox_profile: skill_sdk::SandboxProfile::ReadOnly,
            environment_allowlist: Vec::new(),
            install_root: std::path::PathBuf::from("/tmp/sample"),
            manifest_digest: "a".repeat(64),
            receipt_digest: "b".repeat(64),
        },
        admission_binding: None,
        registry_generation: generation,
        registry_generation_digest: Some("c".repeat(64)),
        base_registry_digest: Some("d".repeat(64)),
        overlay_generation_digest: None,
        sandbox_backend: "test".to_string(),
        timeout_seconds: 30,
        memory_reservation_mib: 128,
    }
}

#[test]
fn disabled_pool_requires_per_request_fallback() {
    let pool = WarmRunnerPool::default();
    assert!(matches!(
        pool.checkout(&key(1)),
        WarmPoolCheckout::Fallback("warm_pool_disabled")
    ));
    assert_eq!(pool.idle_count(), 0);
}

#[test]
fn enabled_pool_without_idle_runner_requests_spawn() {
    let pool = WarmRunnerPool::new(true, 1, 0, 60);
    assert!(matches!(pool.checkout(&key(1)), WarmPoolCheckout::Spawn(_)));
}

#[cfg(target_os = "linux")]
#[test]
fn low_memory_guard_returns_structured_per_request_fallback() {
    let pool = WarmRunnerPool::new(true, 1, u64::MAX, 60);
    assert!(matches!(
        pool.checkout(&key(1)),
        WarmPoolCheckout::Fallback("warm_pool_low_memory")
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_checked_out_process_terminates_it() {
    let mut command = tokio::process::Command::new("sh");
    command.args(["-c", "while IFS= read -r line; do :; done"]);
    let process = super::WarmRunnerProcess::spawn(command).expect("runner process");
    let pid = process.id().expect("pid") as i32;
    drop(process);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if !alive {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "runner remained alive"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn checked_in_process_is_reused_and_generation_change_discards_it() {
    let pool = WarmRunnerPool::new(true, 1, 0, 60);
    let mut command = tokio::process::Command::new("sh");
    command.args([
        "-c",
        "while IFS= read -r line; do printf '%s\\n' '{\"status\":\"ok\"}'; done",
    ]);
    let mut process = super::WarmRunnerProcess::spawn(command).expect("runner process");
    process.send("{}").await.expect("send");
    assert_eq!(
        process.records.next().await.expect("record").expect("line"),
        r#"{"status":"ok"}"#
    );
    let initial_epoch = match pool.checkout(&key(1)) {
        WarmPoolCheckout::Spawn(epoch) => epoch,
        _ => panic!("expected spawn"),
    };
    pool.checkin(key(1), initial_epoch, process);
    assert_eq!(pool.idle_count(), 1);
    let (process, checkout_epoch) = match pool.checkout(&key(1)) {
        WarmPoolCheckout::Reused(process, epoch) => (process, epoch),
        _ => panic!("expected reuse"),
    };
    pool.invalidate_all();
    pool.checkin(key(1), checkout_epoch, process);
    assert_eq!(pool.idle_count(), 0);
    assert!(matches!(pool.checkout(&key(2)), WarmPoolCheckout::Spawn(_)));
    assert_eq!(pool.idle_count(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn invalidating_pool_terminates_and_reaps_idle_processes() {
    let pool = WarmRunnerPool::new(true, 1, 0, 60);
    let mut command = tokio::process::Command::new("sh");
    command.args(["-c", "while IFS= read -r line; do :; done"]);
    let process = super::WarmRunnerProcess::spawn(command).expect("runner process");
    let pid = process.id().expect("pid") as i32;
    let epoch = match pool.checkout(&key(1)) {
        WarmPoolCheckout::Spawn(epoch) => epoch,
        _ => panic!("expected spawn"),
    };
    pool.checkin(key(1), epoch, process);
    assert_eq!(pool.idle_count(), 1);

    pool.invalidate_all();
    assert_eq!(pool.idle_count(), 0);

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let alive_or_unreaped = unsafe { libc::kill(pid, 0) } == 0;
        if !alive_or_unreaped {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "invalidated runner remained alive or unreaped"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn runner_larger_than_its_reservation_is_not_kept_warm() {
    let pool = WarmRunnerPool::new(true, 1, 0, 60);
    let mut command = tokio::process::Command::new("sh");
    command.args(["-c", "while IFS= read -r line; do :; done"]);
    let process = super::WarmRunnerProcess::spawn(command).expect("runner process");
    assert!(process.resident_memory_mib().is_some_and(|rss| rss > 0));
    let mut underreported = key(1);
    underreported.memory_reservation_mib = 0;
    let epoch = match pool.checkout(&underreported) {
        WarmPoolCheckout::Spawn(epoch) => epoch,
        _ => panic!("expected spawn"),
    };
    pool.checkin(underreported, epoch, process);
    assert_eq!(pool.idle_count(), 0);
}
