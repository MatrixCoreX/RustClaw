use super::*;
use crate::runtime::host_filesystem::HostFilesystemPolicy;
use std::fs;

struct Fixture {
    _temporary: tempfile::TempDir,
    root: std::path::PathBuf,
    filesystem: HostFilesystemPolicy,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        Self {
            _temporary: temporary,
            root,
            filesystem: HostFilesystemPolicy::without_private_authority(),
        }
    }

    fn page(&self, cursor: usize, limit: usize) -> Result<Value, ListDirError> {
        list_directory_page(
            ".",
            &self.root,
            Some(&self.root),
            &self.filesystem,
            cursor,
            limit,
        )
    }
}

fn full_materialized_reference(root: &Path, filesystem: &HostFilesystemPolicy) -> Vec<Value> {
    let mut entries = filesystem
        .open_directory(Some(root), root)
        .unwrap()
        .entries()
        .unwrap()
        .into_iter()
        .map(|entry| {
            let kind = match entry.file_type {
                FileType::Symlink => "symlink",
                FileType::Directory => "directory",
                FileType::RegularFile => "file",
                _ => "other",
            };
            json!({
                "name": entry.name.to_string_lossy().into_owned(),
                "path": root.join(&entry.name).display().to_string(),
                "kind": kind,
                "is_symlink": entry.file_type == FileType::Symlink,
                "size_bytes": entry.size_bytes,
                "modified_ts": entry.modified_ts,
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    entries
}

fn reference_hash(entries: &[Value]) -> String {
    let mut digest = Sha256::new();
    for entry in entries {
        digest.update(entry.to_string().as_bytes());
        digest.update([b'\n']);
    }
    format!("{:x}", digest.finalize())
}

fn assert_pages_match_reference(fixture: &Fixture) {
    let expected = full_materialized_reference(&fixture.root, &fixture.filesystem);
    let hash = reference_hash(&expected);
    for cursor in 0..=expected.len() {
        for limit in [0, 1, 3, usize::MAX] {
            let actual = fixture.page(cursor, limit).unwrap();
            let end = cursor.saturating_add(limit).min(expected.len());
            assert_eq!(actual["entries"], json!(expected[cursor..end]));
            assert_eq!(actual["total_count"], expected.len());
            assert_eq!(actual["count"], end - cursor);
            assert_eq!(actual["snapshot_hash"], hash);
            assert_eq!(actual["complete"], end == expected.len());
            assert_eq!(
                actual["names"],
                json!(expected[cursor..end]
                    .iter()
                    .map(|row| row["name"].clone())
                    .collect::<Vec<_>>())
            );
            assert_eq!(actual["requested_path"], ".");
            assert_eq!(actual["canonical_path"], fixture.root.to_str().unwrap());
            if end < expected.len() {
                assert_eq!(actual["next_cursor"], end);
                assert_eq!(actual["continuation"]["args"]["cursor"], end);
                assert_eq!(actual["continuation"]["args"]["path"], ".");
                assert_eq!(actual["continuation"]["args"]["max_entries"], limit);
                assert_eq!(actual["continuation"]["snapshot_hash"], hash);
            } else {
                assert!(actual["next_cursor"].is_null());
                assert!(actual["continuation"].is_null());
            }
        }
    }
    assert!(matches!(
        fixture.page(expected.len() + 1, 1),
        Err(ListDirError::CursorOutOfRange { cursor, total })
            if cursor == expected.len() + 1 && total == expected.len()
    ));
}

#[test]
fn list_dir_empty_directory_retains_complete_hash_and_cursor_contract() {
    assert_pages_match_reference(&Fixture::new());
}

#[test]
fn list_dir_pages_match_materialized_rows_hashes_and_continuations() {
    let fixture = Fixture::new();
    for name in ["z.txt", "a space.txt", "middle", "caf\u{e9}.txt"] {
        fs::write(fixture.root.join(name), name.as_bytes()).unwrap();
    }
    fs::create_dir(fixture.root.join("directory")).unwrap();
    std::os::unix::fs::symlink("z.txt", fixture.root.join("link")).unwrap();
    std::os::unix::fs::symlink("missing", fixture.root.join("dangling")).unwrap();
    assert_pages_match_reference(&fixture);
}

#[cfg(target_os = "linux")]
#[test]
fn list_dir_raw_names_keep_the_existing_lossy_projection_and_stable_hash() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    for name in [b"raw-\xfe".as_slice(), b"other-\xff".as_slice()] {
        fs::write(fixture.root.join(OsString::from_vec(name.to_vec())), b"raw").unwrap();
    }
    assert_pages_match_reference(&fixture);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn list_dir_wide_page_reports_native_peak_memory_against_materialized_reference() {
    let fixture = Fixture::new();
    for index in 0..20_000 {
        fs::File::create_new(fixture.root.join(format!("entry-{index:05}"))).unwrap();
    }
    let optimized = memory_probe(&fixture.root, "page");
    let reference = memory_probe(&fixture.root, "reference");
    assert_eq!(optimized["rows"], 20_000);
    assert_eq!(reference["rows"], 20_000);
    assert_eq!(optimized["snapshot_hash"], reference["snapshot_hash"]);
    assert_eq!(optimized["page_count"], 17);
    assert_eq!(reference["page_count"], 17);
    for report in [&optimized, &reference] {
        assert!(report["peak_resident_bytes"].as_u64().unwrap() > 0);
    }
    println!(
        "LIST_DIR_MEMORY_ACCEPTANCE rows=20000 page_limit=17 optimized_peak_bytes={} reference_peak_bytes={} hash={}",
        optimized["peak_resident_bytes"], reference["peak_resident_bytes"],
        optimized["snapshot_hash"]
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn memory_probe(root: &Path, mode: &str) -> Value {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let module = module_path!().split_once("::").unwrap().1;
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            &format!("{module}::list_dir_isolated_memory_probe"),
            "--exact",
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("APP_LIST_DIR_PROBE_ROOT", root)
        .env("APP_LIST_DIR_PROBE_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("owned directory memory fixture did not finish: {result:?}");
            }
        }
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["list_dir_memory_probe"] == true)
        .unwrap_or_else(|| {
            panic!(
                "owned directory report missing: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        });
    assert_eq!(report["mode"], mode);
    report
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
#[ignore = "selected exactly by the owned native memory comparison"]
fn list_dir_isolated_memory_probe() {
    let root = std::path::PathBuf::from(std::env::var_os("APP_LIST_DIR_PROBE_ROOT").unwrap());
    let mode = std::env::var("APP_LIST_DIR_PROBE_MODE").unwrap();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let reference;
    let page;
    let (rows, page_count, hash) = match mode.as_str() {
        "page" => {
            page = list_directory_page(".", &root, Some(&root), &filesystem, 0, 17).unwrap();
            (
                page["total_count"].as_u64().unwrap(),
                page["count"].as_u64().unwrap(),
                page["snapshot_hash"].as_str().unwrap().to_string(),
            )
        }
        "reference" => {
            reference = full_materialized_reference(&root, &filesystem);
            (
                reference.len() as u64,
                reference.len().min(17) as u64,
                reference_hash(&reference),
            )
        }
        _ => panic!("unknown owned memory probe mode"),
    };
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // The kernel initializes the record on success; never read it on error.
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
    let peak = u64::try_from(unsafe { usage.assume_init() }.ru_maxrss).unwrap();
    #[cfg(target_os = "linux")]
    let peak = peak.checked_mul(1024).unwrap();
    println!(
        "\n{}",
        json!({
            "list_dir_memory_probe": true, "mode": mode, "rows": rows,
            "page_count": page_count, "snapshot_hash": hash, "peak_resident_bytes": peak,
        })
    );
    std::process::exit(42);
}
