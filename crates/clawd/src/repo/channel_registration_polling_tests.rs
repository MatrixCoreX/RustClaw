use super::*;

fn polling_state(db: &Connection, id: i64) -> (Option<i64>, String, String) {
    db.query_row(
        "SELECT install_poll_interval_seconds,updated_at,status FROM pending_channel_bind_sessions WHERE id=?1",
        [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap()
}

#[test]
fn polling_backoff_uses_the_last_committed_interval_after_reopen() {
    let mut fixture = Fixture::new();
    let expected = fixture.prepared("feishu");
    assert!(expected.install_poll_interval_seconds.is_none());
    assert_eq!(
        slow_registration_polling(&mut fixture.db, &expected)
            .unwrap()
            .install_poll_interval_seconds,
        Some(10)
    );
    fixture.close_file();
    let mut reopened = Fixture::open(&fixture.path);
    let next = slow_registration_polling(&mut reopened, &expected).unwrap();
    assert_eq!(next.install_poll_interval_seconds, Some(15));
    assert_eq!(next.expires_at, expected.expires_at);
    assert_eq!(next.install_device_code, expected.install_device_code);
    assert_eq!(next.status, "pending");
    assert!(matches!(
        registration_gate(&reopened, &expected).unwrap(),
        RegistrationGate::Ready { .. }
    ));
}

#[test]
fn polling_backoff_invalid_or_overflowing_state_never_becomes_a_short_interval() {
    for interval in [
        0,
        -1,
        MAX_REGISTRATION_POLL_INTERVAL_SECONDS - 4,
        MAX_REGISTRATION_POLL_INTERVAL_SECONDS,
        i64::MAX,
    ] {
        let mut fixture = Fixture::new();
        let expected = fixture.prepared("lark");
        fixture.db.execute(
            "UPDATE pending_channel_bind_sessions SET install_poll_interval_seconds=?1 WHERE id=?2",
            params![interval, expected.id],
        ).unwrap();
        let before = polling_state(&fixture.db, expected.id);
        let error = slow_registration_polling(&mut fixture.db, &expected).unwrap_err();
        assert_eq!(
            error.downcast_ref::<RegistrationJournalError>(),
            Some(&RegistrationJournalError::InvalidPollInterval)
        );
        assert_eq!(polling_state(&fixture.db, expected.id), before);
        assert!(matches!(
            registration_gate(&fixture.db, &expected).unwrap(),
            RegistrationGate::Ready { .. }
        ));
    }
    let mut fixture = Fixture::new();
    let expected = fixture.prepared("lark");
    fixture
        .db
        .execute(
            "UPDATE pending_channel_bind_sessions SET install_poll_interval_seconds=?1 WHERE id=?2",
            params![MAX_REGISTRATION_POLL_INTERVAL_SECONDS - 5, expected.id],
        )
        .unwrap();
    let last = slow_registration_polling(&mut fixture.db, &expected).unwrap();
    assert_eq!(
        last.install_poll_interval_seconds,
        Some(MAX_REGISTRATION_POLL_INTERVAL_SECONDS)
    );
}

#[test]
fn polling_backoff_fences_stale_rows_and_non_ready_journals() {
    for mutation in [
        "owner",
        "token",
        "device",
        "bound",
        "revision",
        "claimed",
        "unprepared",
    ] {
        let mut fixture = Fixture::new();
        let expected = if mutation == "unprepared" {
            fixture.session("feishu")
        } else {
            fixture.prepared("feishu")
        };
        let sql = match mutation {
            "owner" => Some("UPDATE pending_channel_bind_sessions SET user_key='fixture-other' WHERE id=?1"),
            "token" => Some("UPDATE pending_channel_bind_sessions SET bind_token='fixture-other-token' WHERE id=?1"),
            "device" => Some("UPDATE pending_channel_bind_sessions SET install_device_code='fixture-other-device' WHERE id=?1"),
            "bound" => Some("UPDATE pending_channel_bind_sessions SET status='bound' WHERE id=?1"),
            _ => None,
        };
        if let Some(sql) = sql {
            fixture.db.execute(sql, [expected.id]).unwrap();
        } else if mutation == "revision" {
            advance_configuration_revision(&mut fixture.db, "feishu").unwrap();
        } else if mutation == "claimed" {
            let _claim = acquire(&mut fixture.db, &expected);
        }
        let before = polling_state(&fixture.db, expected.id);
        let error = slow_registration_polling(&mut fixture.db, &expected).unwrap_err();
        assert_eq!(
            error.downcast_ref::<RegistrationJournalError>(),
            Some(&RegistrationJournalError::Superseded),
            "mutation={mutation}"
        );
        assert_eq!(polling_state(&fixture.db, expected.id), before);
    }
}

#[test]
fn polling_backoff_serializes_concurrent_file_connections_without_lost_increments() {
    let mut fixture = Fixture::new();
    let expected = fixture.prepared("feishu");
    fixture
        .db
        .execute(
            "UPDATE pending_channel_bind_sessions SET install_poll_interval_seconds=1 WHERE id=?1",
            [expected.id],
        )
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = fixture.path.clone();
            let expected = expected.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = Fixture::open(&path);
                barrier.wait();
                slow_registration_polling(&mut db, &expected)
                    .unwrap()
                    .install_poll_interval_seconds
                    .unwrap()
            })
        })
        .collect();
    let mut intervals: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    intervals.sort();
    assert_eq!(intervals, [6, 11]);
    fixture.close_file();
    let reopened = Fixture::open(&fixture.path);
    assert_eq!(polling_state(&reopened, expected.id).0, Some(11));
}

#[test]
fn polling_backoff_write_failure_rolls_back_without_consuming_authority() {
    let mut fixture = Fixture::new();
    let expected = fixture.prepared("lark");
    fixture.db.execute_batch(
        "CREATE TRIGGER fixture_polling_write_failure BEFORE UPDATE OF install_poll_interval_seconds
         ON pending_channel_bind_sessions BEGIN SELECT RAISE(ABORT,'fixture_write_failure'); END;",
    ).unwrap();
    let before = polling_state(&fixture.db, expected.id);
    assert!(slow_registration_polling(&mut fixture.db, &expected).is_err());
    assert_eq!(polling_state(&fixture.db, expected.id), before);
    assert!(matches!(
        registration_gate(&fixture.db, &expected).unwrap(),
        RegistrationGate::Ready { .. }
    ));
    fixture
        .db
        .execute_batch("DROP TRIGGER fixture_polling_write_failure")
        .unwrap();
    assert_eq!(
        slow_registration_polling(&mut fixture.db, &expected)
            .unwrap()
            .install_poll_interval_seconds,
        Some(10)
    );
}
