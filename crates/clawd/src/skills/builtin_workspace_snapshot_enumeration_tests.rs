use super::*;

struct Fixture {
    root: tempfile::TempDir,
    filesystem: HostFilesystemPolicy,
}

impl Fixture {
    fn new(files: &[&str]) -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("tree")).unwrap();
        for name in files {
            let path = root.path().join("tree").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name.as_bytes()).unwrap();
        }
        Self {
            root,
            filesystem: HostFilesystemPolicy::without_private_authority(),
        }
    }

    fn capture(&self, max_entries: usize) -> Result<Vec<SnapshotEntry>, String> {
        let root = self.root.path().canonicalize().unwrap();
        capture_snapshot_with_entry_limit(&root, "tree", None, &self.filesystem, max_entries)
    }

    fn assert_limit(&self, limit: usize) {
        let error = self.capture(limit).unwrap_err();
        let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
        assert_eq!(parsed.error_code, "snapshot_limit_exceeded");
        let extra = parsed.extra.unwrap();
        assert_eq!(extra["details"]["max_entries"], limit);
        assert_eq!(extra["details"]["entry_count"], limit + 1);
        assert!(!claw_core::workspace_state::workspace_state_root(self.root.path()).exists());
    }
}

#[test]
fn snapshot_enumeration_accepts_empty_and_exact_entry_budgets() {
    let empty = Fixture::new(&[]);
    let entries = empty.capture(1).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, "directory");

    let fixture = Fixture::new(&["z", "a"]);
    let entries = fixture.capture(3).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["tree", "tree/a", "tree/z"]
    );
    assert_eq!(
        entries[1].sha256.as_deref(),
        Some(sha256_label(b"a").as_str())
    );
    assert_eq!(
        entries[2].sha256.as_deref(),
        Some(sha256_label(b"z").as_str())
    );
}

#[test]
fn snapshot_enumeration_refuses_zero_and_wide_overflow_before_checkpoint_creation() {
    Fixture::new(&[]).assert_limit(0);
    let fixture = Fixture::new(&["a", "b", "c", "d"]);
    fixture.assert_limit(3);
    assert_eq!(
        fs::read_dir(fixture.root.path().join("tree"))
            .unwrap()
            .count(),
        4
    );
    assert_eq!(fs::read(fixture.root.path().join("tree/a")).unwrap(), b"a");
}

#[test]
fn snapshot_enumeration_accounts_for_already_visited_nested_entries() {
    let fixture = Fixture::new(&["a/one", "a/two", "b/one", "b/two"]);
    fixture.assert_limit(5);
    assert_eq!(fixture.capture(7).unwrap().len(), 7);
}

#[test]
fn snapshot_enumeration_reserves_pending_siblings_before_descending() {
    let fixture = Fixture::new(&["a/one", "c", "d"]);
    std::os::unix::fs::symlink("missing", fixture.root.path().join("tree/b")).unwrap();
    // The four root children already exhaust the five-entry budget. Seeing
    // a/one must stop here, before inspecting the later disallowed symlink.
    fixture.assert_limit(5);
    assert_eq!(
        fs::read(fixture.root.path().join("tree/a/one")).unwrap(),
        b"a/one"
    );
    assert!(fixture.root.path().join("tree/b").is_symlink());
}

#[test]
fn snapshot_enumeration_releases_reservations_after_complete_visit() {
    let fixture = Fixture::new(&["a/one", "b"]);
    let root = fixture.root.path().canonicalize().unwrap();
    let mut entries = Vec::new();
    let mut budget = SnapshotBudget {
        entries: 0,
        pending_entries: 0,
        bytes: 0,
        max_entries: 4,
    };
    capture_path(
        &root,
        &root.join("tree"),
        "tree",
        None,
        &mut entries,
        &mut budget,
        &fixture.filesystem,
    )
    .unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(budget.entries, 4);
    assert_eq!(budget.pending_entries, 0);
}
