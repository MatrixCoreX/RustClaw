use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::{link_project_path_alias, resolve_project_identity, unlink_project_path_alias};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "memory-project-identity-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&path).expect("create project fixture root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn setup_db() -> Connection {
    let db = Connection::open_in_memory().expect("project identity db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("auth schema");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("principal schema");
    db
}

#[test]
fn same_git_common_dir_unifies_worktrees_but_different_clones_stay_isolated() {
    let db = setup_db();
    let root = TestDir::new();
    let first = root.path().join("first");
    let first_git = first.join(".git");
    let worktree = root.path().join("worktree");
    let worktree_git_dir = first_git.join("worktrees/w2");
    let second = root.path().join("second");
    fs::create_dir_all(&first_git).expect("first git dir");
    fs::create_dir_all(&worktree).expect("worktree dir");
    fs::create_dir_all(&worktree_git_dir).expect("worktree git dir");
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", worktree_git_dir.display()),
    )
    .expect("worktree git marker");
    fs::write(worktree_git_dir.join("commondir"), "../..\n").expect("commondir marker");
    fs::create_dir_all(second.join(".git")).expect("second git dir");

    let first_id = resolve_project_identity(&db, &first).expect("first project");
    let worktree_id = resolve_project_identity(&db, &worktree).expect("worktree project");
    let second_id = resolve_project_identity(&db, &second).expect("second project");

    assert_eq!(first_id.project_ref, worktree_id.project_ref);
    assert_eq!(first_id.locator_kind, "git_common_dir");
    assert_ne!(first_id.project_ref, second_id.project_ref);
}

#[test]
fn non_git_path_move_requires_explicit_alias_and_unlink_restores_isolation() {
    let db = setup_db();
    let root = TestDir::new();
    let old_path = root.path().join("old");
    let moved_path = root.path().join("moved");
    fs::create_dir_all(&old_path).expect("old path");
    fs::create_dir_all(&moved_path).expect("moved path");
    let old = resolve_project_identity(&db, &old_path).expect("old project");
    let moved_before = resolve_project_identity(&db, &moved_path).expect("moved project before");
    assert_ne!(old.project_ref, moved_before.project_ref);

    link_project_path_alias(&db, &old.project_ref, &moved_path).expect("link moved alias");
    let moved_after = resolve_project_identity(&db, &moved_path).expect("moved project after");
    assert_eq!(old.project_ref, moved_after.project_ref);

    assert!(unlink_project_path_alias(&db, &old.project_ref, &moved_path).expect("unlink alias"));
    let moved_unlinked =
        resolve_project_identity(&db, &moved_path).expect("moved project unlinked");
    assert_ne!(old.project_ref, moved_unlinked.project_ref);
}
