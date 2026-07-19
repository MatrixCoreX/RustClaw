use super::task_journal_coding_commands::{is_test_command_token, is_verification_command_token};

#[test]
fn recognizes_python_unittest_with_pipeline_and_versioned_interpreter() {
    assert!(is_test_command_token(
        "python3 -m unittest test_calc_core.py -v 2>&1 | tail -50"
    ));
    assert!(is_test_command_token(
        "cd /workspace && /usr/bin/python3.12 -m unittest test_calc.py"
    ));
}

#[test]
fn recognizes_non_test_verification_commands_without_matching_inspection() {
    assert!(is_verification_command_token(
        "cd /workspace && cargo clippy --all-targets"
    ));
    assert!(!is_verification_command_token("git status --short"));
}
