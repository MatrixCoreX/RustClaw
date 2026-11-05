use super::*;
use std::sync::{mpsc, Arc};
use std::time::Duration;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("configuration-files-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn target(&self, name: &str) -> Target {
        Target::workspace(&self.0, self.0.join(name))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
#[test]
fn configuration_files_missing_target_alias_resolves_symlink_before_parent_component() {
    let fixture = Fixture::new();
    let outside = Fixture::new();
    fs::create_dir(outside.0.join("child")).unwrap();
    std::os::unix::fs::symlink(outside.0.join("child"), fixture.0.join("alias")).unwrap();
    let alias = fixture.0.join("alias/../new.toml");
    let physical = outside.0.join("new.toml");
    assert_eq!(locks::key(&alias).unwrap(), locks::key(&physical).unwrap());
    // Only the explicit trusted configured-path constructor can resolve this
    // host choice; a request's workspace-relative path must remain rejected.
    assert!(Target::workspace(&fixture.0, alias.clone())
        .publish(b"rejected")
        .is_err());
    assert!(!physical.exists());
    Target::configured(&fixture.0, alias)
        .unwrap()
        .publish(b"configured")
        .unwrap();
    assert_eq!(fs::read(physical).unwrap(), b"configured");
    assert!(!fixture.0.join("new.toml").exists());
}

#[test]
fn configuration_files_parent_normalization_does_not_treat_a_file_as_a_directory() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("not-directory"), b"untouched").unwrap();
    let invalid = fixture.0.join("not-directory/../target");
    assert!(locks::key(&invalid).is_err());
    assert!(Target::configured(&fixture.0, invalid).is_err());
    assert_eq!(
        fs::read(fixture.0.join("not-directory")).unwrap(),
        b"untouched"
    );
    assert!(!fixture.0.join("target").exists());
}

#[test]
fn configuration_files_preflight_rejects_later_directory_without_publishing_first() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("active"), b"before").unwrap();
    fs::create_dir(fixture.0.join("mirror")).unwrap();
    assert!(publish_copies(
        vec![fixture.target("active"), fixture.target("mirror")],
        b"after"
    )
    .is_err());
    assert_eq!(fs::read(fixture.0.join("active")).unwrap(), b"before");
}

#[test]
fn configuration_files_failed_publication_restores_owned_files_but_preserves_independent_commit() {
    let fixture = Fixture::new();
    let edits = ["first", "second"].map(|name| Edit {
        target: fixture.target(name),
        original: None,
        updated: b"ours".to_vec(),
    });
    let mut calls = 0;
    let error = commit_with(&edits, |target, bytes| {
        calls += 1;
        target.publish(bytes)?;
        if calls == 2 {
            fs::write(&edits[0].target.path, b"independent")?;
            return Err(io::Error::other("fixture late failure"));
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.kind, FailureKind::Recovery);
    assert_eq!(error.index, 1);
    assert_eq!(fs::read(&edits[0].target.path).unwrap(), b"independent");
    assert!(!edits[1].target.path.exists());
}

#[test]
fn configuration_files_rechecks_unpublished_targets_after_each_publication() {
    let fixture = Fixture::new();
    let edits = ["first", "second"].map(|name| Edit {
        target: fixture.target(name),
        original: None,
        updated: b"ours".to_vec(),
    });
    let mut calls = 0;
    let result = commit_with(&edits, |target, bytes| {
        calls += 1;
        target.publish(bytes)?;
        if calls == 1 {
            fs::write(&edits[1].target.path, b"independent")?;
        }
        Ok(())
    });
    assert_eq!(result.unwrap_err().kind, FailureKind::Recovery);
    assert_eq!(calls, 1);
    assert!(!edits[0].target.path.exists());
    assert_eq!(fs::read(&edits[1].target.path).unwrap(), b"independent");
}

#[test]
fn configuration_files_equivalent_lock_paths_share_keys_even_before_creation() {
    let fixture = Fixture::new();
    let canonical = fixture.0.canonicalize().unwrap();
    assert_eq!(
        locks::key(&fixture.0.join("new/./config")).unwrap(),
        canonical.join("new/config")
    );
    #[cfg(unix)]
    {
        let other = Fixture::new();
        std::os::unix::fs::symlink(&fixture.0, other.0.join("alias")).unwrap();
        assert_eq!(
            locks::key(&other.0.join("alias/new/config")).unwrap(),
            locks::key(&fixture.0.join("new/config")).unwrap()
        );
    }
}

#[test]
fn configuration_files_unrelated_paths_progress_while_an_overlapping_path_waits() {
    let fixture = Fixture::new();
    let shared = Arc::new(fixture.0.join("shared"));
    let separate = fixture.0.join("separate");
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let owner_path = Arc::clone(&shared);
    let owner = std::thread::spawn(move || {
        with_locked_files(&[(*owner_path).clone()], || {
            held_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        })
        .unwrap()
    });
    held_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let (overlap_tx, overlap_rx) = mpsc::channel();
    let overlap = std::thread::spawn(move || {
        with_locked_files(&[(*shared).clone()], || overlap_tx.send(()).unwrap()).unwrap()
    });
    let (separate_tx, separate_rx) = mpsc::channel();
    let other = std::thread::spawn(move || {
        with_locked_files(&[separate], || separate_tx.send(()).unwrap()).unwrap()
    });
    let independent = separate_rx.recv_timeout(Duration::from_secs(2));
    let still_waiting = overlap_rx.try_recv().is_err();
    release_tx.send(()).unwrap();
    owner.join().unwrap();
    other.join().unwrap();
    overlap.join().unwrap();
    assert!(
        independent.is_ok(),
        "unrelated configuration must not wait on a global operation lock"
    );
    assert!(still_waiting);
    overlap_rx.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn configuration_files_panicked_request_does_not_poison_future_publication() {
    let fixture = Fixture::new();
    let paths = [fixture.0.join("config")];
    let panic =
        std::panic::catch_unwind(|| with_locked_files(&paths, || panic!("fixture-only panic")));
    assert!(panic.is_err());
    let mut completed = false;
    with_locked_files(&paths, || completed = true).unwrap();
    assert!(completed);
}

#[cfg(unix)]
#[test]
fn configuration_files_workspace_symlink_is_rejected_without_touching_other_targets() {
    let fixture = Fixture::new();
    let outside = Fixture::new();
    fs::write(outside.0.join("secret"), b"outside").unwrap();
    std::os::unix::fs::symlink(outside.0.join("secret"), fixture.0.join("link")).unwrap();
    assert!(publish_copies(vec![fixture.target("new"), fixture.target("link")], b"ours").is_err());
    assert!(!fixture.0.join("new").exists());
    assert_eq!(fs::read(outside.0.join("secret")).unwrap(), b"outside");
}
