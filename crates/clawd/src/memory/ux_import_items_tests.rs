use super::*;
use crate::memory::capacity_test_fixtures::Fixture;

fn source(content: &str, kind: &str) -> MemoryListItem {
    serde_json::from_value(json!({
        "id":"original", "revision":1, "kind":kind, "scope_kind":"current_project",
        "origin":"untrusted_source", "status":"active", "content":content, "source":"unknown",
        "evidence_available":false, "trust_tier":"untrusted", "updated_at_ts":1,
        "expires_at_ts":null, "supersedes_memory_id":null, "last_recalled_at_ts":null,
        "freshness":"fresh"
    }))
    .unwrap()
}
fn accepted(db: &Connection, owner: &str, content: &str, kind: &str) -> Value {
    match prepare(db, owner, &source(content, kind), None).unwrap() {
        Decision::Accepted(item) => item,
        other => panic!("expected accepted item, got {other:?}"),
    }
}
fn count(db: &Connection, table: &str) -> i64 {
    assert!(matches!(table, "memory_facts" | "user_preferences"));
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn legacy_character_filter_is_explicit_and_not_a_new_streaming_ceiling() {
    let env = Fixture::memory();
    let db = env.state().core.db.get().unwrap();
    let item = source("中文文", "fact");
    assert_eq!(
        prepare(&db, &env.principal, &item, Some(2)).unwrap(),
        Decision::Skipped
    );
    assert!(matches!(
        prepare(&db, &env.principal, &item, Some(3)).unwrap(),
        Decision::Accepted(_)
    ));
    let large = source(&"字".repeat(32_769), "fact");
    assert_eq!(
        prepare(&db, &env.principal, &large, Some(32_768)).unwrap(),
        Decision::Skipped
    );
    assert!(matches!(
        prepare(&db, &env.principal, &large, None).unwrap(),
        Decision::Accepted(_)
    ));
}

#[test]
fn unsupported_kinds_and_empty_content_are_skipped_for_both_ingress_paths() {
    let env = Fixture::memory();
    let db = env.state().core.db.get().unwrap();
    for (content, kind) in [("x", "unknown"), (" \n\t", "fact"), ("", "preference")] {
        for limit in [None, Some(32_768)] {
            assert_eq!(
                prepare(&db, &env.principal, &source(content, kind), limit).unwrap(),
                Decision::Skipped
            );
        }
    }
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(count(&db, "user_preferences"), 0);
}

#[test]
fn read_only_filter_preserves_provenance_without_granting_source_trust() {
    let env = Fixture::memory();
    let db = env.state().core.db.get().unwrap();
    db.pragma_update(None, "query_only", "ON").unwrap();
    let item = accepted(&db, &env.principal, "shared filter", "fact");
    assert_eq!(item["source_memory_id"], "original");
    assert_eq!(item["source_scope_kind"], "current_project");
    assert_eq!(item["source_origin"], "untrusted_source");
    assert!(item.get("trust_tier").is_none());
    assert!(db.is_autocommit());
    assert_eq!(count(&db, "memory_facts"), 0);
}

#[test]
fn duplicate_filter_is_cross_kind_but_principal_private() {
    let env = Fixture::memory();
    let key = crate::repo::auth::create_auth_key(env.state(), "user").unwrap();
    let other = crate::resolve_auth_identity_by_key(env.state(), &key)
        .unwrap()
        .unwrap();
    let db = env.state().core.db.get().unwrap();
    let item = accepted(&db, &env.principal, "same content", "fact");
    crate::memory::ux::atomic_mutation(&db, |db| {
        assert_eq!(
            insert(db, &env.principal, &env.key, "import-one", &item, 100)?,
            1
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        prepare(
            &db,
            &env.principal,
            &source("same content", "preference"),
            None
        )
        .unwrap(),
        Decision::Duplicate
    );
    assert!(matches!(
        prepare(
            &db,
            &other.principal_id,
            &source("same content", "fact"),
            None
        )
        .unwrap(),
        Decision::Accepted(_)
    ));
}

#[test]
fn all_supported_kinds_use_the_same_principal_scoped_low_trust_writer() {
    let env = Fixture::memory();
    let db = env.state().core.db.get().unwrap();
    crate::memory::ux::atomic_mutation(&db, |db| {
        for kind in ["fact", "preference", "recent"] {
            let item = accepted(db, &env.principal, &format!("content-{kind}"), kind);
            assert_eq!(
                insert(db, &env.principal, &env.key, "one-whole-import", &item, 100)?,
                1
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(count(&db, "memory_facts"), 2);
    assert_eq!(count(&db, "user_preferences"), 1);
    let downgraded:i64=db.query_row(
        "SELECT COUNT(*) FROM (
            SELECT principal_id,scope_kind,scope_ref,trust_tier,origin FROM memory_facts
            UNION ALL SELECT principal_id,scope_kind,scope_ref,trust_tier,origin FROM user_preferences
        ) WHERE principal_id=?1 AND scope_kind='principal' AND scope_ref=?1
          AND trust_tier='imported_legacy' AND origin='imported_legacy'",
        [&env.principal],|r|r.get(0)).unwrap();
    assert_eq!(downgraded, 3);
}

#[test]
fn canonical_writer_keeps_existing_fact_and_preference_idempotency() {
    let env = Fixture::memory();
    let db = env.state().core.db.get().unwrap();
    crate::memory::ux::atomic_mutation(&db, |db| {
        for kind in ["fact", "preference"] {
            let item = accepted(db, &env.principal, &format!("unique-{kind}"), kind);
            assert_eq!(
                insert(db, &env.principal, &env.key, "same-import", &item, 100)?,
                1
            );
            assert_eq!(
                insert(db, &env.principal, &env.key, "same-import", &item, 101)?,
                0
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(count(&db, "memory_facts"), 1);
    assert_eq!(count(&db, "user_preferences"), 1);
}

#[test]
fn canonical_writer_cannot_commit_an_item_outside_the_whole_import_transaction() {
    let env = Fixture::memory();
    let db = env.state().core.db.get().unwrap();
    let item = accepted(&db, &env.principal, "not independently committed", "fact");
    let error = insert(
        &db,
        &env.principal,
        &env.key,
        "missing-whole-transaction",
        &item,
        100,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "memory_import_item_requires_transaction");
    assert_eq!(count(&db, "memory_facts"), 0);
}

#[test]
fn malformed_digest_kind_or_content_is_rejected_without_panicking_or_writing() {
    let env = Fixture::memory();
    let db = env.state().core.db.get().unwrap();
    let valid = accepted(&db, &env.principal, "valid content", "fact");
    for (key, value) in [
        ("content_digest", json!("bad")),
        ("content_digest", json!("字字字字字字字字")),
        ("kind", json!("unknown")),
        ("content", json!("")),
    ] {
        let mut bad = valid.clone();
        bad[key] = value;
        let result = crate::memory::ux::atomic_mutation(&db, |db| {
            insert(db, &env.principal, &env.key, "corrupt-receipt", &bad, 100)
        });
        assert_eq!(
            result.unwrap_err().to_string(),
            "memory_import_preview_corrupt"
        );
    }
    assert_eq!(count(&db, "memory_facts"), 0);
}

#[test]
fn caller_failure_rolls_back_all_shared_item_writes() {
    let env = Fixture::memory();
    let db = env.state().core.db.get().unwrap();
    let item = accepted(&db, &env.principal, "must roll back", "fact");
    let result: anyhow::Result<()> = crate::memory::ux::atomic_mutation(&db, |db| {
        assert_eq!(
            insert(
                db,
                &env.principal,
                &env.key,
                "failed-whole-import",
                &item,
                100
            )?,
            1
        );
        anyhow::bail!("fixture_whole_import_failed")
    });
    assert!(result.is_err());
    assert_eq!(count(&db, "memory_facts"), 0);
}
