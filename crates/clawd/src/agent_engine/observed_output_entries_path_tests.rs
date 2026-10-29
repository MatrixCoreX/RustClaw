#[cfg(unix)]
#[test]
fn canonical_containment_accepts_root_alias_but_rejects_symlink_escape() {
    use super::{artifact_is_within_workspace, CatalogWorkspace};
    let root = CatalogWorkspace::new();
    let physical = root.0.join("physical");
    std::fs::create_dir(&physical).unwrap();
    let artifact = physical.join("artifact.json");
    std::fs::write(&artifact, "{}").unwrap();
    let alias = root.0.join("alias");
    std::os::unix::fs::symlink(&physical, &alias).unwrap();
    // macOS temporary roots may already contain an equivalent symlink alias.
    assert!(artifact_is_within_workspace(&artifact, &alias));
    assert!(artifact_is_within_workspace(
        &alias.join("artifact.json"),
        &physical
    ));
    let outside = root.0.join("outside.json");
    std::fs::write(&outside, "{}").unwrap();
    let escape = physical.join("escape.json");
    std::os::unix::fs::symlink(&outside, &escape).unwrap();
    assert!(escape.starts_with(&physical));
    assert!(!artifact_is_within_workspace(&escape, &physical));
    assert!(!artifact_is_within_workspace(&outside, &alias));
    assert!(!artifact_is_within_workspace(
        &root.0.join("missing"),
        &alias
    ));
}
