use super::*;
use crate::delivery_service::persistence;

async fn claimed_fixture() -> (
    AppState,
    ChannelDeliveryEnvelope,
    String,
    ChannelDeliveryReceipt,
) {
    let (state, _, _, envelope) = single_connection_fixture();
    let ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token } =
        persistence::claim(&state, &envelope).await.unwrap()
    else {
        panic!("acquired")
    };
    let receipt = accepted_delivery_receipt(&envelope, Default::default(), crate::now_ts_u64());
    (state, envelope, lease_token, receipt)
}

#[tokio::test(flavor = "current_thread")]
async fn receipt_read_record_complete_and_renew_pool_waits_are_isolated() {
    for stage in [
        "receipt_read",
        "receipt_record",
        "dispatch_complete",
        "dispatch_renew",
    ] {
        let (state, envelope, token, receipt) = claimed_fixture().await;
        if stage != "receipt_record" {
            persistence::record_receipt(&state, &receipt).await.unwrap();
        }
        let held = state.core.db.get().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            drop(held);
        });
        let started = Instant::now();
        let (_, delay) = tokio::join!(
            async {
                match stage {
                    "receipt_read" => assert_eq!(
                        persistence::load_receipt(&state, &envelope.idempotency_key)
                            .await
                            .unwrap(),
                        Some(receipt)
                    ),
                    "receipt_record" => {
                        persistence::record_receipt(&state, &receipt).await.unwrap();
                    }
                    "dispatch_complete" => {
                        persistence::complete(&state, &envelope.idempotency_key, &token)
                            .await
                            .unwrap()
                    }
                    "dispatch_renew" => {
                        assert!(
                            persistence::renew(&state, &envelope.idempotency_key, &token)
                                .await
                                .unwrap()
                        )
                    }
                    _ => unreachable!(),
                }
            },
            async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                started.elapsed()
            }
        );
        release.join().unwrap();
        eprintln!(
            "stage={stage} pool_hold_ms=500 timer_delay_ms={}",
            delay.as_millis()
        );
        assert!(delay < Duration::from_millis(400), "{stage}: {delay:?}");
    }
}

#[tokio::test]
async fn lease_completion_and_renewal_keep_repository_fences() {
    let (state, envelope, token, receipt) = claimed_fixture().await;
    assert_eq!(
        persistence::complete(&state, &envelope.idempotency_key, &token)
            .await
            .unwrap_err()
            .to_string(),
        "channel_delivery_dispatch_receipt_required"
    );
    persistence::record_receipt(&state, &receipt).await.unwrap();
    state.core.db.get().unwrap().execute(
        "UPDATE channel_delivery_dispatch_claims SET lease_token='replacement-owner', lease_expires_at_ts=123 WHERE idempotency_key=?1",
        [&envelope.idempotency_key],
    ).unwrap();
    assert!(
        !persistence::renew(&state, &envelope.idempotency_key, &token)
            .await
            .unwrap()
    );
    assert_eq!(
        persistence::complete(&state, &envelope.idempotency_key, &token)
            .await
            .unwrap_err()
            .to_string(),
        "channel_delivery_dispatch_lease_mismatch"
    );
    let expires: i64 = state.core.db.get().unwrap().query_row(
        "SELECT lease_expires_at_ts FROM channel_delivery_dispatch_claims WHERE idempotency_key=?1", [&envelope.idempotency_key], |row| row.get(0)
    ).unwrap();
    assert_eq!(expires, 123);
    persistence::complete(&state, &envelope.idempotency_key, "replacement-owner")
        .await
        .unwrap();
    assert!(
        !persistence::renew(&state, &envelope.idempotency_key, "replacement-owner")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn persistence_preserves_results_errors_panics_and_receipt_integrity() {
    assert_eq!(
        persistence::run_blocking("fixture", || Ok(7))
            .await
            .unwrap(),
        7
    );
    assert_eq!(
        persistence::run_blocking::<()>("fixture", || Err(anyhow!("fixture_error")))
            .await
            .unwrap_err()
            .to_string(),
        "fixture_error"
    );
    let panic = persistence::run_blocking::<()>("fixture_panic", || panic!("fixture_panic"))
        .await
        .unwrap_err();
    assert!(panic
        .to_string()
        .contains("channel_delivery_persistence_join_failed stage=fixture_panic"));
    let (state, envelope, _, receipt) = claimed_fixture().await;
    persistence::record_receipt(&state, &receipt).await.unwrap();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE channel_delivery_receipts SET receipt_json='{}' WHERE idempotency_key=?1",
            [&envelope.idempotency_key],
        )
        .unwrap();
    assert_eq!(
        persistence::load_receipt(&state, &envelope.idempotency_key)
            .await
            .unwrap_err()
            .to_string(),
        "channel_delivery_receipt_load_failed"
    );
    assert!(persistence::claim(&state, &envelope).await.is_err());
}

#[tokio::test]
async fn cancelled_waiter_cannot_replay_a_started_receipt_write() {
    let (state, envelope, _, receipt) = claimed_fixture().await;
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let (done, finished) = tokio::sync::oneshot::channel();
    let count = Arc::new(AtomicUsize::new(0));
    let executions = count.clone();
    let pool = state.core.db.clone();
    let writer = tokio::spawn(persistence::run_blocking(
        "fixture_receipt_write",
        move || {
            executions.fetch_add(1, Ordering::SeqCst);
            let _ = started.send(());
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            let result = crate::repo::record_channel_delivery_receipt(&pool, &receipt);
            let _ = done.send(());
            result
        },
    ));
    ready.await.unwrap();
    writer.abort();
    assert!(writer.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), finished)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(persistence::load_receipt(&state, &envelope.idempotency_key)
        .await
        .unwrap()
        .is_some());
    assert!(matches!(
        persistence::claim(&state, &envelope).await.unwrap(),
        ClaimChannelDeliveryDispatchOutcome::ExistingReceipt(_)
    ));
    let events: i64 = state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM channel_delivery_receipt_events WHERE idempotency_key=?1",
            [&envelope.idempotency_key],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn file_backed_contention_keeps_runtime_responsive_and_errors_visible() {
    for (mode, lock) in [("DELETE", "BEGIN EXCLUSIVE"), ("WAL", "BEGIN IMMEDIATE")] {
        let (mut state, _, _, envelope) = single_connection_fixture();
        let dir = std::env::temp_dir().join(format!("delivery-persistence-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        state.core.db = r2d2::Pool::builder()
            .max_size(2)
            .build(
                r2d2_sqlite::SqliteConnectionManager::file(dir.join("receipts.sqlite")).with_init(
                    move |db| {
                        db.busy_timeout(Duration::from_secs(3))?;
                        db.pragma_update(None, "journal_mode", mode)
                    },
                ),
            )
            .unwrap();
        let writer = state.core.db.get().unwrap();
        let actual_mode: String = writer
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(actual_mode.to_ascii_uppercase(), mode);
        crate::repo::channel_delivery_receipt::ensure_channel_delivery_receipt_schema(&writer)
            .unwrap();
        writer.execute_batch(lock).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            writer.execute_batch("COMMIT").unwrap();
        });
        let receipt = accepted_delivery_receipt(&envelope, Default::default(), crate::now_ts_u64());
        let start = Instant::now();
        let (result, delay) = tokio::join!(persistence::record_receipt(&state, &receipt), async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            start.elapsed()
        });
        release.join().unwrap();
        let busy = match result {
            Ok(_) => {
                assert!(persistence::load_receipt(&state, &envelope.idempotency_key)
                    .await
                    .unwrap()
                    .is_some());
                false
            }
            Err(error) => {
                assert_eq!(lock, "BEGIN IMMEDIATE");
                let error = error.downcast::<rusqlite::Error>().unwrap();
                assert!(matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ));
                assert!(persistence::load_receipt(&state, &envelope.idempotency_key)
                    .await
                    .unwrap()
                    .is_none());
                true
            }
        };
        eprintln!(
            "stage=file_writer_lock mode={mode} lock={lock} busy={busy} lock_hold_ms=500 timer_delay_ms={}",
            delay.as_millis()
        );
        assert!(delay < Duration::from_millis(400), "{delay:?}");
        drop(state);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
