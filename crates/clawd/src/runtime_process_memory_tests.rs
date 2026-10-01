use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::{aggregate_selected, process_role, runtime_descendants, ProcessRow};

fn row(pid: u32, parent_pid: u32, executable: &str) -> ProcessRow {
    ProcessRow {
        pid,
        parent_pid,
        executable: PathBuf::from(executable),
    }
}

#[test]
fn descendants_include_runtime_children_but_not_unrelated_processes() {
    let rows = BTreeMap::from([
        (10, row(10, 1, "/srv/bin/clawd")),
        (11, row(11, 10, "/srv/bin/skill-runner")),
        (12, row(12, 11, "/usr/bin/python3")),
        (20, row(20, 1, "/usr/bin/python3")),
    ]);
    let selected = runtime_descendants(&rows, &BTreeSet::from([10]));
    assert_eq!(selected, BTreeSet::from([10, 11, 12]));
}

#[test]
fn aggregation_reports_bounded_role_totals_without_process_details() {
    let rows = BTreeMap::from([
        (10, row(10, 1, "/srv/bin/clawd")),
        (11, row(11, 10, "/srv/bin/skill-runner")),
        (12, row(12, 11, "/usr/bin/python3")),
    ]);
    let bytes = BTreeMap::from([(10, 100_u64), (11, 20), (12, 30)]);
    let sample = aggregate_selected(&rows, &BTreeSet::from([10, 11, 12]), "fixture", |pid| {
        bytes.get(&pid).copied()
    });
    assert_eq!(sample.process_count, 3);
    assert_eq!(sample.resident_and_swap_bytes, 150);
    assert_eq!(sample.roles.get("core"), Some(&100));
    assert_eq!(sample.roles.get("skill_runner"), Some(&20));
    assert_eq!(sample.roles.get("skill_process"), Some(&30));
}

#[test]
fn roles_are_based_on_executable_identity_not_arguments_or_natural_language() {
    assert_eq!(process_role(&PathBuf::from("/bin/webd")), "web_gateway");
    assert_eq!(process_role(&PathBuf::from("/bin/telegramd")), "channel");
    assert_eq!(process_role(&PathBuf::from("/bin/chromium")), "browser");
    assert_eq!(process_role(&PathBuf::from("/bin/worker")), "child_process");
}

#[test]
fn current_process_is_measurable_on_supported_hosts() {
    let sample = super::collect_runtime_process_memory(std::path::Path::new("."))
        .expect("current runtime process memory sample");
    assert!(sample.process_count >= 1);
    assert!(sample.resident_and_swap_bytes > 0);
    assert!(!sample.measurement.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_status_parser_handles_status_and_smaps_units() {
    let fixture = "Name:\ttest\nPPid:\t42\nPss:\t123 kB\nSwapPss:\t7 kB\n";
    assert_eq!(super::linux_status_kib(fixture, "PPid"), Some(42));
    assert_eq!(super::linux_status_kib(fixture, "Pss"), Some(123));
    assert_eq!(super::linux_status_kib(fixture, "SwapPss"), Some(7));
    assert_eq!(super::linux_status_kib(fixture, "VmRSS"), None);
}
