use super::{prepare_macos_seatbelt, SandboxNetwork};
use std::path::Path;

#[test]
fn seatbelt_profile_allows_only_the_current_system_temp_subtree_for_temp_writes() {
    let prepared = prepare_macos_seatbelt(
        Path::new("/usr/bin/true"),
        Path::new("/"),
        &[],
        SandboxNetwork::Deny,
    )
    .expect("prepare Seatbelt command");
    let arguments = prepared
        .command
        .get_args()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let profile = arguments.get(1).expect("Seatbelt profile argument");
    let temp_directory =
        std::fs::canonicalize(std::env::temp_dir()).expect("canonical macOS temporary directory");

    assert!(profile.contains(&format!(
        "(allow file-write* (subpath \"{}\"))",
        temp_directory.display()
    )));
    assert!(profile.contains("(allow default)"));
    assert!(profile.contains("(deny file-write*)"));
    assert!(profile.contains("(deny network*)"));
}
