use std::fs;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

// Only new, isolated fixture data and an unreaped immediate child are owned.
// Dropping the input pipe ends the script; no saved PID/group is signalled.
pub(crate) struct LiveJobFixture {
    directory: tempfile::TempDir,
    child: Child,
    input: Option<ChildStdin>,
}

impl LiveJobFixture {
    pub(crate) fn new() -> Self {
        use std::os::unix::process::CommandExt;
        let directory = tempfile::Builder::new().prefix("jo-").tempdir().unwrap();
        let script = directory.path().join("run.sh");
        fs::write(&script, "printf ready > \"$1\"\nread -r _ || :\n").unwrap();
        let mut child = Command::new("/bin/sh")
            .arg(&script)
            .arg(directory.path().join("ready"))
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let fixture = Self {
            directory,
            child,
            input: Some(input),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.path().join("ready").is_file() {
            assert!(Instant::now() < deadline, "owned_fixture_not_ready");
            std::thread::sleep(Duration::from_millis(2));
        }
        fixture.write("pid", &fixture.child.id().to_string());
        fixture.write("process_command_marker", script.to_str().unwrap());
        assert_eq!(
            super::process_identity_state(fixture.path(), fixture.pid()),
            super::ProcessIdentityState::AliveVerified
        );
        fixture
    }

    pub(crate) fn path(&self) -> &Path {
        self.directory.path()
    }

    pub(crate) fn pid(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn write(&self, name: &str, value: &str) {
        fs::write(self.path().join(name), value).unwrap();
    }

    pub(crate) fn request_cancel(&self) {
        self.write("cancel_requested_at", "1000");
        self.write("terminate_grace_seconds", "3600");
    }

    pub(crate) fn finish(&mut self) {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(Instant::now() < deadline, "owned_fixture_not_finished");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for LiveJobFixture {
    fn drop(&mut self) {
        drop(self.input.take());
        // This fixture's fixed script exits on EOF. Only its held immediate
        // child is eligible for cleanup if fixture setup itself is interrupted.
        if matches!(self.child.try_wait(), Ok(None)) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while matches!(self.child.try_wait(), Ok(None)) {
                if Instant::now() >= deadline {
                    let _ = self.child.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let _ = self.child.wait();
        }
    }
}

#[test]
fn owned_observation_fixture_reaps_only_its_immediate_process() {
    let mut fixture = LiveJobFixture::new();
    fixture.request_cancel();
    fixture.finish();
    assert_eq!(
        super::process_identity_state(fixture.path(), fixture.pid()),
        super::ProcessIdentityState::Missing
    );
}
