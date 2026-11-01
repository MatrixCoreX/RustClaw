use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("px-{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn directory(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&path)
            .unwrap();
        path.canonicalize().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn seatbelt_path_encoding_preserves_valid_data_and_rejects_profile_line_breaks() {
    let path = Path::new("/tmp/valid \"quoted\"\\path-\u{4e2d}-\u{fffd}");
    assert_eq!(
        seatbelt_path_literal(path).unwrap(),
        "/tmp/valid \\\"quoted\\\"\\\\path-\u{4e2d}-\u{fffd}"
    );
    for invalid in ["/tmp/nul\0path", "/tmp/line\npath", "/tmp/line\rpath"] {
        assert_eq!(
            seatbelt_path_literal(Path::new(invalid)),
            Err("sandbox_path_invalid")
        );
    }
    let composed = Path::new("/tmp/\u{e9}");
    let decomposed = Path::new("/tmp/e\u{301}");
    assert_ne!(
        seatbelt_path_literal(composed),
        seatbelt_path_literal(decomposed)
    );
}

#[test]
fn seatbelt_path_encoding_rejects_non_utf8_without_replacement() {
    use std::os::unix::ffi::OsStringExt;
    let raw = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/raw-\xff".to_vec()));
    let replacement = Path::new("/tmp/raw-\u{fffd}");
    assert_ne!(raw, replacement);
    assert_eq!(
        seatbelt_path_literal(replacement).unwrap(),
        "/tmp/raw-\u{fffd}"
    );
    assert_eq!(seatbelt_path_literal(&raw), Err("sandbox_path_invalid"));
    for bytes in [vec![0xc0, 0xaf], vec![0xed, 0xa0, 0x80], vec![0xe2, 0x82]] {
        let raw = PathBuf::from(std::ffi::OsString::from_vec(bytes));
        assert_eq!(seatbelt_path_literal(&raw), Err("sandbox_path_invalid"));
    }
}

#[test]
fn private_directories_are_canonical_bounded_and_never_replace_missing_state() {
    let root = Fixture::new();
    let private = root.directory("private");
    let missing = root.0.join("missing");
    assert!(matches!(
        DirectoryExclusions::admit(&[missing.clone()], &root.0),
        Err("sandbox_private_directory_unavailable")
    ));
    assert!(!missing.exists());
    assert!(DirectoryExclusions::admit(&[PathBuf::from("relative")], &root.0).is_err());
    assert!(DirectoryExclusions::admit(&vec![private.clone(); 33], &root.0).is_err());
    assert!(DirectoryExclusions::admit(&[root.0.clone()], &private).is_err());
    let file = root.0.join("file");
    fs::write(&file, b"non-secret").unwrap();
    assert!(DirectoryExclusions::admit(&[file], &root.0).is_err());
    let link = root.0.join("link");
    std::os::unix::fs::symlink(&private, &link).unwrap();
    assert!(DirectoryExclusions::admit(&[link], &root.0).is_err());
    fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(DirectoryExclusions::admit(&[private], &root.0).is_err());
}

#[test]
fn private_directories_deduplicate_ancestors_without_string_prefix_overreach() {
    let root = Fixture::new();
    let private = root.directory("private");
    let child = root.directory("private/child");
    let public = root.directory("private-sibling");
    let exclusions =
        DirectoryExclusions::admit(&[child, private.clone(), private.clone()], &root.0).unwrap();
    assert_eq!(exclusions.roots, vec![private.canonicalize().unwrap()]);
    assert_eq!(exclusions.check_writable(&public), Ok(()));
    assert_eq!(
        exclusions.check_writable(&private),
        Err("sandbox_private_directory_writable")
    );
    assert!(exclusions
        .check_writable(&private.join("future-child"))
        .is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn private_mount_targets_cover_all_ancestor_aliases_after_writable_binds() {
    let root = Fixture::new();
    let private = root.directory("workspace/private");
    let workspace = private.parent().unwrap().to_path_buf();
    let exclusions = DirectoryExclusions::admit(&[private.clone()], &workspace).unwrap();
    let first = PathBuf::from("/tmp/agent-runtime-test-alias/0");
    let second = PathBuf::from("/tmp/agent-runtime-test-alias/1");
    let targets = exclusions
        .mount_targets(&[
            (workspace.clone(), first.clone()),
            (workspace.clone(), second.clone()),
            (workspace.clone(), workspace.clone()),
        ])
        .unwrap();
    assert_eq!(
        targets,
        vec![private, first.join("private"), second.join("private")]
    );
    assert!(exclusions
        .mount_targets(&[(workspace.clone(), exclusions.roots[0].clone())])
        .is_err());
    assert!(exclusions
        .mount_targets(&[(exclusions.roots[0].clone(), first)])
        .is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn private_mount_anchors_cover_nested_ancestors_aliases_and_shared_parents_once() {
    let root = Fixture::new();
    let first = root.directory("workspace/parent/deeper/private");
    let second = root.directory("workspace/parent/other-private");
    let workspace = root.0.join("workspace");
    let exclusions = DirectoryExclusions::admit(&[first, second], &workspace).unwrap();
    let alias = PathBuf::from("/tmp/agent-runtime-test-alias/0");
    let mappings = [
        (workspace.clone(), workspace.clone()),
        (workspace.clone(), alias.clone()),
    ];
    assert_eq!(
        exclusions.mount_anchors(&mappings).unwrap(),
        vec![
            (workspace.join("parent"), workspace.join("parent")),
            (
                workspace.join("parent/deeper"),
                workspace.join("parent/deeper")
            ),
            (workspace.join("parent"), alias.join("parent")),
            (workspace.join("parent/deeper"), alias.join("parent/deeper")),
        ]
    );
    assert!(exclusions.mount_anchors(&[]).unwrap().is_empty());
    assert_eq!(
        exclusions
            .mount_anchors(&vec![mappings[0].clone(); 129])
            .err(),
        Some("sandbox_private_mount_limit")
    );
    assert!(exclusions
        .mount_anchors(&[(workspace, PathBuf::from("relative"))])
        .is_err());
    assert!(DirectoryExclusions::admit(&[], &root.0)
        .unwrap()
        .mount_anchors(&mappings)
        .unwrap()
        .is_empty());
}

#[test]
fn private_seatbelt_rules_escape_paths_and_reject_lossy_or_multiline_names() {
    let root = Fixture::new();
    let private = root.directory("private\"quoted\\path");
    let exclusions = DirectoryExclusions::admit(&[private], &root.0).unwrap();
    let rules = exclusions.seatbelt_rules().unwrap();
    assert!(rules.contains("(deny file-read* (subpath \""));
    assert!(rules.contains("(deny file-write* (subpath \""));
    assert!(rules.contains("\\\"quoted\\\\path"));
    for name in ["line\nbreak", "line\rbreak"] {
        let path = root.0.join(name);
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        assert_eq!(
            DirectoryExclusions::admit(&[path], &root.0)
                .unwrap()
                .seatbelt_rules()
                .err(),
            Some("sandbox_private_directory_invalid")
        );
    }
}

#[test]
fn private_seatbelt_rules_reject_non_utf8_without_requiring_filesystem_support() {
    use std::os::unix::ffi::OsStringExt;
    let root = Fixture::new();
    // APFS rejects these filenames at creation. Profile encoding must reject
    // the same raw values on every backend, without lossy replacement.
    for bytes in [vec![0xff], vec![0xc0, 0xaf], vec![0xe2, 0x82]] {
        let path = root.0.join(std::ffi::OsString::from_vec(bytes));
        let exclusions = DirectoryExclusions { roots: vec![path] };
        assert_eq!(
            exclusions.seatbelt_rules().err(),
            Some("sandbox_private_directory_invalid")
        );
    }
}
