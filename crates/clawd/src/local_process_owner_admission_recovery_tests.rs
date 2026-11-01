use super::*;

#[test]
fn owner_admission_ready_write_failure_retains_incomplete_state_without_a_store() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    journal.db.execute_batch(journal::INIT_SQL).unwrap();
    journal
        .db
        .execute_batch(
            "CREATE TRIGGER refuse_ready BEFORE UPDATE ON local_process_authority_bootstrap
        WHEN NEW.phase = 'ready' BEGIN SELECT RAISE(ABORT, 'fixture_abort'); END;",
        )
        .unwrap();
    assert!(matches!(
        journal.initialize(&fixture.broker()),
        Err(AdmissionFailure::Sqlite {
            stage: AdmissionStage::CommitReady,
            sqlite_code: Some(ErrorCode::ConstraintViolation),
            ..
        })
    ));
    assert!(fixture.key_file().exists());
    assert_eq!(fixture.sequence(), 0);
    assert_eq!(
        journal
            .resume(&fixture.broker(), &absent_pin())
            .err()
            .unwrap(),
        AdmissionFailure::IncompleteInitialization
    );
    assert_eq!(
        journal.initialize(&fixture.broker()).err().unwrap(),
        AdmissionFailure::AlreadyInitialized
    );
}

#[test]
fn owner_admission_busy_reservation_has_no_key_creation_effect() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    journal.db.busy_timeout(Duration::ZERO).unwrap();
    let writer = Fixture::open(&fixture.database);
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(matches!(
        journal.initialize(&fixture.broker()),
        Err(AdmissionFailure::Sqlite {
            stage: AdmissionStage::Reserve,
            sqlite_code: Some(ErrorCode::DatabaseBusy),
            ..
        })
    ));
    assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
    writer.execute_batch("ROLLBACK").unwrap();
    journal.initialize(&fixture.broker()).unwrap();
}

#[test]
fn owner_admission_readonly_reservation_has_no_key_creation_effect() {
    let fixture = Fixture::new();
    let db = Connection::open_with_flags(
        &fixture.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut journal = OwnerAuthorityJournal::from_connection(db).unwrap();
    assert!(matches!(
        journal.initialize(&fixture.broker()),
        Err(AdmissionFailure::Sqlite { .. })
    ));
    assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
}

#[test]
fn owner_admission_ready_barrier_failure_returns_no_filesystem_admission() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    let fingerprint = journal
        .initialize(&fixture.broker())
        .unwrap()
        .authority_fingerprint()
        .unwrap();
    journal
        .db
        .execute_batch(
            "CREATE TRIGGER refuse_barrier BEFORE UPDATE ON local_process_authority_bootstrap
        BEGIN SELECT RAISE(ABORT, 'fixture_abort'); END;",
        )
        .unwrap();
    assert!(matches!(
        journal.resume(&fixture.broker(), &fingerprint),
        Err(AdmissionFailure::Sqlite {
            stage: AdmissionStage::AdmitExisting,
            sqlite_code: Some(ErrorCode::ConstraintViolation),
            ..
        })
    ));
    assert_eq!(fixture.sequence(), 1);
}

#[test]
fn owner_admission_concurrent_initializers_have_one_journal_and_key_winner() {
    use std::sync::Barrier;

    let fixture = Fixture::new();
    let barrier = Barrier::new(8);
    let winners = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            let (fixture, barrier) = (&fixture, &barrier);
            handles.push(scope.spawn(move || {
                let mut journal = fixture.journal();
                let broker = fixture.broker();
                barrier.wait();
                match journal.initialize(&broker) {
                    Ok(store) => Some(store.authority_fingerprint().unwrap()),
                    Err(error) => {
                        assert_eq!(error, AdmissionFailure::AlreadyInitialized);
                        None
                    }
                }
            }));
        }
        handles
            .into_iter()
            .filter_map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(winners.len(), 1);
    assert_eq!(
        fixture
            .journal()
            .resume(&fixture.broker(), &winners[0])
            .unwrap()
            .authority_fingerprint()
            .unwrap(),
        winners[0]
    );
}

#[test]
fn owner_admission_concurrent_existing_admissions_preserve_one_immutable_identity() {
    use std::sync::Barrier;

    let fixture = Fixture::new();
    let expected = fixture
        .journal()
        .initialize(&fixture.broker())
        .unwrap()
        .authority_fingerprint()
        .unwrap();
    let barrier = Barrier::new(8);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            let (fixture, barrier) = (&fixture, &barrier);
            handles.push(scope.spawn(move || {
                let mut journal = fixture.journal();
                let broker = fixture.broker();
                barrier.wait();
                assert_eq!(
                    journal
                        .resume(&broker, &expected)
                        .unwrap()
                        .authority_fingerprint()
                        .unwrap(),
                    expected
                );
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
    });
    assert_eq!(fixture.sequence(), 9);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_admission_real_key_sync_failure_cannot_be_promoted_to_ready() {
    use std::os::unix::fs::OpenOptionsExt;

    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    let parent = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
        .open(&fixture.keys)
        .unwrap();
    let broker = OwnerAuthorityBroker::from_admitted_directory(parent).unwrap();
    assert!(matches!(
        journal.initialize(&broker),
        Err(AdmissionFailure::Authority(AuthorityFailure {
            os_error: Some(libc::EBADF),
            ..
        }))
    ));
    assert!(fixture.key_file().exists());
    assert_eq!(fixture.sequence(), 0);
    assert_eq!(
        journal
            .resume(&fixture.broker(), &absent_pin())
            .err()
            .unwrap(),
        AdmissionFailure::IncompleteInitialization
    );
}

#[cfg(target_os = "linux")]
#[test]
fn owner_admission_actual_process_exit_preserves_pending_or_ready_journal_state() {
    for case in ["admission_reserve_exit", "admission_ready_exit"] {
        let fixture = Fixture::new();
        let output = super::super::super::owner_scope::tests::fixture_command(case, None)
            .env("APP_OWNER_REFERENCE_TEST_ROOT", &fixture.root)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
        if case == "admission_reserve_exit" {
            assert_eq!(
                fixture
                    .journal()
                    .resume(&fixture.broker(), &absent_pin())
                    .err()
                    .unwrap(),
                AdmissionFailure::IncompleteInitialization
            );
            assert_eq!(fixture.sequence(), 0);
            assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
        } else {
            // The owned child has completed initialization. This fixture reads
            // public metadata only; real host admission must retain its own pin.
            let bytes: Vec<u8> = Fixture::open(&fixture.database)
                .query_row(
                    "SELECT fingerprint FROM local_process_authority_bootstrap",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let expected = AuthorityFingerprint::from_slice(&bytes).unwrap();
            fixture
                .journal()
                .resume(&fixture.broker(), &expected)
                .unwrap();
            assert_eq!(fixture.sequence(), 2);
        }
    }
}
