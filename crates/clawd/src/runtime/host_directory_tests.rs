use super::*;
use crate::runtime::host_filesystem::HostFilesystemPolicy;
use std::fs;
use std::os::unix::fs::symlink;

#[test]
fn directory_iterator_defers_metadata_until_the_entry_is_requested() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("marker");
    fs::write(&marker, b"old").unwrap();
    let policy = HostFilesystemPolicy::without_private_authority();
    let directory = policy
        .open_directory(Some(root.path()), root.path())
        .unwrap();
    let mut entries = directory.entries_iter().unwrap();
    fs::write(&marker, b"new-longer-content").unwrap();
    let entry = entries.next().unwrap().unwrap();
    assert_eq!(entry.name, "marker");
    assert_eq!(entry.size_bytes, Some(b"new-longer-content".len() as u64));
    assert!(entries.next().is_none());
}

#[test]
fn directory_iterator_keeps_its_admitted_inode_after_path_replacement() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("directory");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("original"), b"original").unwrap();
    let policy = HostFilesystemPolicy::without_private_authority();
    let directory = policy.open_directory(Some(root.path()), &original).unwrap();
    let entries = directory.entries_iter().unwrap();
    fs::rename(&original, root.path().join("moved")).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("replacement"), b"replacement").unwrap();
    let entries = entries.collect::<io::Result<Vec<_>>>().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "original");
    assert_eq!(entries[0].size_bytes, Some(8));
    assert_eq!(
        fs::read(original.join("replacement")).unwrap(),
        b"replacement"
    );
}

#[test]
fn directory_iterator_retains_no_follow_metadata_and_excludes_dot_entries() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file"), b"content").unwrap();
    fs::create_dir(root.path().join("directory")).unwrap();
    symlink("missing-target", root.path().join("link")).unwrap();
    let policy = HostFilesystemPolicy::without_private_authority();
    let directory = policy
        .open_directory(Some(root.path()), root.path())
        .unwrap();
    assert_eq!(directory.entries_iter().unwrap().take(0).count(), 0);
    let mut entries = directory
        .entries_iter()
        .unwrap()
        .collect::<io::Result<Vec<_>>>()
        .unwrap();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].file_type, FileType::Directory);
    assert_eq!(entries[0].size_bytes, None);
    assert_eq!(entries[1].file_type, FileType::RegularFile);
    assert_eq!(entries[1].size_bytes, Some(7));
    assert_eq!(entries[2].file_type, FileType::Symlink);
    assert_eq!(entries[2].size_bytes, None);
    assert_eq!(directory.entries().unwrap().len(), 3);
}
