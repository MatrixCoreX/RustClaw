use super::*;
use crate::secure_workspace_fs::tests::TestWorkspace;
use std::fs;

fn target(root: &Path) -> std::path::PathBuf {
    root.join(".agent-runtime/artifacts/canonical-evidence/task/digest.json")
}

#[test]
fn publication_is_private_verified_and_idempotent() {
    let root = TestWorkspace::new("evidence");
    let path = target(root.path());
    publish(root.path(), &path, b"verified").unwrap();
    publish(root.path(), &path, b"verified").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"verified");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[test]
fn corrupt_existing_object_is_not_reused_or_overwritten() {
    let root = TestWorkspace::new("evidence-corrupt");
    let path = target(root.path());
    publish(root.path(), &path, b"verified").unwrap();
    fs::write(&path, b"modified").unwrap();
    assert_eq!(
        publish(root.path(), &path, b"verified").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(fs::read(&path).unwrap(), b"modified");
}

#[cfg(unix)]
#[test]
fn symlink_parent_or_leaf_never_redirects_evidence_or_chmods_external_data() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for leaf in [false, true] {
        let root = TestWorkspace::new("evidence-symlink");
        let outside = TestWorkspace::new("evidence-outside");
        let external = outside.path().join("private");
        fs::write(&external, b"outside").unwrap();
        fs::set_permissions(&external, fs::Permissions::from_mode(0o644)).unwrap();
        let path = target(root.path());
        if leaf {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            symlink(&external, &path).unwrap();
        } else {
            fs::create_dir_all(root.path().join(".agent-runtime/artifacts")).unwrap();
            symlink(
                outside.path(),
                root.path()
                    .join(".agent-runtime/artifacts/canonical-evidence"),
            )
            .unwrap();
        }
        assert!(publish(root.path(), &path, b"verified").is_err());
        assert_eq!(fs::read(&external).unwrap(), b"outside");
        assert_eq!(
            fs::metadata(&external).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
    }
}

#[cfg(unix)]
#[test]
fn hardlinks_and_nonprivate_existing_files_are_rejected() {
    use std::os::unix::fs::PermissionsExt;
    for hardlink in [false, true] {
        let root = TestWorkspace::new("evidence-hardlink");
        let path = target(root.path());
        publish(root.path(), &path, b"verified").unwrap();
        if hardlink {
            fs::hard_link(&path, root.path().join("link")).unwrap();
        } else {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert_eq!(
            publish(root.path(), &path, b"verified").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}

#[test]
fn concurrent_identical_publications_remain_verified_without_temporary_residue() {
    let root = TestWorkspace::new("evidence-concurrent");
    for round in 0..24 {
        let path = target(root.path()).with_file_name(format!("{round}.json"));
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let (path, barrier) = (&path, &barrier);
                let workspace = root.path();
                scope.spawn(move || {
                    barrier.wait();
                    publish(workspace, path, b"verified").unwrap();
                });
            }
        });
        verify(root.path(), &path, b"verified").unwrap();
        assert_eq!(
            fs::read_dir(path.parent().unwrap()).unwrap().count(),
            round + 1
        );
    }
}

#[test]
fn conflicting_concurrent_publications_reject_the_loser_without_overwriting() {
    let root = TestWorkspace::new("evidence-conflict");
    let path = target(root.path());
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8u8)
            .map(|value| {
                let (path, barrier) = (&path, &barrier);
                let workspace = root.path();
                scope.spawn(move || {
                    barrier.wait();
                    match publish(workspace, path, &[value]) {
                        Ok(()) => Some(value),
                        Err(error) => {
                            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                            None
                        }
                    }
                })
            })
            .collect();
        let winners: Vec<_> = workers
            .into_iter()
            .filter_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(winners.len(), 1);
        verify(root.path(), &path, &winners).unwrap();
        assert_eq!(fs::read(&path).unwrap(), winners);
    });
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn invalid_parent_is_explicit_and_never_publishes_a_reference() {
    let root = TestWorkspace::new("evidence-invalid-parent");
    fs::write(root.path().join(".agent-runtime"), b"not a directory").unwrap();
    assert!(publish(root.path(), &target(root.path()), b"verified").is_err());
}
