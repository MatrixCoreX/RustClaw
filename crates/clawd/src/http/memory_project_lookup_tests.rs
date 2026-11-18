use super::fixtures::ProjectFixture;
use axum::http::StatusCode;

#[test]
fn cold_writable_get_does_not_create_project_identity() {
    let mut fixture = ProjectFixture::new(false);
    fixture.writable_pool();
    assert_eq!(fixture.count("memory_project_identities"), 0);
    let before = fixture.setup.version();
    let (status, reply) = fixture.get();
    assert_eq!(
        fixture.count("memory_project_identities"),
        0,
        "cold project GET created an unadmitted identity: {reply}"
    );
    assert_eq!(
        fixture.setup.version(),
        before,
        "cold project GET committed a write"
    );
    assert_eq!(status, StatusCode::NOT_FOUND, "{reply}");
    assert_eq!(reply["error"], "memory_project_identity_uninitialized");
    assert!(reply["data"].is_null());
    fixture.assert_no_private_metadata(&reply);
}

#[test]
fn cold_readonly_get_reports_uninitialized_without_attempting_creation() {
    let fixture = ProjectFixture::new(false);
    let before = fixture.setup.version();
    let (status, reply) = fixture.get();
    assert_eq!(fixture.setup.version(), before);
    assert_eq!(fixture.count("memory_project_identities"), 0);
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "readonly cold lookup must be a structured absence,not a failed write: {reply}"
    );
    assert_eq!(reply["error"], "memory_project_identity_uninitialized");
    assert!(reply["data"].is_null());
}

#[test]
fn known_project_does_not_backfill_an_unrelated_missing_binding() {
    let fixture = ProjectFixture::new(false);
    let project = fixture.initialize();
    let digest = crate::repo::auth::credential_digest(&fixture.setup.other_key);
    assert_eq!(
        fixture
            .setup
            .direct
            .execute(
                "DELETE FROM credential_bindings WHERE credential_digest=?1",
                [&digest],
            )
            .unwrap(),
        1
    );
    let before = fixture.setup.version();
    let (status, reply) = fixture.get();
    assert_eq!(fixture.setup.version(), before);
    assert_eq!(
        status,
        StatusCode::OK,
        "known project read was blocked by unrelated credential backfill: {reply}"
    );
    fixture.assert_scope(status, &reply, &project);
    assert_eq!(
        fixture
            .setup
            .direct
            .query_row(
                "SELECT COUNT(*) FROM credential_bindings WHERE credential_digest=?1",
                [&digest],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
        "GET cannot repair an unrelated credential"
    );
}

#[test]
fn missing_identity_migration_is_not_repaired_by_writable_get() {
    let mut fixture = ProjectFixture::new(false);
    fixture.initialize();
    fixture.writable_pool();
    assert_eq!(fixture.setup.direct.execute(
        "DELETE FROM runtime_schema_migrations WHERE migration_id='008_memory_context_identity_settings_v1'", [],
    ).unwrap(), 1);
    let before = fixture.setup.version();
    let (status, reply) = fixture.get();
    assert_eq!(
        fixture.migration_count(),
        0,
        "project GET repaired the missing identity migration: {reply}"
    );
    assert_eq!(fixture.setup.version(), before);
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{reply}");
    assert_eq!(reply["error"], "memory_project_lookup_unavailable");
    assert!(reply["data"].is_null());
}

#[test]
fn corrupt_identity_migration_is_unavailable_without_read_repair() {
    let fixture = ProjectFixture::new(false);
    fixture.initialize();
    assert_eq!(fixture.setup.direct.execute(
        "UPDATE runtime_schema_migrations SET schema_digest='sha256:corrupt-project-fixture' WHERE migration_id='008_memory_context_identity_settings_v1'", [],
    ).unwrap(), 1);
    let before = fixture.setup.version();
    let (status, reply) = fixture.get();
    assert_eq!(fixture.setup.version(), before);
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "corrupt project schema needs structured unavailability: {reply}"
    );
    assert_eq!(reply["error"], "memory_project_lookup_unavailable");
    assert!(reply["data"].is_null());
}

#[test]
fn explicit_initialization_survives_repeated_readonly_queries() {
    let fixture = ProjectFixture::new(false);
    let project = fixture.initialize();
    let before = fixture.setup.version();
    for _ in 0..3 {
        let (status, reply) = fixture.get();
        fixture.assert_scope(status, &reply, &project);
        assert_eq!(fixture.setup.version(), before);
        assert_eq!(fixture.count("memory_project_identities"), 1);
        assert_eq!(fixture.count("memory_project_aliases"), 0);
    }
}
