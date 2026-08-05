use super::{
    decode_vector_blob, encode_vector_blob, ExactSqliteVectorIndex, MemoryVectorIndex, VectorWrite,
};

#[test]
fn versioned_f32_blob_round_trips_and_rejects_corruption() {
    let vector = crate::memory::embedding::embed_text_locally("向量格式测试");
    let blob = encode_vector_blob(&vector).expect("encode");
    assert_eq!(decode_vector_blob(&blob).expect("decode"), vector);

    let mut corrupt = blob;
    corrupt.truncate(corrupt.len() - 1);
    assert_eq!(
        decode_vector_blob(&corrupt).unwrap_err().to_string(),
        "memory_vector_length_invalid"
    );
}

#[test]
fn exact_backend_searches_the_entire_eligible_scope() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().expect("db");
    db.execute(
        "INSERT OR IGNORE INTO auth_keys(user_key, role, enabled, created_at)
         VALUES ('vector-user', 'user', 1, '1')",
        [],
    )
    .unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    crate::repo::ensure_principal_ownership_schema(&db).unwrap();
    crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "vector-user")
        .unwrap()
        .unwrap();
    let profile = super::register_configured_profile(&db, &state.policy.memory).unwrap();
    let query = crate::memory::embedding::embed_text_locally("火星任务代号");
    let unrelated = crate::memory::embedding::embed_text_locally("厨房菜单");
    for (retrieval_id, vector) in [(1_i64, unrelated), (99_i64, query.clone())] {
        db.execute(
            "INSERT INTO memory_retrieval_index(
                id, source_kind, user_id, chat_id, user_key, principal_id, scope_kind,
                scope_ref, memory_kind, search_text, topic_tags, vector_json,
                embedding_model, embedding_dims, embedding_version, metadata_json,
                created_at_ts, updated_at_ts
             ) VALUES (?1, 'memory_fact', 1, 0, 'vector-user', ?2, 'principal', ?2,
                       'semantic_fact', ?3, '', '[]', 'pending', 0, 'pending', '{}', ?1, ?1)",
            rusqlite::params![retrieval_id, principal_id, format!("row-{retrieval_id}")],
        )
        .unwrap();
        ExactSqliteVectorIndex
            .upsert(
                &db,
                &profile,
                &VectorWrite {
                    retrieval_id,
                    principal_id: &principal_id,
                    scope_kind: "principal",
                    scope_ref: &principal_id,
                    projection_digest: "sha256:fixture",
                    vector: &vector,
                },
            )
            .unwrap();
    }
    let access =
        crate::memory::scope::resolve_memory_access(&db, &principal_id, None, None).unwrap();
    let nearest = ExactSqliteVectorIndex
        .nearest(&db, &access, &profile, &query, 1)
        .unwrap();
    assert_eq!(nearest[0].retrieval_id, 99);
}

#[test]
fn exact_backend_scales_to_the_configured_scope_ceiling_without_candidate_truncation() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let mut db = state.core.db.get().expect("db");
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    let profile = super::register_configured_profile(&db, &state.policy.memory).unwrap();
    let principal_id = "principal-vector-scale";
    let row_count = state.policy.memory.max_rows.max(1);
    let mut query = vec![0.0_f32; profile.dimensions];
    query[0] = 1.0;
    let mut unrelated = vec![0.0_f32; profile.dimensions];
    unrelated[1] = 1.0;
    let query_blob = encode_vector_blob(&query).unwrap();
    let query_checksum = super::digest_bytes(&query_blob);
    let unrelated_blob = encode_vector_blob(&unrelated).unwrap();
    let unrelated_checksum = super::digest_bytes(&unrelated_blob);
    let tx = db.transaction().unwrap();
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO memory_vector_rows(
                    retrieval_id, principal_id, scope_kind, scope_ref, profile_id,
                    generation, projection_version, projection_digest, vector_format,
                    dimensions, normalization, vector_blob, vector_checksum, status,
                    created_at_ts, updated_at_ts
                 ) VALUES (?1, ?2, 'principal', ?2, ?3, 1, ?4, 'scale',
                           'f32le_v1', ?5, 'unit_length', ?6, ?7, 'active', 1, 1)",
            )
            .unwrap();
        for index in 1..=row_count {
            let is_target = index == row_count;
            insert
                .execute(rusqlite::params![
                    index as i64,
                    principal_id,
                    profile.profile_id,
                    profile.projection_version,
                    profile.dimensions as i64,
                    if is_target {
                        query_blob.as_slice()
                    } else {
                        unrelated_blob.as_slice()
                    },
                    if is_target {
                        query_checksum.as_str()
                    } else {
                        unrelated_checksum.as_str()
                    },
                ])
                .unwrap();
        }
    }
    tx.commit().unwrap();
    let access =
        crate::memory::scope::resolve_memory_access(&db, principal_id, None, None).unwrap();
    let started = std::time::Instant::now();
    let nearest = ExactSqliteVectorIndex
        .nearest(&db, &access, &profile, &query, 1)
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(nearest[0].retrieval_id, row_count as i64);
    assert_eq!(nearest[0].score, 1.0);
    eprintln!(
        "MEMORY_VECTOR_SCALE_JSON {}",
        serde_json::json!({
            "backend": "exact_sqlite_f32le_v1",
            "rows": row_count,
            "dimensions": profile.dimensions,
            "elapsed_us": elapsed.as_micros(),
            "candidate_truncation": false,
        })
    );
}
