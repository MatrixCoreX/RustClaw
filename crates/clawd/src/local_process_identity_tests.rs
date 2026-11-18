use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use super::{process_identity_state, ProcessIdentityState, TempDirGuard};

#[test]
fn live_orphan_group_cannot_verify_an_unrelated_job() {
    let owner = TempDirGuard::new("orphan_group_owner");
    let unrelated = TempDirGuard::new("orphan_group_unrelated");
    let script = owner.path().join("run.sh");
    let child_record = owner.path().join("child_pid");
    let child_ready = owner.path().join("child_ready");
    let child_parent = owner.path().join("child_parent");
    std::fs::write(
        &script,
        "#!/usr/bin/env bash\nexec 3<&0\nbash -c 'printf %s \"$PPID\" > \"$1\"; printf ready > \"$2\"; read -r _ || :' child \"$3\" \"$2\" <&3 >/dev/null 2>&1 &\nprintf '%s' \"$!\" > \"$1\"\nfor ((i=0;i<1000;i++)); do [[ -s \"$2\" ]] && exit 0; sleep 0.01; done\nexit 9\n",
    )
    .unwrap();
    let mut leader = Command::new("bash")
        .arg(&script)
        .arg(&child_record)
        .arg(&child_ready)
        .arg(&child_parent)
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let group_id = leader.id();
    assert!(group_id > 1);
    // Closing this pipe on return or panic ends only the fixture's child.
    let _owned_input = leader.stdin.take().unwrap();
    assert!(leader.wait().unwrap().success());
    assert_eq!(std::fs::read(&child_ready).unwrap(), b"ready");
    assert_eq!(
        std::fs::read_to_string(&child_parent)
            .unwrap()
            .parse::<u32>()
            .unwrap(),
        group_id
    );
    let child_pid: u32 = std::fs::read_to_string(&child_record)
        .unwrap()
        .parse()
        .unwrap();
    let child = crate::local_process_owner::instance::observe_process(child_pid).unwrap();
    assert_eq!(
        child.state,
        crate::local_process_owner::instance::ProcessRunState::Active
    );
    assert_eq!(child.process_group, group_id);

    std::fs::write(unrelated.path().join("pid"), group_id.to_string()).unwrap();
    // Only observational '-0' probes, never destructive signals.
    let observed = process_identity_state(unrelated.path(), group_id);
    assert_ne!(observed, ProcessIdentityState::AliveVerified);
}
