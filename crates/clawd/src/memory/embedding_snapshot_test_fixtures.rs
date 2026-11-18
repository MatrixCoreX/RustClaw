use super::*;
use rusqlite::{params, Connection};

pub(super) fn snapshot(db: &Connection, owner: &str, profile: &str, generation: i64, state: &str) {
    db.execute(
        "INSERT INTO memory_vector_snapshots(snapshot_id,principal_id,profile_id,generation,
         row_count,source_digest,snapshot_checksum,state,created_at_ts,updated_at_ts)
         VALUES(?1,?2,?3,?4,1,'fixture-source','fixture-checksum',?5,1,1)",
        params![
            format!("snapshot-{}", uuid::Uuid::new_v4()),
            owner,
            profile,
            generation,
            state
        ],
    )
    .unwrap();
}

pub(super) fn terminal_job(
    db: &Connection,
    owner: &str,
    profile: &str,
    generation: i64,
    status: &str,
) {
    db.execute(
        "INSERT INTO memory_embedding_jobs(job_id,retrieval_id,principal_id,scope_kind,scope_ref,
         profile_id,profile_generation,request_item_id,projection_version,projection_digest,
         consent_policy_digest,status,attempt,not_before_ts,created_at_ts,updated_at_ts)
         VALUES(?1,-1,?2,'principal',?2,?3,?4,'fixture-item','fixture-projection','fixture-digest',
         'fixture-consent',?5,1,1,1,1)",
        params![
            format!("terminal-job-{}", uuid::Uuid::new_v4()),
            owner,
            profile,
            generation,
            status
        ],
    )
    .unwrap();
}

pub(super) fn state(db: &Connection, owner: &str, profile: &str, generation: i64) -> String {
    db.query_row(
        "SELECT state FROM memory_vector_snapshots WHERE principal_id=?1 AND profile_id=?2 AND generation=?3",
        params![owner,profile,generation], |row|row.get(0),
    ).unwrap()
}

pub(super) async fn unknown_response() -> (Fixture, Credential, LoopbackEmbedding) {
    let mut server = LoopbackEmbedding::start().await;
    let (fixture, credential) = remote_fixture(&server.endpoint).await;
    fixture.state().core.db.get().unwrap().execute_batch(
        "CREATE TRIGGER snapshot_receive_fault BEFORE UPDATE OF checkpoint_json ON memory_embedding_jobs
         WHEN json_extract(NEW.checkpoint_json,'$.phase')='response_received'
         BEGIN SELECT RAISE(ABORT,'snapshot_response_persistence_failed'); END;",
    ).unwrap();
    let state = fixture.state().clone();
    let worker =
        tokio::spawn(
            async move { runtime::run_one_embedding_batch(&state, "snapshot-worker").await },
        );
    server.wait_for_request().await;
    server.release();
    assert!(worker.await.unwrap().is_err());
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    {
        let db = fixture.state().core.db.get().unwrap();
        assert_eq!(
            scalar(
                &db,
                "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status='running'
            AND json_extract(checkpoint_json,'$.phase')='dispatch_started'"
            ),
            1
        );
        db.execute_batch("DROP TRIGGER snapshot_receive_fault")
            .unwrap();
        assert_eq!(
            db.execute(
                "UPDATE memory_embedding_jobs SET lease_expires_at_ts=?1 WHERE status='running'",
                [crate::now_ts_u64() as i64 - 1]
            )
            .unwrap(),
            1
        );
        pressure(&db, "explicit_write_blocked");
    }
    (fixture, credential, server)
}
