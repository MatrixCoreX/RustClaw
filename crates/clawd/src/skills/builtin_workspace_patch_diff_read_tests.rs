use super::*;

const PATCH: &str = "diff --git a/src/a.txt b/src/a.txt\n--- a/src/a.txt\n+++ b/src/a.txt\n@@ -1 +1 @@\n-before\n+after\n";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    checkpoint: PathBuf,
    filesystem: HostFilesystemPolicy,
}

impl Fixture {
    fn new(git: bool) -> Self {
        let temporary = tempfile::Builder::new()
            .prefix("agent-runtime-patch-diff-io-")
            .tempdir()
            .unwrap();
        let root = temporary.path().canonicalize().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/a.txt"), b"before\n").unwrap();
        if git {
            assert!(Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["init", "--quiet"])
                .output()
                .unwrap()
                .status
                .success());
        }
        let filesystem = HostFilesystemPolicy::without_private_authority();
        let output = execute_workspace_patch_with_policy(
            &root,
            "owned-diff-reader",
            serde_json::json!({"action":"apply_patch","patch":PATCH})
                .as_object()
                .unwrap(),
            &filesystem,
            ToolSandboxBackend::Auto,
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["status"], "ok");
        assert_eq!(value["action"], "apply_patch");
        let checkpoint =
            resolve_checkpoint_dir(&root, value["checkpoint_id"].as_str().unwrap(), &filesystem)
                .unwrap();
        assert_eq!(
            read_manifest(&checkpoint, &filesystem).unwrap().state,
            "applied"
        );
        assert_eq!(fs::read(root.join("src/a.txt")).unwrap(), b"after\n");
        Self {
            _temporary: temporary,
            root,
            checkpoint,
            filesystem,
        }
    }

    fn read(&self, filesystem: &HostFilesystemPolicy) -> Result<String, String> {
        saved_diff(&self.root, &self.checkpoint, filesystem)
    }

    fn assert_current(&self) {
        assert_eq!(fs::read(self.root.join("src/a.txt")).unwrap(), b"after\n");
        assert_eq!(
            read_manifest(&self.checkpoint, &self.filesystem)
                .unwrap()
                .state,
            "applied"
        );
    }
}

fn saved_diff(
    root: &Path,
    checkpoint: &Path,
    filesystem: &HostFilesystemPolicy,
) -> Result<String, String> {
    let id = checkpoint.file_name().unwrap().to_str().unwrap();
    execute_workspace_patch_with_policy(
        root,
        "owned-diff-reader",
        serde_json::json!({"action":"diff","checkpoint_id":id})
            .as_object()
            .unwrap(),
        filesystem,
        ToolSandboxBackend::Auto,
    )
}

fn error(result: Result<String, String>) -> (String, Value) {
    let parsed = crate::skills::parse_structured_skill_error(
        &result.err().expect("invalid saved patch must be refused"),
    )
    .unwrap();
    (parsed.error_code, parsed.extra.unwrap())
}

#[test]
fn saved_diff_native_and_git_producers_preserve_the_original_patch() {
    for git in [false, true] {
        let fixture = Fixture::new(git);
        let output: Value =
            serde_json::from_str(&fixture.read(&fixture.filesystem).unwrap()).unwrap();
        assert_eq!(output["action"], "diff");
        assert_eq!(output["patch"], PATCH);
        assert_eq!(output["patch_id"], sha256_label(PATCH.as_bytes()));
        fixture.assert_current();
    }
}

#[test]
fn saved_diff_exact_reader_boundary_and_overflow_remain_distinct() {
    for length in [MAX_PATCH_BYTES, MAX_PATCH_BYTES + 1] {
        let fixture = Fixture::new(false);
        let bytes = vec![b'x'; length];
        let mut manifest = read_manifest(&fixture.checkpoint, &fixture.filesystem).unwrap();
        manifest.patch_id = sha256_label(&bytes);
        write_manifest(&fixture.checkpoint, &manifest, &fixture.filesystem).unwrap();
        fs::write(fixture.checkpoint.join("change.patch"), &bytes).unwrap();
        let result = fixture.read(&fixture.filesystem);
        if length == MAX_PATCH_BYTES {
            let output: Value = serde_json::from_str(&result.unwrap()).unwrap();
            assert_eq!(output["patch"].as_str().unwrap().as_bytes(), bytes);
        } else {
            let (code, extra) = error(result);
            assert_eq!(code, "checkpoint_too_large");
            assert_eq!(extra["details"]["max_bytes"], MAX_PATCH_BYTES);
            assert_eq!(
                extra["details"]["patch_bytes_at_least"],
                MAX_PATCH_BYTES + 1
            );
        }
        fixture.assert_current();
        assert_eq!(
            fs::read(fixture.checkpoint.join("change.patch")).unwrap(),
            bytes
        );
    }
}

#[test]
fn saved_diff_missing_and_invalid_utf8_keep_the_read_error_contract() {
    for missing in [false, true] {
        let fixture = Fixture::new(false);
        let path = fixture.checkpoint.join("change.patch");
        if missing {
            fs::remove_file(&path).unwrap();
        } else {
            fs::write(&path, [0xff]).unwrap();
        }
        assert_eq!(
            error(fixture.read(&fixture.filesystem)).0,
            "checkpoint_patch_read_failed"
        );
        fixture.assert_current();
    }
}

#[test]
fn saved_diff_refuses_mismatched_content_without_rewriting_the_checkpoint() {
    let fixture = Fixture::new(false);
    let mut bytes = PATCH.as_bytes().to_vec();
    bytes[0] = b'x';
    fs::write(fixture.checkpoint.join("change.patch"), &bytes).unwrap();
    let (code, extra) = error(fixture.read(&fixture.filesystem));
    assert_eq!(code, "checkpoint_patch_hash_mismatch");
    assert_eq!(
        extra["details"]["expected_patch_id"],
        sha256_label(PATCH.as_bytes())
    );
    assert_eq!(extra["details"]["observed_patch_id"], sha256_label(&bytes));
    assert_eq!(
        fs::read(fixture.checkpoint.join("change.patch")).unwrap(),
        bytes
    );
    fixture.assert_current();
}

#[test]
fn saved_diff_keeps_private_target_checkpoint_and_nonregular_admission() {
    for checkpoint in [false, true] {
        let fixture = Fixture::new(false);
        let private = if checkpoint {
            fixture.checkpoint.clone()
        } else {
            fixture.root.join("src")
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let filesystem = HostFilesystemPolicy::pin_directories(&[private], &fixture.root).unwrap();
        assert_eq!(
            error(fixture.read(&filesystem)).0,
            "host_private_directory_access_denied"
        );
        fixture.assert_current();
    }
    let fixture = Fixture::new(false);
    let target = fixture.root.join("src/a.txt");
    fs::remove_file(&target).unwrap();
    fs::create_dir(&target).unwrap();
    assert!(fixture.read(&fixture.filesystem).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        fs::remove_dir(&target).unwrap();
        let sentinel = fixture.root.join("sentinel.txt");
        fs::write(&sentinel, b"keep-owned\n").unwrap();
        symlink(&sentinel, &target).unwrap();
        assert!(fixture.read(&fixture.filesystem).is_err());
        assert_eq!(fs::read(sentinel).unwrap(), b"keep-owned\n");
    }
}

#[test]
fn saved_diff_still_reports_the_patch_when_the_current_file_is_missing() {
    let fixture = Fixture::new(false);
    fs::remove_file(fixture.root.join("src/a.txt")).unwrap();
    let output: Value = serde_json::from_str(&fixture.read(&fixture.filesystem).unwrap()).unwrap();
    assert_eq!(output["patch"], PATCH);
    assert!(!fixture.root.join("src/a.txt").exists());
}

#[test]
fn saved_diff_malformed_digest_diagnostics_never_echo_unbounded_metadata() {
    for invalid in [
        String::new(),
        "invalid_digest_token".repeat(100),
        format!("sha256:{}", "x".repeat(64)),
    ] {
        let fixture = Fixture::new(false);
        let mut manifest = read_manifest(&fixture.checkpoint, &fixture.filesystem).unwrap();
        manifest.patch_id = invalid;
        write_manifest(&fixture.checkpoint, &manifest, &fixture.filesystem).unwrap();
        let (code, extra) = error(fixture.read(&fixture.filesystem));
        assert_eq!(code, "checkpoint_patch_hash_mismatch");
        assert!(extra["details"]["expected_patch_id"].is_null());
        assert_eq!(extra["details"]["expected_patch_id_valid"], false);
        assert_eq!(
            extra["details"]["observed_patch_id"],
            sha256_label(PATCH.as_bytes())
        );
        assert!(serde_json::to_vec(&extra).unwrap().len() < 1024);
        fixture.assert_current();
    }
}

#[cfg(target_os = "linux")]
fn assert_diff_memory_bound(test_name: &str, large_patch: bool) {
    use std::os::unix::fs::MetadataExt;

    const CHILD_ROOT: &str = "APP_TEST_PATCH_DIFF_IO_ROOT";
    const LENGTH: u64 = 512 * 1024 * 1024;
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root).canonicalize().unwrap();
        assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("agent-runtime-patch-diff-io-"));
        let metadata = fs::metadata(&root).unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o7777, 0o700);
        let checkpoints = fs::read_dir(
            claw_core::workspace_state::workspace_state_root(&root).join("checkpoints"),
        )
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
        assert_eq!(checkpoints.len(), 1);
        let checkpoint = &checkpoints[0];
        let virtual_bytes = fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmSize:").map(|value| {
                    value
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                        * 1024
                })
            })
            .unwrap();
        let mut inherited = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut inherited) },
            0
        );
        let ceiling = virtual_bytes
            .checked_add(64 * 1024 * 1024)
            .unwrap()
            .min(inherited.rlim_cur);
        assert!(ceiling > virtual_bytes + 4 * 1024 * 1024);
        let no_core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        let address = libc::rlimit {
            rlim_cur: ceiling,
            rlim_max: ceiling,
        };
        // Limits belong only to this independently executed fixture child.
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_CORE, &no_core) }, 0);
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_AS, &address) }, 0);
        let filesystem = HostFilesystemPolicy::without_private_authority();
        let result = saved_diff(&root, checkpoint, &filesystem);
        let result_code = if large_patch {
            let (code, extra) = error(result);
            assert_eq!(code, "checkpoint_too_large");
            assert_eq!(
                extra["details"]["patch_bytes_at_least"],
                MAX_PATCH_BYTES + 1
            );
            code
        } else {
            let output: Value = serde_json::from_str(&result.unwrap()).unwrap();
            assert_eq!(output["patch"], PATCH);
            "ok".to_string()
        };
        let mut prefix = [0; 6];
        fs::File::open(root.join("src/a.txt"))
            .unwrap()
            .read_exact(&mut prefix)
            .unwrap();
        assert_eq!(&prefix, b"after\n");
        assert_eq!(
            read_manifest(checkpoint, &filesystem).unwrap().state,
            "applied"
        );
        let completed = serde_json::json!({"schema_version":1,"test_name":test_name,
            "large_patch":large_patch,"result_code":result_code,
            "virtual_bytes_before":virtual_bytes,"address_limit_bytes":ceiling});
        fs::write(
            root.join("diff-read-result.json"),
            serde_json::to_vec(&completed).unwrap(),
        )
        .unwrap();
        return;
    }
    let fixture = Fixture::new(false);
    let large = if large_patch {
        fixture.checkpoint.join("change.patch")
    } else {
        fixture.root.join("src/a.txt")
    };
    fs::OpenOptions::new()
        .write(true)
        .open(&large)
        .unwrap()
        .set_len(LENGTH)
        .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--test-threads=1", "--nocapture"])
        .env(CHILD_ROOT, &fixture.root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "owned diff child failed: {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let completed: Value = serde_json::from_slice(
        &fs::read(fixture.root.join("diff-read-result.json"))
            .expect("selected child must complete the diff proof"),
    )
    .unwrap();
    assert_eq!(completed["schema_version"], 1);
    assert_eq!(completed["test_name"], test_name);
    assert_eq!(completed["large_patch"], large_patch);
    assert_eq!(
        completed["result_code"],
        if large_patch {
            "checkpoint_too_large"
        } else {
            "ok"
        }
    );
    assert_eq!(fs::metadata(large).unwrap().len(), LENGTH);
    if large_patch {
        fixture.assert_current();
    }
    println!("saved_diff_read_case={completed}");
}

#[cfg(target_os = "linux")]
#[test]
fn saved_diff_large_payload_is_not_materialized() {
    assert_diff_memory_bound(
        "skills::builtin::builtin_workspace_patch::diff_read_tests::saved_diff_large_payload_is_not_materialized",
        false,
    );
}

#[cfg(target_os = "linux")]
#[test]
fn saved_diff_large_stored_patch_is_bounded_before_materialization() {
    assert_diff_memory_bound(
        "skills::builtin::builtin_workspace_patch::diff_read_tests::saved_diff_large_stored_patch_is_bounded_before_materialization",
        true,
    );
}
