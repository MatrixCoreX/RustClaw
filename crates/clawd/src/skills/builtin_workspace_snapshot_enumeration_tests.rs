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

#[test]
fn snapshot_enumeration_deep_tree_completes_on_an_owned_small_stack() {
    let fixture = Fixture::new(&[]);
    let root = fixture.root.path().canonicalize().unwrap();
    let mut path = root.join("tree");
    for _ in 0..256 {
        path.push("n");
        fs::create_dir(&path).unwrap();
    }
    fs::write(path.join("value"), b"owned-deep-tree").unwrap();
    let module = module_path!().split_once("::").unwrap().1;
    let case = format!("{module}::snapshot_stack_isolated_fixture");
    let output = run_stack_fixture(&root, &case);
    assert_eq!(
        output.status.code(),
        Some(42),
        "owned stack fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["snapshot_stack_complete"] == true)
        .expect("actual traversal completion report");
    assert_eq!(report["entries"], 258);
    assert_eq!(report["sha256"], sha256_label(b"owned-deep-tree"));
    assert_eq!(fs::read(path.join("value")).unwrap(), b"owned-deep-tree");
    let absent = run_stack_fixture(&root, &format!("{module}::missing_stack_fixture"));
    assert!(absent.status.success());
    assert_ne!(absent.status.code(), Some(42));
    assert!(!String::from_utf8_lossy(&absent.stdout).contains("snapshot_stack_complete"));
}

fn run_stack_fixture(root: &Path, case: &str) -> std::process::Output {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            case,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("APP_SNAPSHOT_STACK_FIXTURE_ROOT", root)
        .env("RUST_BACKTRACE", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "owned stack fixture exceeded its test deadline: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.wait_with_output().unwrap()
}

#[test]
#[ignore = "Executed exactly by the guarded small-stack parent regression"]
fn snapshot_stack_isolated_fixture() {
    let root = PathBuf::from(std::env::var_os("APP_SNAPSHOT_STACK_FIXTURE_ROOT").unwrap());
    let (count, digest) = std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            let filesystem = HostFilesystemPolicy::without_private_authority();
            let entries = capture_snapshot_with_entry_limit(&root, "tree", None, &filesystem, 258)
                .expect("deep capture within the entry budget");
            assert_eq!(entries.len(), 258);
            let leaf = entries.iter().find(|entry| entry.kind == "file").unwrap();
            assert_eq!(leaf.sha256, Some(sha256_label(b"owned-deep-tree")));
            (entries.len(), leaf.sha256.clone().unwrap())
        })
        .unwrap()
        .join()
        .unwrap();
    println!(
        "{}",
        json!({"snapshot_stack_complete": true, "entries": count, "sha256": digest})
    );
    std::process::exit(42);
}
