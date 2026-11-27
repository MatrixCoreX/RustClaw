use super::*;

fn insert_kb_doc_row(
    db: &rusqlite::Connection,
    user_key: &str,
    source_ref: &str,
    text: &str,
    ts: i64,
) {
    db.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, source_memory_id, source_pref_key, source_ref, user_id, chat_id, user_key,
                memory_kind, role, search_text, trigger_text, topic_tags, vector_json, metadata_json,
                salience, success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             )
             VALUES (?1, NULL, NULL, ?2, 0, 0, ?3, ?4, NULL, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            rusqlite::params![
                crate::memory::RETRIEVAL_SOURCE_KB_DOC,
                source_ref,
                user_key,
                crate::memory::RETRIEVAL_KIND_KNOWLEDGE_DOC,
                text,
                crate::memory::retrieval::build_topic_tags(text),
                crate::memory::retrieval::vector_to_json(
                    &crate::memory::embedding::embed_text_locally(text),
                ),
                r#"{"scope_kind":"user","namespace":"photo_docs","path":"photo_rules.md"}"#,
                0.78_f32,
                crate::memory::RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
                crate::memory::RETRIEVAL_PRODUCER_KB,
                ts,
            ],
        )
        .expect("insert kb doc row");
    db.execute(
        "INSERT INTO memory_retrieval_index_fts(rowid, search_text, topic_tags)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![
            db.last_insert_rowid(),
            text,
            crate::memory::retrieval::build_topic_tags(text)
        ],
    )
    .expect("insert fixture KB lexical projection");
}

pub(super) fn seed_photo_organize_policy_memory(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) {
    let db = state.core.db.get().expect("db pool");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("ensure retrieval schema");
    crate::memory::embedding_jobs::initialize_embedding_runtime(&db, &state.policy.memory)
        .expect("initialize owned memory projections");
    let principal = crate::repo::auth::principal_id_for_user_key(&db, user_key)
        .unwrap()
        .unwrap();
    let ts = crate::now_ts_u64() as i64;

    db.execute(
            "INSERT INTO user_preferences (user_id, chat_id, user_key, pref_key, pref_value, confidence, source, updated_at, updated_at_ts, principal_id, scope_ref, memory_id)
             VALUES (?1, ?2, ?3, 'photo_grouping', 'PHOTO_ALLOWED_PREF_by_year_month', 0.95, 'test', ?4, ?4, ?5, ?5, 'photo-pref-fixture')",
            rusqlite::params![user_id, chat_id, user_key, ts, principal],
        )
        .expect("insert preference");
    db.execute(
            "INSERT INTO long_term_memories (user_id, chat_id, user_key, summary, source_memory_id, created_at, updated_at, created_at_ts, updated_at_ts, principal_id, scope_ref)
             VALUES (?1, ?2, ?3, 'PHOTO_BLOCKED_LONG_TERM_SUMMARY', 1, ?4, ?4, ?4, ?4, ?5, ?5)",
            rusqlite::params![user_id, chat_id, user_key, ts, principal],
        )
        .expect("insert long term summary");

    crate::memory::indexing::upsert_knowledge_fact(
        &db,
        user_id,
        user_key,
        "photo_profile",
        crate::memory::RETRIEVAL_KIND_SEMANTIC_FACT,
        "test:photo:allowed-fact",
        "PHOTO_ALLOWED_FACT 请准备照片整理候选目录 prefer grouping travel photos by capture date",
        ts,
    )
    .expect("insert knowledge fact");
    let kb_db = state
        .core
        .skill_storage
        .pool_for("kb")
        .expect("KB storage owner")
        .get()
        .expect("KB db pool");
    // Indexed KB recall requires an owner-matching namespace and source reference.
    kb_db
        .execute(
            "INSERT INTO kb_namespaces
            (owner_user_key, namespace, updated_at_epoch, next_chunk_seq,
             revision, parser_version, chunker_version, embedding_version)
         VALUES (?1, 'photo_docs', ?2, 1, 1, 'plain-v1', 'chars-v1', 'none')",
            rusqlite::params![user_key, ts],
        )
        .expect("insert fixture KB namespace");
    insert_kb_doc_row(
        &kb_db,
        user_key,
        &format!("kb:{user_key}:photo_docs:photo_rules:0"),
        "PHOTO_ALLOWED_KB_DOC 请准备照片整理候选目录 preserve original EXIF timestamp during organization",
        ts,
    );
    for (offset, role, text, kind, salience, instructional) in [
        (
            1,
            crate::memory::MEMORY_ROLE_USER,
            "PHOTO_BLOCKED_RECENT_EVENT previous photo operation command",
            crate::memory::MEMORY_TYPE_GENERIC,
            0.9,
            true,
        ),
        (
            2,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            "PHOTO_BLOCKED_ASSISTANT_RESULT previous classified folder result",
            crate::memory::MEMORY_TYPE_ASSISTANT_OUTCOME,
            0.9,
            false,
        ),
        (
            3,
            crate::memory::MEMORY_ROLE_USER,
            "PHOTO_BLOCKED_UNFINISHED_GOAL continue moving all photos now",
            crate::memory::MEMORY_TYPE_UNFINISHED_GOAL,
            0.9,
            false,
        ),
    ] {
        db.execute(
            "INSERT INTO memories(id,user_id,chat_id,user_key,role,content,created_at,created_at_ts,principal_id,scope_kind,scope_ref,memory_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?7,?8,'principal',?8,?9)",
            rusqlite::params![100+offset,user_id,chat_id,user_key,role,text,ts+offset,principal,format!("photo-fixture-{offset}")],
        ).expect("insert canonical photo fixture memory");
        crate::memory::indexing::index_memory_row(
            &db,
            user_id,
            chat_id,
            user_key,
            100 + offset,
            role,
            text,
            kind,
            salience,
            instructional,
            ts + offset,
        )
        .expect("index canonical photo fixture memory");
    }
}
