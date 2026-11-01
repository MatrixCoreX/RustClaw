use super::*;
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};
use std::time::Duration;

const GUARD: &str = "APP_ANCILLARY_TRUNCATION_TEST";
const REPORT_KIND: &str = "ancillary_truncation_complete";

#[test]
fn truncated_control_drain_and_drop_are_checked_in_owned_fresh_children() {
    let module = module_path!().split_once("::").unwrap().1;
    let selector = format!("{module}::truncated_control_child");
    for mode in ["drain", "drop"] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &selector,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .env(GUARD, mode)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                // This exact child runs only the owned socket/file probe, not a payload tree.
                let _ = child.kill();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "ancillary child deadline: mode={mode} status={:?}",
                    output.status
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(42),
            "mode={mode} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|value| value["kind"] == REPORT_KIND)
            .expect("fresh child completion report, not a zero-test success");
        assert_eq!(report["mode"], mode);
        assert_eq!(report["truncated"], true);
        assert_eq!(report["source_preserved"], true);
        assert_eq!(report["stdio_preserved"], true);
        assert_eq!(report["sent_fds"], 16);
        if mode == "drain" {
            let count = report["received_fds"].as_u64().unwrap();
            assert!(count > 0 && count < 16, "{report}");
        } else {
            assert!(report["received_fds"].is_null());
        }
        println!("ANCILLARY_TRUNCATION_ACCEPTANCE {report}");
    }
}

#[test]
#[ignore = "Executed exactly in a fresh child by the ancillary truncation parent"]
fn truncated_control_child() {
    let mode = std::env::var(GUARD).expect("fresh child guard");
    assert!(matches!(mode.as_str(), "drain" | "drop"));
    let (sender, receiver) = UnixStream::pair().unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let source = tempfile::tempfile().unwrap();
    let original = source.metadata().unwrap();
    super::tests::raw_send(&sender, b"probe", &[&source; 16]);
    let mut bytes = [0; 5];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    #[cfg(target_os = "linux")]
    let flags = RecvFlags::CMSG_CLOEXEC;
    #[cfg(not(target_os = "linux"))]
    let flags = RecvFlags::empty();
    let message = recvmsg(
        &receiver,
        &mut [IoSliceMut::new(&mut bytes)],
        &mut ancillary,
        flags,
    )
    .unwrap();
    assert_eq!(&bytes, b"probe");
    assert_eq!(message.bytes, bytes.len());
    assert!(message.flags.contains(ReturnFlags::CTRUNC));
    let received = if mode == "drain" {
        let mut received = 0;
        for message in ancillary.drain() {
            match message {
                RecvAncillaryMessage::ScmRights(rights) => {
                    for fd in rights {
                        let metadata = File::from(fd).metadata().unwrap();
                        assert_eq!(
                            (metadata.dev(), metadata.ino()),
                            (original.dev(), original.ino())
                        );
                        received += 1;
                    }
                }
                _ => panic!("unexpected ancillary type"),
            }
        }
        Some(received)
    } else {
        None
    };
    // Returning early on CTRUNC still invokes Drop; it must be safe too.
    drop(ancillary);
    assert!(fcntl_getfd(&source).is_ok());
    assert!(fcntl_getfd(std::io::stdin()).is_ok());
    assert!(fcntl_getfd(std::io::stdout()).is_ok());
    assert!(fcntl_getfd(std::io::stderr()).is_ok());
    println!(
        "\n{}",
        serde_json::json!({
            "kind": REPORT_KIND, "mode": mode, "truncated": true,
            "sent_fds": 16, "received_fds": received,
            "source_preserved": true, "stdio_preserved": true,
        })
    );
    std::process::exit(42);
}
