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

    fn full_materialized_reference(&self) -> Vec<Value> {
        let mut entries = self
            .filesystem
            .open_directory(Some(&self.root), &self.root)
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
                    "path": self.root.join(&entry.name).display().to_string(),
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
    let expected = fixture.full_materialized_reference();
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
