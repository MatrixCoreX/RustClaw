#[test]
fn rebind_moves_only_the_selected_users_kb_data() {
    let runtime = crate::skill_storage::SkillStorageRuntime::test_default();
    let db = runtime.kb_pool().get().expect("KB db");
    db.execute(
        "INSERT INTO kb_namespaces
            (owner_user_key, namespace, payload_json, updated_at_epoch)
         VALUES (
            'rk-old', 'docs',
            '{\"namespace\":\"docs\",\"owner_user_key\":\"rk-old\",\"updated_at_epoch\":1,\"next_chunk_seq\":1,\"docs\":{},\"chunks\":[]}',
            1
         )",
        [],
    )
    .expect("namespace");
    db.execute(
        "INSERT INTO memory_retrieval_index (
            source_kind, source_ref, user_id, chat_id, user_key, memory_kind,
            search_text, metadata_json, created_at_ts, updated_at_ts
         ) VALUES (
            'kb_doc', 'kb:rk-old:docs:chunk-1', 0, 0, 'rk-old',
            'knowledge_doc', 'manual',
            '{\"owner_user_key\":\"rk-old\",\"namespace\":\"docs\"}', 1, 1
         )",
        [],
    )
    .expect("retrieval row");
    drop(db);

    assert_eq!(
        runtime
            .rebind_kb_user_key("rk-old", "rk-new")
            .expect("rebind"),
        2
    );
    let db = runtime.kb_pool().get().expect("KB db");
    let payload: String = db
        .query_row(
            "SELECT payload_json FROM kb_namespaces WHERE owner_user_key='rk-new'",
            [],
            |row| row.get(0),
        )
        .expect("payload");
    assert!(payload.contains("\"owner_user_key\":\"rk-new\""));
    let (source_ref, metadata): (String, String) = db
        .query_row(
            "SELECT source_ref, metadata_json FROM memory_retrieval_index
             WHERE user_key='rk-new'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("retrieval identity");
    assert!(source_ref.starts_with("kb:rk-new:"));
    assert!(metadata.contains("\"owner_user_key\":\"rk-new\""));
}
