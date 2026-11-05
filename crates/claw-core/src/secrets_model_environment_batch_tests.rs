use super::*;

fn values(root: &Path) -> BTreeMap<String, String> {
    read(&path(root)).unwrap()
}

#[test]
fn model_environment_batch_publishes_all_vendors_once_and_preserves_unrelated_keys() {
    let fixture = OwnedFixture(fixture());
    save(&fixture.0, "google", "fixture-unrelated").unwrap();
    let mut calls = 0;
    let result = with_saved_many(
        &fixture.0,
        &[("custom", "fixture-one"), ("mimo", "fixture-two")],
        || {
            calls += 1;
            let entries = values(&fixture.0);
            assert_eq!(entries["CUSTOM_API_KEY"], "fixture-one");
            assert_eq!(entries["MIMO_API_KEY"], "fixture-two");
            assert_eq!(entries["GOOGLE_API_KEY"], "fixture-unrelated");
            Ok::<_, ()>(42)
        },
    )
    .unwrap();
    assert_eq!((result, calls), (42, 1));
}

#[test]
fn model_environment_batch_failed_publication_restores_exact_original_bytes() {
    let fixture = OwnedFixture(fixture());
    save(&fixture.0, "custom", "fixture-original").unwrap();
    let before = fs::read(path(&fixture.0)).unwrap();
    let result = with_saved_many(
        &fixture.0,
        &[("custom", "fixture-one"), ("mimo", "fixture-two")],
        || Err::<(), _>("fixture-operation"),
    );
    assert!(matches!(
        result,
        Err(PublicationError::Operation("fixture-operation"))
    ));
    assert_eq!(fs::read(path(&fixture.0)).unwrap(), before);
}

#[test]
fn model_environment_batch_failed_first_publication_removes_only_its_owned_file() {
    let fixture = OwnedFixture(fixture());
    let result = with_saved_many(
        &fixture.0,
        &[("custom", "fixture-one"), ("mimo", "fixture-two")],
        || Err::<(), _>(()),
    );
    assert!(matches!(result, Err(PublicationError::Operation(()))));
    assert!(!path(&fixture.0).exists());
}

#[test]
fn model_environment_batch_invalid_late_value_never_publishes_any_vendor() {
    let fixture = OwnedFixture(fixture());
    save(&fixture.0, "custom", "fixture-original").unwrap();
    let before = fs::read(path(&fixture.0)).unwrap();
    for updates in [
        vec![("custom", "fixture-one"), ("mimo", "invalid\nvalue")],
        vec![("custom", "fixture-one"), ("unknown", "fixture-two")],
    ] {
        let mut called = false;
        let result = with_saved_many(&fixture.0, &updates, || {
            called = true;
            Ok::<_, ()>(())
        });
        assert!(matches!(result, Err(PublicationError::Environment(_))));
        assert!(!called);
        assert_eq!(fs::read(path(&fixture.0)).unwrap(), before);
    }
}

#[test]
fn model_environment_batch_recovery_preserves_an_independent_commit() {
    let fixture = OwnedFixture(fixture());
    let independent = b"CUSTOM_API_KEY=fixture-independent\n";
    let result = with_saved_many(
        &fixture.0,
        &[("custom", "fixture-one"), ("mimo", "fixture-two")],
        || {
            fs::write(path(&fixture.0), independent).unwrap();
            Err::<(), _>(())
        },
    );
    assert!(matches!(result, Err(PublicationError::Recovery(_))));
    assert_eq!(fs::read(path(&fixture.0)).unwrap(), independent);
}

#[test]
fn model_environment_batch_empty_or_unchanged_values_do_not_rewrite_bytes_or_file_identity() {
    let fixture = OwnedFixture(fixture());
    with_saved_many(&fixture.0, &[], || Ok::<_, ()>(())).unwrap();
    assert!(!path(&fixture.0).exists());
    save(&fixture.0, "custom", "fixture-original").unwrap();
    fs::write(
        path(&fixture.0),
        b"# preserve custom comment\nCUSTOM_API_KEY=fixture-original\n",
    )
    .unwrap();
    let before = fs::read(path(&fixture.0)).unwrap();
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path(&fixture.0)).unwrap().ino()
    };
    with_saved_many(&fixture.0, &[("custom", "fixture-original")], || {
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(fs::read(path(&fixture.0)).unwrap(), before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(path(&fixture.0)).unwrap().ino(), inode);
    }
}

#[test]
fn model_environment_batch_duplicate_aliases_are_rejected_before_publication() {
    let fixture = OwnedFixture(fixture());
    let mut called = false;
    let result = with_saved_many(
        &fixture.0,
        &[("custom", "fixture-one"), ("CUSTOM", "fixture-two")],
        || {
            called = true;
            Ok::<_, ()>(())
        },
    );
    assert!(matches!(result, Err(PublicationError::Environment(_))));
    assert!(!called);
    assert!(!path(&fixture.0).exists());
}

#[test]
fn model_environment_batch_panicked_publication_restores_all_values_and_releases_lock() {
    let fixture = OwnedFixture(fixture());
    save(&fixture.0, "custom", "fixture-original").unwrap();
    let before = fs::read(path(&fixture.0)).unwrap();
    let result = std::panic::catch_unwind(|| {
        with_saved_many(
            &fixture.0,
            &[("custom", "fixture-one"), ("mimo", "fixture-two")],
            || -> Result<(), ()> { panic!("fixture-only batch publication panic") },
        )
    });
    assert!(result.is_err());
    assert_eq!(fs::read(path(&fixture.0)).unwrap(), before);
    save(&fixture.0, "custom", "fixture-after-panic").unwrap();
    assert_eq!(values(&fixture.0)["CUSTOM_API_KEY"], "fixture-after-panic");
}
