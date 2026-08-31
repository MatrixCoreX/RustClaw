use super::*;

fn claim_request(event_id: &str, payload: &[u8]) -> ChannelEventClaimRequest {
    claim_request_for(ChannelKind::Feishu, "app-id", event_id, payload)
}

fn claim_request_for(
    channel: ChannelKind,
    account_id: &str,
    event_id: &str,
    payload: &[u8],
) -> ChannelEventClaimRequest {
    let mut request = ChannelEventClaimRequest::new(channel, account_id, event_id, payload);
    request.provider_nonce = Some(format!("nonce-{event_id}"));
    request
}

#[test]
fn claim_is_atomic_and_reacquires_only_after_expiry() {
    let mut db = Connection::open_in_memory().expect("db");
    ensure_channel_event_admission_schema(&db).expect("schema");
    let request = claim_request("event-1", b"body");
    let first = claim_channel_event_in_db(&mut db, &request, 100).expect("first claim");
    let lease = match first {
        ClaimChannelEventOutcome::Acquired {
            lease_token,
            lease_expires_at_ts,
        } => {
            assert_eq!(lease_expires_at_ts, 400);
            lease_token
        }
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(
        claim_channel_event_in_db(&mut db, &request, 101).expect("duplicate"),
        ClaimChannelEventOutcome::InProgress {
            lease_expires_at_ts: 400
        }
    );
    let reacquired = claim_channel_event_in_db(&mut db, &request, 401).expect("reacquire");
    match reacquired {
        ClaimChannelEventOutcome::Acquired { lease_token, .. } => assert_ne!(lease_token, lease),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn completed_event_and_retryable_release_have_stable_results() {
    let mut db = Connection::open_in_memory().expect("db");
    ensure_channel_event_admission_schema(&db).expect("schema");
    let request = claim_request("event-2", b"body");
    let lease = match claim_channel_event_in_db(&mut db, &request, 100).expect("claim") {
        ClaimChannelEventOutcome::Acquired { lease_token, .. } => lease_token,
        other => panic!("unexpected {other:?}"),
    };
    let finish = ChannelEventFinishRequest {
        schema_version: claw_core::channel_event_admission::CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
        channel: request.channel,
        account_id: request.account_id.clone(),
        provider_event_id: request.provider_event_id.clone(),
        payload_sha256: request.payload_sha256.clone(),
        lease_token: lease,
        outcome: ChannelEventFinishOutcome::Completed,
    };
    assert_eq!(
        finish_channel_event_in_db(&mut db, &finish, 101).expect("finish"),
        FinishChannelEventOutcome::Completed
    );
    assert_eq!(
        finish_channel_event_in_db(&mut db, &finish, 102).expect("repeat finish"),
        FinishChannelEventOutcome::AlreadyCompleted
    );
    assert_eq!(
        claim_channel_event_in_db(&mut db, &request, 103).expect("repeat claim"),
        ClaimChannelEventOutcome::Completed
    );
}

#[test]
fn retryable_failure_releases_the_event_for_a_new_attempt() {
    let mut db = Connection::open_in_memory().expect("db");
    ensure_channel_event_admission_schema(&db).expect("schema");
    let request = claim_request("event-retryable", b"body");
    let first_lease = match claim_channel_event_in_db(&mut db, &request, 100).expect("claim") {
        ClaimChannelEventOutcome::Acquired { lease_token, .. } => lease_token,
        other => panic!("unexpected {other:?}"),
    };
    let finish = ChannelEventFinishRequest {
        schema_version: claw_core::channel_event_admission::CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
        channel: request.channel,
        account_id: request.account_id.clone(),
        provider_event_id: request.provider_event_id.clone(),
        payload_sha256: request.payload_sha256.clone(),
        lease_token: first_lease.clone(),
        outcome: ChannelEventFinishOutcome::RetryableFailure,
    };
    assert_eq!(
        finish_channel_event_in_db(&mut db, &finish, 101).expect("release"),
        FinishChannelEventOutcome::Released
    );
    let second_lease = match claim_channel_event_in_db(&mut db, &request, 102).expect("reclaim") {
        ClaimChannelEventOutcome::Acquired { lease_token, .. } => lease_token,
        other => panic!("unexpected {other:?}"),
    };
    assert_ne!(second_lease, first_lease);
}

#[test]
fn digest_and_nonce_reuse_conflicts_fail_closed() {
    let mut db = Connection::open_in_memory().expect("db");
    ensure_channel_event_admission_schema(&db).expect("schema");
    let first = claim_request("event-3", b"first");
    claim_channel_event_in_db(&mut db, &first, 100).expect("first claim");

    let changed = claim_request("event-3", b"changed");
    assert!(matches!(
        claim_channel_event_in_db(&mut db, &changed, 101),
        Err(ChannelEventAdmissionError::PayloadConflict)
    ));

    let mut reused_nonce = claim_request("event-4", b"other");
    reused_nonce.provider_nonce = first.provider_nonce;
    assert!(matches!(
        claim_channel_event_in_db(&mut db, &reused_nonce, 101),
        Err(ChannelEventAdmissionError::NonceConflict)
    ));
}

#[test]
fn concurrent_claims_have_one_owner_and_completion_survives_reopen() {
    use std::sync::{Arc, Barrier};

    let path = std::env::temp_dir().join(format!(
        "channel-event-admission-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    {
        let db = Connection::open(&path).expect("create db");
        ensure_channel_event_admission_schema(&db).expect("schema");
    }
    let request = claim_request("event-concurrent", b"body");
    let barrier = Arc::new(Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let path = path.clone();
            let request = request.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = Connection::open(path).expect("open worker db");
                db.busy_timeout(std::time::Duration::from_secs(2))
                    .expect("busy timeout");
                barrier.wait();
                claim_channel_event_in_db(&mut db, &request, 1_000).expect("concurrent claim")
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().expect("claim thread"))
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ClaimChannelEventOutcome::Acquired { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ClaimChannelEventOutcome::InProgress { .. }))
            .count(),
        1
    );

    let lease_token = outcomes
        .iter()
        .find_map(|outcome| match outcome {
            ClaimChannelEventOutcome::Acquired { lease_token, .. } => Some(lease_token.clone()),
            _ => None,
        })
        .expect("lease token");
    {
        let mut db = Connection::open(&path).expect("open finish db");
        let finish = ChannelEventFinishRequest {
            schema_version:
                claw_core::channel_event_admission::CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
            channel: request.channel,
            account_id: request.account_id.clone(),
            provider_event_id: request.provider_event_id.clone(),
            payload_sha256: request.payload_sha256.clone(),
            lease_token,
            outcome: ChannelEventFinishOutcome::Completed,
        };
        assert_eq!(
            finish_channel_event_in_db(&mut db, &finish, 1_001).expect("finish"),
            FinishChannelEventOutcome::Completed
        );
    }
    {
        let mut reopened = Connection::open(&path).expect("reopen db");
        assert_eq!(
            claim_channel_event_in_db(&mut reopened, &request, 1_002).expect("claim after reopen"),
            ClaimChannelEventOutcome::Completed
        );
    }
    std::fs::remove_file(path).expect("remove db");
}

#[test]
fn completed_event_replay_after_reopen_cannot_repeat_channel_side_effects() {
    let path = std::env::temp_dir().join(format!(
        "channel-event-side-effects-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let requests = [
        (ChannelKind::Feishu, "feishu-app"),
        (ChannelKind::Lark, "lark-app"),
        (ChannelKind::Telegram, "telegram-bot"),
        (ChannelKind::Wechat, "wechat-bot"),
        (ChannelKind::Whatsapp, "whatsapp-phone"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (channel, account_id))| {
        claim_request_for(
            channel,
            account_id,
            &format!("event-side-effects-{index}"),
            format!("signed-provider-body-{index}").as_bytes(),
        )
    })
    .collect::<Vec<_>>();

    {
        let mut db = Connection::open(&path).expect("create db");
        ensure_channel_event_admission_schema(&db).expect("schema");
        db.execute_batch(
            "CREATE TABLE test_channel_side_effects (
                effect_kind TEXT NOT NULL,
                provider_event_id TEXT NOT NULL,
                PRIMARY KEY(effect_kind, provider_event_id)
            );",
        )
        .expect("side effect fixture schema");
        for request in &requests {
            let lease_token = match claim_channel_event_in_db(&mut db, request, 100).expect("claim")
            {
                ClaimChannelEventOutcome::Acquired { lease_token, .. } => lease_token,
                other => panic!("unexpected {other:?}"),
            };
            for effect_kind in ["media_artifact", "binding", "task", "terminal_delivery"] {
                db.execute(
                    "INSERT INTO test_channel_side_effects(effect_kind, provider_event_id)
                     VALUES (?1, ?2)",
                    rusqlite::params![effect_kind, request.provider_event_id],
                )
                .expect("record first-attempt side effect");
            }
            let finish = ChannelEventFinishRequest {
                schema_version:
                    claw_core::channel_event_admission::CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
                channel: request.channel,
                account_id: request.account_id.clone(),
                provider_event_id: request.provider_event_id.clone(),
                payload_sha256: request.payload_sha256.clone(),
                lease_token,
                outcome: ChannelEventFinishOutcome::Completed,
            };
            assert_eq!(
                finish_channel_event_in_db(&mut db, &finish, 101).expect("finish"),
                FinishChannelEventOutcome::Completed
            );
        }
    }

    {
        let mut reopened = Connection::open(&path).expect("reopen db");
        for request in &requests {
            assert_eq!(
                claim_channel_event_in_db(&mut reopened, request, 102).expect("replay claim"),
                ClaimChannelEventOutcome::Completed
            );
        }
        let side_effect_count: i64 = reopened
            .query_row(
                "SELECT COUNT(*) FROM test_channel_side_effects",
                [],
                |row| row.get(0),
            )
            .expect("side effect count");
        assert_eq!(side_effect_count, 20);
        for request in &requests {
            for effect_kind in ["media_artifact", "binding", "task", "terminal_delivery"] {
                let count: i64 = reopened
                    .query_row(
                        "SELECT COUNT(*) FROM test_channel_side_effects
                         WHERE effect_kind = ?1 AND provider_event_id = ?2",
                        rusqlite::params![effect_kind, request.provider_event_id],
                        |row| row.get(0),
                    )
                    .expect("effect count");
                assert_eq!(count, 1, "{effect_kind} {:?}", request.channel);
            }
        }
    }
    std::fs::remove_file(path).expect("remove db");
}
