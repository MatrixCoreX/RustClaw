use super::*;
use std::{process::{Command, Stdio}, time::Instant};

#[test]
#[ignore = "Child entry used only by the parent-exit acceptance test"]
fn parent_host() {
    let Some(output) = std::env::var_os("DESKTOP_WALLET_PARENT_PROBE") else { return; };
    let root = tempfile::tempdir().unwrap();
    let mut worker = client(&root.path().join("vault"));
    worker.initialize(PASSWORD).unwrap();
    worker.create("parent death fixture").unwrap();
    fs::write(output, worker.worker.as_ref().unwrap().id().to_string()).unwrap();
    // Parent test terminates this host without running destructors.
    loop { std::thread::sleep(Duration::from_secs(1)); }
}

#[test]
#[ignore = "Requires packaged binary and disposable OS credentials"]
fn parent_death_terminates_unlocked_worker() {
    let root = tempfile::tempdir().unwrap();
    let pid_file = root.path().join("pid");
    let mut host = Command::new(std::env::current_exe().unwrap()).args(["--exact",
        "wallet::worker::client::native_tests::parent_tests::parent_host", "--ignored", "--nocapture"])
        .env("DESKTOP_WALLET_PARENT_PROBE", &pid_file).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !pid_file.exists() {
        assert!(host.try_wait().unwrap().is_none(), "parent fixture failed");
        if Instant::now() >= deadline { let _=host.kill(); let _=host.wait(); panic!("parent fixture timeout"); }
        std::thread::sleep(Duration::from_millis(50));
    }
    let pid: u32 = fs::read_to_string(pid_file).unwrap().parse().unwrap();
    #[cfg(windows)]
    let process = unsafe { windows_sys::Win32::System::Threading::OpenProcess(
        windows_sys::Win32::System::Threading::SYNCHRONIZATION_SYNCHRONIZE, 0, pid) };
    host.kill().unwrap();
    host.wait().unwrap();
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{Foundation::*, System::Threading::*};
        assert!(!process.is_null());
        let status = WaitForSingleObject(process, 5000);
        CloseHandle(process);
        assert_eq!(status, WAIT_OBJECT_0, "orphaned worker");
    }
    #[cfg(unix)]
    {
        let deadline = Instant::now() + Duration::from_secs(5);
        while unsafe { libc::kill(pid as i32, 0) } == 0 {
            #[cfg(target_os="linux")]
            if fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.contains(") Z")) { break; }
            assert!(Instant::now() < deadline, "orphaned worker");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    evidence("wallet-native-parent.json", json!({"ok":true,"checks":["hard_parent_exit_terminates_unlocked_worker"]}));
}
