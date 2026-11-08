use std::path::PathBuf;

pub(super) fn isolate_workspace(state: &mut crate::AppState) {
    let workspace = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "retention-maintenance-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let mut directory = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(&workspace).unwrap();
    println!(
        "retention_cleanup_fixture_workspace={}",
        workspace.display()
    );
    // Retain the owned workspace as evidence; never clean the shared temp root.
    state.skill_rt.workspace_root = workspace.clone();
    state.skill_rt.default_locator_search_dir = workspace;
}

#[test]
fn cleanup_fixture_workspaces_are_private_and_distinct() {
    let first = super::test_state();
    let second = super::test_state();
    assert_ne!(
        first.skill_rt.workspace_root,
        second.skill_rt.workspace_root
    );
    for state in [&first, &second] {
        let path = &state.skill_rt.workspace_root;
        assert!(path.is_dir());
        assert_eq!(path.canonicalize().unwrap(), *path);
        assert_eq!(
            path.parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        assert_eq!(path, &state.skill_rt.default_locator_search_dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o700);
        }
    }
}

fn orphan_artifact(state: &crate::AppState) -> PathBuf {
    let path = claw_core::workspace_state::workspace_artifacts_root(&state.skill_rt.workspace_root)
        .join("delivery")
        .join("unreferenced-fixture-task")
        .join("fixture-artifact.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"owned-cleanup-fixture-artifact").unwrap();
    path
}

#[test]
fn cleanup_fixture_removes_only_its_own_orphan_artifacts() {
    let first = super::test_state();
    let second = super::test_state();
    let own = orphan_artifact(&first);
    let other = orphan_artifact(&second);
    assert!(own.is_file() && other.is_file());

    super::cleanup_once(&first).unwrap();
    assert!(!own.exists(), "own orphan is the positive cleanup control");
    assert_eq!(
        std::fs::read(&other).unwrap(),
        b"owned-cleanup-fixture-artifact"
    );

    super::cleanup_once(&second).unwrap();
    assert!(!other.exists());
}
