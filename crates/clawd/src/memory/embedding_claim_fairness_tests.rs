use rusqlite::params;

use crate::memory::capacity_test_fixtures::{persist_preference_sources, Fixture};
use crate::memory::vector_store::LOCAL_PROFILE_ID;

fn seed_owner(fixture: &Fixture, key: &str, count: usize, queued_at: i64) -> String {
    let db = fixture.state().core.db.get().unwrap();
    let principal = crate::repo::auth::principal_id_for_user_key(&db, key)
        .unwrap()
        .unwrap();
    crate::memory::retention::ensure_principal_quota(
        &db,
        &fixture.state().policy.memory,
        &principal,
        crate::now_ts_u64() as i64,
    )
    .unwrap();
    let entries = (0..count)
        .map(|n| {
            (
                format!("queue-{n}"),
                format!("owned source {n}"),
                1.0,
                "user".to_string(),
            )
        })
        .collect::<Vec<_>>();
    persist_preference_sources(&db, 1, 0, key, &entries, 1);
    crate::memory::indexing::index_preference_entries(&db, 1, 0, key, &entries, 1).unwrap();
    assert_eq!(
        db.execute(
            "UPDATE memory_embedding_jobs SET not_before_ts=?1, created_at_ts=?1
             WHERE principal_id=?2 AND status='queued'",
            params![queued_at, principal],
        )
        .unwrap(),
        count
    );
    principal
}

fn two_owners() -> (Fixture, String, String) {
    let mut fixture = Fixture::memory();
    fixture.state_mut().policy.memory.embedding_batch_size = 2;
    let first = seed_owner(&fixture, &fixture.key, 4, 1);
    let key = crate::repo::auth::create_auth_key(fixture.state(), "user").unwrap();
    let second = seed_owner(&fixture, &key, 2, 2);
    (fixture, first, second)
}

fn rows(
    db: &rusqlite::Connection,
    owner: &str,
) -> Vec<(String, String, i64, Option<String>, Option<i64>)> {
    db.prepare(
        "SELECT job_id,status,attempt,lease_owner,lease_expires_at_ts
         FROM memory_embedding_jobs WHERE principal_id=?1 ORDER BY job_id",
    )
    .unwrap()
    .query_map([owner], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .unwrap()
    .collect::<rusqlite::Result<Vec<_>>>()
    .unwrap()
}

#[test]
fn queued_unserved_owner_is_not_hidden_by_a_busy_older_owner() {
    let (fixture, first, second) = two_owners();
    let initial = super::claim_embedding_batch(fixture.state(), "first-worker").unwrap();
    assert_eq!(initial.len(), 2);
    assert!(initial.iter().all(|j| j.principal_id == first));
    let before = rows(&fixture.state().core.db.get().unwrap(), &first);

    let next = super::claim_embedding_batch(fixture.state(), "second-worker").unwrap();
    assert_eq!(next.len(), 2);
    assert!(
        next.iter().all(|j| j.principal_id == second),
        "an unserved eligible owner must get a batch before the busy older owner gets another"
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        rows(&db, &first),
        before,
        "claiming a peer cannot rewrite existing leases or queued rows"
    );
    assert!(next
        .iter()
        .all(|j| j.profile_id == LOCAL_PROFILE_ID && j.attempt == 1));
    assert!(next
        .iter()
        .all(|j| j.profile_generation == next[0].profile_generation
            && j.consent_policy_digest == next[0].consent_policy_digest));
    eprintln!("EMBEDDING_CLAIM_FAIRNESS busy_owner_running=2 unserved_owner_claimed=2 existing_owner_rows_unchanged=true");
}

#[test]
fn paused_unserved_owner_is_not_claimed_for_fairness() {
    let (fixture, first, second) = two_owners();
    assert_eq!(
        super::claim_embedding_batch(fixture.state(), "first-worker")
            .unwrap()
            .len(),
        2
    );
    let before;
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute(
            "INSERT INTO memory_embedding_controls(principal_id,profile_id,state,updated_at_ts)
             VALUES (?1,?2,'paused',1)",
            params![second, LOCAL_PROFILE_ID],
        )
        .unwrap();
        before = rows(&db, &second);
    }
    let next = super::claim_embedding_batch(fixture.state(), "second-worker").unwrap();
    assert_eq!(next.len(), 2);
    assert!(next.iter().all(|j| j.principal_id == first));
    assert_eq!(
        rows(&fixture.state().core.db.get().unwrap(), &second),
        before
    );
}

#[test]
fn circuit_blocked_unserved_owner_is_not_claimed_for_fairness() {
    let (fixture, first, second) = two_owners();
    assert_eq!(
        super::claim_embedding_batch(fixture.state(), "first-worker")
            .unwrap()
            .len(),
        2
    );
    let before;
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute(
            "INSERT INTO memory_embedding_circuits(principal_id,profile_id,failure_count,
                open_until_ts,last_error_code,updated_at_ts)
             VALUES (?1,?2,4,?3,'fixture_rate_limit',1)",
            params![second, LOCAL_PROFILE_ID, crate::now_ts_u64() as i64 + 300],
        )
        .unwrap();
        before = rows(&db, &second);
    }
    let next = super::claim_embedding_batch(fixture.state(), "second-worker").unwrap();
    assert_eq!(next.len(), 2);
    assert!(next.iter().all(|j| j.principal_id == first));
    assert_eq!(
        rows(&fixture.state().core.db.get().unwrap(), &second),
        before
    );
}
