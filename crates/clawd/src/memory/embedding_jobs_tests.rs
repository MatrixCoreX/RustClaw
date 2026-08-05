use crate::memory::vector_store::MemoryVectorIndex;

#[tokio::test]
async fn local_embedding_outbox_commits_a_versioned_vector_after_source_write() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().expect("db");
    db.execute(
        "INSERT OR IGNORE INTO auth_keys(user_key, role, enabled, created_at)
         VALUES ('embedding-user', 'user', 1, '1')",
        [],
    )
    .unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    crate::repo::ensure_principal_ownership_schema(&db).unwrap();
    crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
    super::super::vector_store::register_configured_profile(&db, &state.policy.memory).unwrap();
    crate::memory::indexing::index_preference_entries(
        &db,
        1,
        0,
        "embedding-user",
        &[(
            "answer_language".to_string(),
            "简体中文".to_string(),
            0.9,
            "user".to_string(),
        )],
        1,
    )
    .unwrap();
    let queued: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status = 'queued'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(queued, 1);
    drop(db);

    assert!(
        super::run_one_embedding_batch(&state, "embedding-test-worker")
            .await
            .unwrap()
    );
    let db = state.core.db.get().unwrap();
    let (completed, vector_count): (i64, i64) = (
        db.query_row(
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status = 'completed'",
            [],
            |row| row.get(0),
        )
        .unwrap(),
        db.query_row(
            "SELECT COUNT(*) FROM memory_vector_rows WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap(),
    );
    assert_eq!((completed, vector_count), (1, 1));
}

#[test]
fn remote_reindex_requires_explicit_consent_before_outbound_jobs_exist() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    crate::repo::ensure_principal_ownership_schema(&db).unwrap();
    crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
    let mut config = state.policy.memory.clone();
    config.embedding_provider_kind = "remote_http".to_string();
    config.embedding_endpoint_ref = "MEMORY_EMBEDDING_TEST_ENDPOINT".to_string();
    config.embedding_credential_ref = "MEMORY_EMBEDDING_TEST_TOKEN".to_string();
    config.embedding_model = "fixture-embedding".to_string();
    config.embedding_dims = 3;
    let error = super::enqueue_reindex(&db, &config, "principal:test", "sha256:no-consent", false)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "memory_embedding_remote_consent_required"
    );
    let jobs: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_embedding_jobs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(jobs, 0);
}

#[derive(Default)]
struct PayloadSplittingProvider {
    batch_sizes: std::sync::Mutex<Vec<usize>>,
}

impl crate::memory::embedding::MemoryEmbeddingProvider for PayloadSplittingProvider {
    fn spec(&self) -> crate::memory::embedding::MemoryEmbeddingSpec {
        crate::memory::embedding::local_hash_embedding_spec()
    }

    fn embed_batch<'a>(
        &'a self,
        items: &'a [crate::memory::embedding::EmbeddingRequestItem],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Vec<crate::memory::embedding::EmbeddingResponseItem>,
                        crate::memory::embedding::EmbeddingProviderError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.batch_sizes.lock().unwrap().push(items.len());
            if items.len() > 1 {
                return Err(crate::memory::embedding::EmbeddingProviderError {
                    error_code: "memory_embedding_payload_too_large",
                    retryable: false,
                    retry_after_seconds: None,
                    status_code: Some(413),
                });
            }
            Ok(items
                .iter()
                .map(|item| crate::memory::embedding::EmbeddingResponseItem {
                    request_item_id: item.request_item_id.clone(),
                    vector: crate::memory::embedding::embed_text_locally(&item.text),
                })
                .collect())
        })
    }
}

#[tokio::test]
async fn payload_413_is_split_without_losing_stable_item_order() {
    let provider = PayloadSplittingProvider::default();
    let requests = (0..3)
        .map(|index| crate::memory::embedding::EmbeddingRequestItem {
            request_item_id: format!("item-{index}"),
            text: format!("fixture {index}"),
        })
        .collect::<Vec<_>>();
    let response = super::embed_batch_with_payload_split(&provider, &requests, usize::MAX)
        .await
        .expect("split response");
    assert_eq!(
        response
            .iter()
            .map(|item| item.request_item_id.as_str())
            .collect::<Vec<_>>(),
        vec!["item-0", "item-1", "item-2"]
    );
    assert_eq!(
        provider.batch_sizes.lock().unwrap().as_slice(),
        &[3, 1, 2, 1, 1]
    );
}

#[tokio::test]
async fn blue_green_reindex_activates_per_principal_and_keeps_old_generation_until_complete() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().unwrap();
    db.execute(
        "INSERT OR IGNORE INTO auth_keys(user_key, role, enabled, created_at)
         VALUES ('reindex-user', 'user', 1, '1')",
        [],
    )
    .unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    crate::repo::ensure_principal_ownership_schema(&db).unwrap();
    crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "reindex-user")
        .unwrap()
        .unwrap();
    crate::memory::indexing::index_preference_entries(
        &db,
        1,
        0,
        "reindex-user",
        &[(
            "format".to_string(),
            "short".to_string(),
            1.0,
            "user".to_string(),
        )],
        1,
    )
    .unwrap();
    let (_snapshot, generation, rows) = super::enqueue_reindex(
        &db,
        &state.policy.memory,
        &principal_id,
        "sha256:local-policy",
        false,
    )
    .unwrap();
    assert_eq!(generation, 2);
    assert_eq!(rows, 1);
    let before = crate::memory::vector_store::active_generation_for_principal(
        &db,
        &principal_id,
        crate::memory::vector_store::LOCAL_PROFILE_ID,
        1,
    )
    .unwrap();
    assert_eq!(before, 1);
    drop(db);

    while super::run_one_embedding_batch(&state, "reindex-test-worker")
        .await
        .unwrap()
    {}
    let db = state.core.db.get().unwrap();
    let after = crate::memory::vector_store::active_generation_for_principal(
        &db,
        &principal_id,
        crate::memory::vector_store::LOCAL_PROFILE_ID,
        1,
    )
    .unwrap();
    assert_eq!(after, 2);
    let snapshot_state: String = db
        .query_row(
            "SELECT state FROM memory_vector_snapshots
             WHERE principal_id = ?1 AND profile_id = ?2 AND generation = 2",
            rusqlite::params![principal_id, crate::memory::vector_store::LOCAL_PROFILE_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(snapshot_state, "active");
}

#[test]
fn withdrawing_remote_consent_cancels_jobs_and_tombstones_only_that_principal() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    crate::memory::vector_store::ensure_vector_pipeline_schema(&db).unwrap();
    let mut config = state.policy.memory.clone();
    config.embedding_provider_kind = "remote_http".to_string();
    config.embedding_endpoint_ref = "MEMORY_EMBEDDING_TEST_ENDPOINT".to_string();
    config.embedding_credential_ref = "MEMORY_EMBEDDING_TEST_TOKEN".to_string();
    config.embedding_model = "fixture-embedding".to_string();
    config.embedding_dims = 3;
    let profile = crate::memory::vector_store::register_configured_profile(&db, &config).unwrap();
    db.execute(
        "INSERT INTO memory_embedding_jobs(
            job_id, retrieval_id, principal_id, scope_kind, scope_ref, profile_id,
            profile_generation, request_item_id, projection_version, projection_digest,
            consent_policy_digest, status, not_before_ts, created_at_ts, updated_at_ts
         ) VALUES ('remote-job-a', 1, 'principal-a', 'principal', 'principal-a', ?1,
                   1, 'item-a', 'v1', 'digest-a', 'policy-a', 'queued', 1, 1, 1),
                  ('remote-job-b', 2, 'principal-b', 'principal', 'principal-b', ?1,
                   1, 'item-b', 'v1', 'digest-b', 'policy-b', 'queued', 1, 1, 1)",
        [profile.profile_id.as_str()],
    )
    .unwrap();
    let vector = vec![1.0_f32, 0.0, 0.0];
    for (retrieval_id, principal_id) in [(1_i64, "principal-a"), (2, "principal-b")] {
        crate::memory::vector_store::ExactSqliteVectorIndex
            .upsert(
                &db,
                &profile,
                &crate::memory::vector_store::VectorWrite {
                    retrieval_id,
                    principal_id,
                    scope_kind: "principal",
                    scope_ref: principal_id,
                    projection_digest: "digest",
                    vector: &vector,
                },
            )
            .unwrap();
    }
    let (cancelled, tombstoned) =
        super::revoke_remote_profiles_for_principal(&db, "principal-a").unwrap();
    assert_eq!((cancelled, tombstoned), (1, 1));
    let other_cancelled: i64 = db
        .query_row(
            "SELECT cancel_requested FROM memory_embedding_jobs WHERE job_id = 'remote-job-b'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let other_status: String = db
        .query_row(
            "SELECT status FROM memory_vector_rows WHERE retrieval_id = 2 AND profile_id = ?1",
            [profile.profile_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(other_cancelled, 0);
    assert_eq!(other_status, "active");
}

#[test]
fn cancelling_reindex_preserves_the_active_generation() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    crate::memory::vector_store::ensure_vector_pipeline_schema(&db).unwrap();
    let profile =
        crate::memory::vector_store::register_configured_profile(&db, &state.policy.memory)
            .unwrap();
    db.execute(
        "INSERT INTO memory_vector_snapshots(
            snapshot_id, principal_id, profile_id, generation, row_count,
            source_digest, snapshot_checksum, state, created_at_ts, updated_at_ts
         ) VALUES ('snapshot-active', 'principal-a', ?1, 1, 1, 'source-a',
                   'checksum-a', 'active', 1, 1),
                  ('snapshot-building', 'principal-a', ?1, 2, 1, 'source-b',
                   '', 'building', 2, 2)",
        [profile.profile_id.as_str()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO memory_embedding_jobs(
            job_id, retrieval_id, principal_id, scope_kind, scope_ref, profile_id,
            profile_generation, request_item_id, projection_version, projection_digest,
            consent_policy_digest, status, not_before_ts, created_at_ts, updated_at_ts
         ) VALUES ('active-job', 1, 'principal-a', 'principal', 'principal-a', ?1,
                   1, 'active-item', 'v1', 'active-digest', 'policy-a', 'completed', 1, 1, 1),
                  ('building-job', 2, 'principal-a', 'principal', 'principal-a', ?1,
                   2, 'building-item', 'v1', 'building-digest', 'policy-a', 'queued', 1, 1, 1)",
        [profile.profile_id.as_str()],
    )
    .unwrap();
    let vector = crate::memory::embedding::embed_text_locally("fixture");
    for (retrieval_id, generation, digest) in
        [(1_i64, 1_u64, "active-digest"), (2, 2, "building-digest")]
    {
        let mut generation_profile = profile.clone();
        generation_profile.generation = generation;
        crate::memory::vector_store::ExactSqliteVectorIndex
            .upsert(
                &db,
                &generation_profile,
                &crate::memory::vector_store::VectorWrite {
                    retrieval_id,
                    principal_id: "principal-a",
                    scope_kind: "principal",
                    scope_ref: "principal-a",
                    projection_digest: digest,
                    vector: &vector,
                },
            )
            .unwrap();
    }

    assert_eq!(
        super::cancel_profile_jobs(&db, "principal-a", &profile.profile_id).unwrap(),
        1
    );
    let statuses = db
        .prepare(
            "SELECT generation, status FROM memory_vector_rows
             WHERE principal_id = 'principal-a' AND profile_id = ?1 ORDER BY generation",
        )
        .unwrap()
        .query_map([profile.profile_id.as_str()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        statuses,
        vec![(1, "active".to_string()), (2, "tombstone".to_string())]
    );
    let snapshot_states = db
        .prepare(
            "SELECT generation, state FROM memory_vector_snapshots
             WHERE principal_id = 'principal-a' AND profile_id = ?1 ORDER BY generation",
        )
        .unwrap()
        .query_map([profile.profile_id.as_str()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        snapshot_states,
        vec![(1, "active".to_string()), (2, "corrupt".to_string())]
    );
}

#[test]
fn retryable_failures_open_a_principal_profile_circuit_and_success_resets_it() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().expect("db");
    let mut config = state.policy.memory.clone();
    config.embedding_circuit_failure_threshold = 2;
    config.embedding_circuit_reset_seconds = 60;
    let error = crate::memory::embedding::EmbeddingProviderError {
        error_code: "memory_embedding_rate_limited",
        retryable: true,
        retry_after_seconds: Some(7),
        status_code: Some(429),
    };

    super::record_provider_failure(&db, "principal-a", "profile-a", &error, &config)
        .expect("first failure");
    assert!(!super::provider_circuit_open(
        &db,
        "principal-a",
        "profile-a",
        crate::now_ts_u64() as i64
    )
    .unwrap());
    super::record_provider_failure(&db, "principal-a", "profile-a", &error, &config)
        .expect("second failure");
    assert!(super::provider_circuit_open(
        &db,
        "principal-a",
        "profile-a",
        crate::now_ts_u64() as i64
    )
    .unwrap());
    assert!(!super::provider_circuit_open(
        &db,
        "principal-b",
        "profile-a",
        crate::now_ts_u64() as i64
    )
    .unwrap());

    super::reset_provider_circuit(&db, "principal-a", "profile-a").unwrap();
    assert!(!super::provider_circuit_open(
        &db,
        "principal-a",
        "profile-a",
        crate::now_ts_u64() as i64
    )
    .unwrap());
}
