#[cfg(target_os = "linux")]
use std::fs::OpenOptions;
use std::fs::{DirBuilder, Permissions};
#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
#[cfg(target_os = "linux")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use super::super::owner_protocol::{
    decode_reply, OwnerAction, OwnerCompletion, OwnerCredential, OwnerPhase, OwnerRequest,
};
use super::*;

struct PrivateDirectory(PathBuf);

impl PrivateDirectory {
    fn new() -> Self {
        // Keep room for macOS's shorter Unix-socket pathname budget.
        let path = std::env::temp_dir().join(format!(
            "owner-ipc-{:016x}",
            uuid::Uuid::new_v4().as_u64_pair().1
        ));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }

    fn socket(&self, name: &str) -> UnixDatagram {
        bind_private_socket(&self.0.join(name))
    }
}

fn bind_private_socket(path: &std::path::Path) -> UnixDatagram {
    let socket = UnixDatagram::bind(path).unwrap();
    std::fs::set_permissions(path, Permissions::from_mode(0o600)).unwrap();
    socket
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn owner_transport_refuses_unbound_or_public_endpoints_before_payload_start() {
    assert!(matches!(
        OwnerDatagram::new(UnixDatagram::unbound().unwrap()),
        Err(OwnerFailure::TransportUnavailable)
    ));
    let directory = PrivateDirectory::new();
    let socket = directory.socket("public.sock");
    std::fs::set_permissions(
        directory.0.join("public.sock"),
        Permissions::from_mode(0o666),
    )
    .unwrap();
    assert!(matches!(
        OwnerDatagram::new(socket),
        Err(OwnerFailure::TransportUnavailable)
    ));
    let socket = directory.socket("private.sock");
    std::fs::set_permissions(&directory.0, Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        OwnerDatagram::new(socket),
        Err(OwnerFailure::TransportUnavailable)
    ));
    std::fs::set_permissions(&directory.0, Permissions::from_mode(0o700)).unwrap();
    assert!(OwnerDatagram::new(directory.socket("valid.sock")).is_ok());
}

#[cfg(target_os = "linux")]
struct Fixture {
    child: FixtureChild,
    directory: PrivateDirectory,
    credential: OwnerCredential,
}

#[cfg(target_os = "linux")]
struct FixtureChild(Option<std::process::Child>);

#[cfg(target_os = "linux")]
impl Drop for FixtureChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.take() {
            // The fixture owns its finite lifetime and payload cleanup.
            let _ = child.wait_with_output();
        }
    }
}

#[cfg(target_os = "linux")]
impl Fixture {
    fn start() -> Self {
        use std::process::Stdio;

        let directory = PrivateDirectory::new();
        let mut child = FixtureChild(Some(
            super::super::owner_scope::tests::fixture_command("transport_server", None)
                .env("APP_OWNER_TRANSPORT_TEST_DIR", &directory.0)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ));
        let ready = directory.0.join("ready.json");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            if child.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                let output = child.0.take().unwrap().wait_with_output().unwrap();
                panic!(
                    "isolated owner startup failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            assert!(
                Instant::now() < deadline,
                "isolated owner did not become ready"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let credential = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        Self {
            child,
            directory,
            credential,
        }
    }

    fn client(&self, name: &str) -> UnixDatagram {
        let socket = self.directory.socket(name);
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
    }

    fn request(&self, id: u64, action: OwnerAction) -> OwnerRequest {
        OwnerRequest {
            schema_version: 1,
            credential: self.credential.clone(),
            request_id: id,
            action,
        }
    }

    fn send(&self, socket: &UnixDatagram, request: &OwnerRequest) {
        self.send_bytes(socket, &serde_json::to_vec(request).unwrap());
    }

    fn send_bytes(&self, socket: &UnixDatagram, bytes: &[u8]) {
        assert_eq!(
            socket
                .send_to(bytes, self.directory.0.join("server.sock"))
                .unwrap(),
            bytes.len()
        );
    }

    fn roundtrip(&self, socket: &UnixDatagram, request: &OwnerRequest) -> OwnerReply {
        self.send(socket, request);
        let mut bytes = [0; MAX_CONTROL_BYTES + 1];
        let (length, sender) = socket.recv_from(&mut bytes).unwrap();
        assert_eq!(
            sender.as_pathname(),
            Some(self.directory.0.join("server.sock").as_path())
        );
        let reply = decode_reply(&bytes[..length]).unwrap();
        assert_eq!(reply.request_id, request.request_id);
        reply
    }

    fn wait_terminal(&self, client: &UnixDatagram, first_id: u64) {
        let deadline = Instant::now() + Duration::from_secs(3);
        for id in first_id.. {
            let reply = self.roundtrip(client, &self.request(id, OwnerAction::Inspect));
            let OwnerOutcome::Ok { snapshot } = reply.outcome else {
                panic!("scope observation failed");
            };
            snapshot.validate().unwrap();
            if snapshot.phase == OwnerPhase::Terminal {
                assert_eq!(snapshot.completion, Some(OwnerCompletion::Cancelled));
                assert_eq!(snapshot.root_exit.unwrap().signal, Some(libc::SIGTERM));
                return;
            }
            assert!(
                Instant::now() < deadline,
                "isolated cancellation did not settle"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn finish(mut self) -> serde_json::Value {
        std::fs::write(self.directory.0.join("finish"), []).unwrap();
        let output = self.child.0.take().unwrap().wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("{stdout}");
        assert!(
            output.status.success(),
            "owner fixture failed: {stdout}\n{stderr}"
        );
        assert!(stdout.contains("running 1 test"));
        assert!(stdout.contains("1 passed; 0 failed"));
        serde_json::from_slice(&std::fs::read(self.directory.0.join("report.json")).unwrap())
            .unwrap()
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owner_transport_reconnect_and_lost_cancel_reply_do_not_repeat_the_action() {
    let fixture = Fixture::start();
    let first = fixture.client("one.sock");
    let inspect = fixture.request(1, OwnerAction::Inspect);
    let initial = fixture.roundtrip(&first, &inspect);
    assert!(matches!(
        initial.outcome,
        OwnerOutcome::Ok {
            snapshot: OwnerSnapshot {
                phase: OwnerPhase::Running,
                ..
            }
        }
    ));
    drop(first);
    let second = fixture.client("two.sock");
    assert_eq!(fixture.roundtrip(&second, &inspect), initial);
    let cancel = fixture.request(2, OwnerAction::Cancel);
    fixture.send(&second, &cancel);
    drop(second);
    let third = fixture.client("three.sock");
    let cached = fixture.roundtrip(&third, &cancel);
    assert_eq!(fixture.roundtrip(&third, &cancel), cached);
    fixture.wait_terminal(&third, 3);
    let report = fixture.finish();
    assert!(report["received"].as_u64().unwrap() >= 6);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_transport_malformed_flood_and_wrong_credential_cannot_cancel_or_consume_sequence() {
    let fixture = Fixture::start();
    let client = fixture.client("one.sock");
    let mut wrong = fixture.request(1, OwnerAction::Cancel);
    wrong.credential.nonce = uuid::Uuid::new_v4();
    fixture.send(&client, &wrong);
    fixture.send_bytes(&client, &vec![b'x'; MAX_CONTROL_BYTES + 1]);
    let unnamed = UnixDatagram::unbound().unwrap();
    unnamed
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    fixture.send(&unnamed, &fixture.request(1, OwnerAction::Cancel));
    for _ in 0..32 {
        fixture.send_bytes(&client, b"{");
    }
    let reply = fixture.roundtrip(&client, &fixture.request(1, OwnerAction::Inspect));
    assert!(matches!(
        reply.outcome,
        OwnerOutcome::Ok {
            snapshot: OwnerSnapshot {
                phase: OwnerPhase::Running,
                ..
            }
        }
    ));
    fixture.roundtrip(&client, &fixture.request(2, OwnerAction::Cancel));
    fixture.wait_terminal(&client, 3);
    let report = fixture.finish();
    assert_eq!(report["rejected"], 35);
    assert!(report["max_received_per_pump"].as_u64().unwrap() <= MAX_PACKETS_PER_PUMP as u64);
}

#[cfg(target_os = "linux")]
pub(crate) fn run_server() {
    use std::process::{Command, Stdio};

    let root = PathBuf::from(std::env::var_os("APP_OWNER_TRANSPORT_TEST_DIR").unwrap());
    let transport = OwnerDatagram::new(bind_private_socket(&root.join("server.sock"))).unwrap();
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "read -r value"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut owner = unsafe {
        OwnerRuntime::spawn(
            &mut command,
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            Duration::from_secs(1),
        )
    }
    .unwrap();
    let _input = owner.take_stdin().unwrap();
    write_private(
        &root.join("credential.json"),
        &serde_json::to_vec(&owner.credential()).unwrap(),
    );
    std::fs::rename(root.join("credential.json"), root.join("ready.json")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    let (mut received, mut rejected, mut max_received, mut replies_failed) = (0, 0, 0, 0);
    loop {
        let step = transport.pump(&mut owner);
        assert!(!step.transport_unavailable);
        received += step.received;
        rejected += step.rejected;
        replies_failed += step.replies_failed;
        max_received = max_received.max(step.received);
        let observation = step.observation.unwrap();
        if root.join("finish").exists() && observation.phase == OwnerPhase::Terminal {
            write_private(
                &root.join("report.json"),
                &serde_json::to_vec(&serde_json::json!({
                    "received": received, "rejected": rejected, "replies_failed": replies_failed,
                    "max_received_per_pump": max_received,
                }))
                .unwrap(),
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "private fixture lifetime exceeded"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(target_os = "linux")]
fn write_private(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}
