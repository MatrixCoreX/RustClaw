use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::super::instance::ProcessRunState;
use super::super::owner_group::OwnedChildGroup;
use super::*;

#[test]
fn group_snapshot_can_miss_a_fork_between_enumeration_and_observation() {
    let fixture = SnapshotFixture::new();
    let parent_script = fixture.0.join("parent.sh");
    let child_record = fixture.0.join("child");
    let parent_record = fixture.0.join("parent");
    std::fs::write(
        &parent_script,
        "exec 3<&0\nread -r next\n/bin/sh -c 'read -r finish' <&3 &\nprintf '%s' \"$!\" > \"$1\"\n",
    )
    .unwrap();
    let mut command = Command::new("/bin/sh");
    command
        .args([
            "-c",
            "exec 3<&0; /bin/sh \"$1\" \"$2\" <&3 & printf '%s' \"$!\" > \"$3\"",
            "snapshot-fixture",
        ])
        .arg(&parent_script)
        .arg(&child_record)
        .arg(&parent_record)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut group = OwnedChildGroup::spawn(&mut command).unwrap();
    let mut input = group.take_stdin().unwrap();
    fixture.wait_until(|| group.poll_exit().unwrap().is_some());
    let parent: u32 = std::fs::read_to_string(&parent_record)
        .unwrap()
        .parse()
        .unwrap();
    let candidates = group_candidates(group.id()).unwrap();
    assert!(candidates.contains(&parent));

    input.write_all(b"fork\n").unwrap();
    fixture.wait_until(|| std::fs::metadata(&child_record).is_ok_and(|value| value.len() > 0));
    let child: u32 = std::fs::read_to_string(&child_record)
        .unwrap()
        .parse()
        .unwrap();
    fixture.wait_until(|| match observe_process(parent) {
        Err(ProcessProbeFailure::Missing) => true,
        Ok(snapshot) => snapshot.state == ProcessRunState::Zombie,
        Err(error) => panic!("unexpected parent probe: {error:?}"),
    });
    assert!(!candidates.contains(&child));
    let stale = observe_group_candidates(group.id(), &candidates).unwrap();
    assert!(stale
        .iter()
        .all(|member| member.state != ProcessRunState::Active));
    assert!(group_members(group.id())
        .unwrap()
        .iter()
        .any(|member| member.birth.pid == child && member.state == ProcessRunState::Active));
    // This proves a real stale scan, not whole-job completion. Keep the held
    // leader until the fixture's own group is cleaned up by its owner.
}

#[test]
fn group_candidate_observation_preserves_bounds_and_special_target_refusal() {
    for invalid in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
        assert_eq!(
            group_candidates(invalid),
            Err(ProcessProbeFailure::InvalidTarget)
        );
        assert_eq!(
            observe_group_candidates(invalid, &[]),
            Err(ProcessProbeFailure::InvalidTarget)
        );
        assert_eq!(
            observe_group_candidates(42, &[invalid]),
            Err(ProcessProbeFailure::InvalidTarget)
        );
    }
    assert_eq!(
        observe_group_candidates(42, &vec![42; MAX_GROUP_MEMBERS + 1]),
        Err(ProcessProbeFailure::Unavailable)
    );
}

struct SnapshotFixture(std::path::PathBuf);

impl SnapshotFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "agent-group-snapshot-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn wait_until(&self, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(
                Instant::now() < deadline,
                "owned snapshot fixture not ready"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for SnapshotFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
