use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) fn assert_oversized_metadata_is_bounded(
    test_name: &str,
    read: impl FnOnce(&Path) -> Result<(), String>,
) {
    const CHILD_ROOT: &str = "APP_TEST_CHECKPOINT_METADATA_CHILD_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root).canonicalize().unwrap();
        assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("agent-runtime-checkpoint-metadata-"));
        let metadata = fs::metadata(&root).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o7777, 0o700);

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
        let address_limit = libc::rlimit {
            rlim_cur: ceiling,
            rlim_max: ceiling,
        };
        let no_core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // This branch runs only in the independently executed owned test child.
        // Never alter the parent verifier or a production daemon's limits.
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_CORE, &no_core) }, 0);
        assert_eq!(
            unsafe { libc::setrlimit(libc::RLIMIT_AS, &address_limit) },
            0
        );
        let error = read(&root).expect_err("oversized checkpoint must be refused");
        let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
        assert_eq!(parsed.error_code, "checkpoint_too_large");
        let result = serde_json::json!({"schema_version":1,"test_name":test_name,
            "error_code":parsed.error_code,"virtual_bytes_before":virtual_bytes,
            "address_limit_bytes":ceiling});
        fs::write(
            root.join("result.json"),
            serde_json::to_vec(&result).unwrap(),
        )
        .unwrap();
        return;
    }

    let temporary = tempfile::Builder::new()
        .prefix("agent-runtime-checkpoint-metadata-")
        .tempdir()
        .unwrap();
    let root = temporary.path().canonicalize().unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let length = 512 * 1024 * 1024;
    fs::File::create(root.join("manifest.json"))
        .unwrap()
        .set_len(length)
        .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--test-threads=1", "--nocapture"])
        .env(CHILD_ROOT, &root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "owned metadata child failed: {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("result.json")).expect("selected child must complete the reader proof"),
    )
    .unwrap();
    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["test_name"], test_name);
    assert_eq!(result["error_code"], "checkpoint_too_large");
    assert_eq!(
        fs::metadata(root.join("manifest.json")).unwrap().len(),
        length
    );
    println!("checkpoint_metadata_read_case={result}");
}
