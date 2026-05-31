use super::*;

#[test]
fn ps_filter_matches_command_case_insensitively() {
    let row = PsRow {
        pid: 42,
        ppid: 1,
        cpu: 0.0,
        mem: 0.0,
        comm: "clawd".to_string(),
    };

    assert!(ps_row_matches_filter(&row, Some("CLAWD")));
    assert!(!ps_row_matches_filter(&row, Some("telegramd")));
}

#[test]
fn command_output_filter_keeps_exit_and_matching_rows() {
    let text =
        "exit=0\nLISTEN 0 128 0.0.0.0:8787 users:((\"clawd\",pid=1))\nLISTEN 0 128 0.0.0.0:5432";

    let filtered = filter_command_output(text, Some("8787"));

    assert!(filtered.starts_with("exit=0"));
    assert!(filtered.contains("8787"));
    assert!(!filtered.contains("5432"));
}
