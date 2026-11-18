use super::*;
use crate::memory::capacity_test_fixtures::Fixture;

fn seeded(kind: &str) -> Fixture {
    let fixture = setup::fixture();
    {
        let db = fixture.state().core.db.get().unwrap();
        seed(
            &db,
            &fixture.key,
            &fixture.principal,
            kind,
            "derived-witness",
        );
        assert!(!rows(&db, &fixture.principal).is_empty());
        assert!(count(&db, &fixture.principal, "memory_embedding_jobs", "1=1") > 0);
    }
    fixture
}

fn count(db: &Connection, owner: &str, table: &str, predicate: &str) -> i64 {
    db.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE principal_id=?1 AND ({predicate})"),
        [owner],
        |row| row.get(0),
    )
    .unwrap()
}

fn suppress(fixture: &Fixture) {
    let db = fixture.state().core.db.get().unwrap();
    record_feedback(
        &db,
        &fixture.principal,
        "derived-witness",
        &feedback(MemoryFeedbackKind::DoNotUse),
        200,
    )
    .unwrap();
    assert!(rows(&db, &fixture.principal).is_empty());
    assert_no_orphan_fts(&db);
}

fn queued_suppression(kind: &str) {
    let fixture = seeded(kind);
    suppress(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        count(
            &db,
            &fixture.principal,
            "memory_embedding_jobs",
            "status IN ('queued','retry_wait','running') AND cancel_requested=0"
        ),
        0,
        "explicit recall suppression must retire existing embedding jobs, not just delete text indexes"
    );
}

async fn vector_suppression(kind: &str) {
    let fixture = seeded(kind);
    assert!(crate::memory::embedding_jobs::run_one_embedding_batch(
        fixture.state(),
        "derived-vector-fixture"
    )
    .await
    .unwrap());
    {
        let db = fixture.state().core.db.get().unwrap();
        assert!(
            count(
                &db,
                &fixture.principal,
                "memory_vector_rows",
                "status='active'"
            ) > 0
        );
    }
    suppress(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        count(
            &db,
            &fixture.principal,
            "memory_vector_rows",
            "status='active'"
        ),
        0,
        "explicit recall suppression must make existing vectors inactive in the same mutation"
    );
}

async fn correction_retires_old_vectors(kind: &str) {
    let fixture = seeded(kind);
    assert!(crate::memory::embedding_jobs::run_one_embedding_batch(
        fixture.state(),
        "derived-correction-fixture"
    )
    .await
    .unwrap());
    let db = fixture.state().core.db.get().unwrap();
    assert!(
        count(
            &db,
            &fixture.principal,
            "memory_vector_rows",
            "status='active'"
        ) > 0
    );
    let corrected = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "derived-witness",
        &correction(),
        200,
    )
    .unwrap();
    assert_replacement(
        &db,
        &fixture.principal,
        kind,
        corrected.replacement_memory_id.as_deref().unwrap(),
    );
    assert_eq!(
        count(
            &db,
            &fixture.principal,
            "memory_vector_rows",
            "status='active'"
        ),
        0,
        "a confirmed correction must retire old vectors before its new embedding is generated"
    );
    assert!(
        count(
            &db,
            &fixture.principal,
            "memory_embedding_jobs",
            "status='queued' AND cancel_requested=0"
        ) > 0,
        "the corrected canonical projection still needs its independent new embedding"
    );
}

#[test]
fn disabled_fact_retires_queued_embeddings() {
    queued_suppression("fact");
}

#[test]
fn disabled_preference_retires_queued_embeddings() {
    queued_suppression("preference");
}

#[test]
fn disabled_transcript_retires_queued_embeddings() {
    queued_suppression("recent");
}

#[tokio::test]
async fn disabled_fact_deactivates_previously_completed_vectors() {
    vector_suppression("fact").await;
}

#[tokio::test]
async fn disabled_preference_deactivates_previously_completed_vectors() {
    vector_suppression("preference").await;
}

#[tokio::test]
async fn disabled_transcript_deactivates_previously_completed_vectors() {
    vector_suppression("recent").await;
}

#[tokio::test]
async fn corrected_fact_retires_its_old_completed_vectors() {
    correction_retires_old_vectors("fact").await;
}

#[tokio::test]
async fn corrected_preference_retires_its_old_completed_vectors() {
    correction_retires_old_vectors("preference").await;
}

#[tokio::test]
async fn suppression_keeps_another_owners_completed_vectors_and_pending_work() {
    let fixture = seeded("preference");
    assert!(crate::memory::embedding_jobs::run_one_embedding_batch(
        fixture.state(),
        "derived-isolation-fixture"
    )
    .await
    .unwrap());
    let (other_key, other_owner) = setup::another_owner(&fixture);
    {
        let db = fixture.state().core.db.get().unwrap();
        seed(
            &db,
            &other_key,
            &other_owner,
            "preference",
            "foreign-derived",
        );
    }
    assert!(crate::memory::embedding_jobs::run_one_embedding_batch(
        fixture.state(),
        "foreign-derived-fixture"
    )
    .await
    .unwrap());
    {
        let db = fixture.state().core.db.get().unwrap();
        seed(&db, &other_key, &other_owner, "fact", "foreign-pending");
    }
    let before = {
        let db = fixture.state().core.db.get().unwrap();
        let active = count(&db, &other_owner, "memory_vector_rows", "status='active'");
        let pending = count(
            &db,
            &other_owner,
            "memory_embedding_jobs",
            "status='queued' AND cancel_requested=0",
        );
        assert!(active > 0 && pending > 0);
        (active, pending, rows(&db, &other_owner))
    };
    suppress(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        (
            count(&db, &other_owner, "memory_vector_rows", "status='active'"),
            count(
                &db,
                &other_owner,
                "memory_embedding_jobs",
                "status='queued' AND cancel_requested=0"
            ),
            rows(&db, &other_owner)
        ),
        before,
        "recall suppression must not alter another stable owner's vector/index/outbox state"
    );
}

#[tokio::test]
async fn missing_index_source_cannot_produce_a_new_active_vector() {
    let fixture = seeded("fact");
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute("DELETE FROM memory_retrieval_index_fts", [])
            .unwrap();
        db.execute("DELETE FROM memory_retrieval_index", [])
            .unwrap();
    }
    assert!(crate::memory::embedding_jobs::run_one_embedding_batch(
        fixture.state(),
        "missing-source-derived-fixture"
    )
    .await
    .unwrap());
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        count(
            &db,
            &fixture.principal,
            "memory_vector_rows",
            "status='active'"
        ),
        0
    );
    assert_eq!(
        count(
            &db,
            &fixture.principal,
            "memory_embedding_jobs",
            "status='failed' AND error_code='memory_embedding_source_missing' AND retryable=0"
        ),
        1
    );
}
