use std::ffi::OsString;

use super::*;

#[test]
fn authority_mode_requires_its_exact_flag_without_extra_arguments() {
    assert!(helper_selected_for_flag([OsString::from(HELPER_FLAG)], HELPER_FLAG).unwrap());
    assert!(!helper_selected_for_flag([OsString::from("--process-owner")], HELPER_FLAG).unwrap());
    assert!(
        !helper_selected_for_flag([OsString::from("--process-authority-extra")], HELPER_FLAG)
            .unwrap()
    );
    assert!(helper_selected_for_flag(
        [OsString::from(HELPER_FLAG), OsString::from("extra")],
        HELPER_FLAG
    )
    .is_err());
}

#[cfg(target_os = "linux")]
#[path = "local_process_authority_process_tests.rs"]
mod process;

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires the source-pinned non-test executable; selected explicitly by native acceptance"]
fn entry_executable_authority_acceptance() {
    process::run();
}
