use super::super::{
    automatic_generation_allowed, cleanup_memory_data, ensure_principal_quota,
    refresh_principal_usage,
};

#[path = "retention_accounting_fixtures.rs"]
mod fixtures;
use fixtures::*;

#[test]
fn single_refresh_counts_utf8_storage_bytes() {
    let (db, config) = fixture();
    source(&db, Some(PRINCIPAL), "中文🙂", NOW);
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("single refresh");
    assert_eq!(usage(&db, PRINCIPAL), (1, 10));
}

#[test]
fn single_refresh_counts_embedded_nul_bytes() {
    let (db, config) = fixture();
    source(&db, Some(PRINCIPAL), "A\0B", NOW);
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("single refresh");
    assert_eq!(usage(&db, PRINCIPAL), (1, 3));
}

#[test]
fn bulk_refresh_counts_utf8_storage_bytes() {
    let (db, config) = fixture();
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("explicit quota initialization");
    source(&db, Some(PRINCIPAL), "中文🙂", NOW);
    refresh_principal_usage(&db, &config, NOW).expect("bulk refresh");
    assert_eq!(usage(&db, PRINCIPAL), (1, 10));
}

#[test]
fn bulk_refresh_counts_embedded_nul_bytes() {
    let (db, config) = fixture();
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("explicit quota initialization");
    source(&db, Some(PRINCIPAL), "A\0B", NOW);
    refresh_principal_usage(&db, &config, NOW).expect("bulk refresh");
    assert_eq!(usage(&db, PRINCIPAL), (1, 3));
}

#[test]
fn single_refresh_enforces_multibyte_byte_quota() {
    let (db, mut config) = fixture();
    config.principal_max_bytes = 9;
    source(&db, Some(PRINCIPAL), "中文🙂", NOW);
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("single refresh");
    assert!(!automatic_generation_allowed(&db, PRINCIPAL).expect("byte quota"));
}

#[test]
fn bulk_refresh_enforces_multibyte_byte_quota() {
    let (db, mut config) = fixture();
    config.principal_max_bytes = 9;
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("explicit quota initialization");
    source(&db, Some(PRINCIPAL), "中文🙂", NOW);
    refresh_principal_usage(&db, &config, NOW).expect("bulk refresh");
    assert!(!automatic_generation_allowed(&db, PRINCIPAL).expect("byte quota"));
}

#[test]
fn bulk_refresh_clears_usage_after_last_source_is_deleted() {
    let (db, config) = fixture();
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("explicit quota initialization");
    source(&db, Some(PRINCIPAL), "alpha", NOW);
    refresh_principal_usage(&db, &config, NOW).expect("initial usage");
    assert_eq!(usage(&db, PRINCIPAL), (1, 5));
    delete_sources(&db, PRINCIPAL);
    refresh_principal_usage(&db, &config, NOW + 1).expect("empty refresh");
    assert_eq!(usage(&db, PRINCIPAL), (0, 0));
}

#[test]
fn retention_deletion_refreshes_last_source_usage() {
    let (db, mut config) = fixture();
    config.retention_days = 1;
    source(&db, Some(PRINCIPAL), "alpha", NOW - 2 * 86_400);
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("initial usage");
    assert_eq!(usage(&db, PRINCIPAL), (1, 5));
    let report = cleanup_memory_data(&db, &config, NOW).expect("retention cleanup");
    assert_eq!(report.deleted_source_rows, 1);
    assert_eq!(usage(&db, PRINCIPAL), (0, 0));
}

#[test]
fn empty_refresh_clears_usage_without_resetting_admin_policy() {
    let (db, config) = fixture();
    source(&db, Some(PRINCIPAL), "alpha", NOW);
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("initial usage");
    set_admin_policy(&db);
    let before = policy(&db);
    delete_sources(&db, PRINCIPAL);
    refresh_principal_usage(&db, &config, NOW + 1).expect("empty refresh");
    assert_eq!(usage(&db, PRINCIPAL), (0, 0));
    assert_eq!(policy(&db), before);
    assert_eq!(updated_at(&db), NOW + 1);
}

#[test]
fn bulk_refresh_keeps_principal_usage_separate() {
    let (db, config) = fixture();
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("explicit quota initialization");
    ensure_principal_quota(&db, &config, "principal-other", NOW)
        .expect("explicit second quota initialization");
    source(&db, Some(PRINCIPAL), "alpha", NOW);
    source(&db, Some("principal-other"), "beta", NOW);
    refresh_principal_usage(&db, &config, NOW).expect("separate usage");
    assert_eq!(usage(&db, PRINCIPAL), (1, 5));
    assert_eq!(usage(&db, "principal-other"), (1, 4));
}

#[test]
fn single_refresh_preserves_admin_policy_revision_and_spent_cost() {
    let (db, config) = fixture();
    source(&db, Some(PRINCIPAL), "alpha", NOW);
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("initial usage");
    set_admin_policy(&db);
    let before = policy(&db);
    source(&db, Some(PRINCIPAL), "beta", NOW);
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW + 1).expect("single refresh");
    assert_eq!(usage(&db, PRINCIPAL), (2, 9));
    assert_eq!(policy(&db), before);
}

#[test]
fn bulk_refresh_preserves_admin_policy_revision_and_spent_cost() {
    let (db, config) = fixture();
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("explicit quota initialization");
    source(&db, Some(PRINCIPAL), "alpha", NOW);
    refresh_principal_usage(&db, &config, NOW).expect("initial usage");
    set_admin_policy(&db);
    let before = policy(&db);
    source(&db, Some(PRINCIPAL), "beta", NOW);
    refresh_principal_usage(&db, &config, NOW + 1).expect("bulk refresh");
    assert_eq!(usage(&db, PRINCIPAL), (2, 9));
    assert_eq!(policy(&db), before);
}

#[test]
fn quota_refresh_does_not_bypass_global_pressure_pause() {
    let (db, config) = fixture();
    source(&db, Some(PRINCIPAL), "alpha", NOW);
    for state in ["automatic_generation_paused", "explicit_write_blocked"] {
        db.execute(
            "UPDATE memory_storage_pressure SET state = ?1,
                reason_code = 'fixture_storage_pressure', observed_bytes = 123,
                revision = 7, updated_at_ts = 1999999999 WHERE singleton_id = 1",
            [state],
        )
        .expect("global pause");
        let before = pressure(&db);
        ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("single refresh");
        refresh_principal_usage(&db, &config, NOW + 1).expect("bulk refresh");
        assert_eq!(pressure(&db), before);
        assert!(!automatic_generation_allowed(&db, PRINCIPAL).expect("global protection"));
    }
}

#[test]
fn empty_source_does_not_refund_spent_background_cost() {
    let (db, mut config) = fixture();
    config.principal_background_cost_microunits = 7;
    source(&db, Some(PRINCIPAL), "alpha", NOW);
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("initial usage");
    db.execute(
        "UPDATE memory_principal_quotas SET used_background_cost_microunits = 7
         WHERE principal_id = ?1",
        [PRINCIPAL],
    )
    .expect("spent background cost");
    delete_sources(&db, PRINCIPAL);
    refresh_principal_usage(&db, &config, NOW + 1).expect("empty refresh");
    assert_eq!(policy(&db).3, 7);
    assert!(!automatic_generation_allowed(&db, PRINCIPAL).expect("spent cost remains spent"));
}

#[test]
fn already_empty_quota_refresh_is_write_free() {
    let (db, config) = fixture();
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("initial empty quota");
    let before = total_changes(&db);
    refresh_principal_usage(&db, &config, NOW + 1).expect("empty refresh");
    assert_eq!(total_changes(&db), before);
    assert_eq!(usage(&db, PRINCIPAL), (0, 0));
    assert_eq!(updated_at(&db), NOW);
}

#[test]
fn unassigned_sources_are_not_charged_to_other_principals() {
    let (db, config) = fixture();
    ensure_principal_quota(&db, &config, PRINCIPAL, NOW).expect("explicit quota initialization");
    source(&db, None, "unowned", NOW);
    source(&db, Some(PRINCIPAL), "alpha", NOW);
    refresh_principal_usage(&db, &config, NOW).expect("principal usage");
    assert_eq!(usage(&db, PRINCIPAL), (1, 5));
    let quota_count: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_principal_quotas", [], |row| {
            row.get(0)
        })
        .expect("only assigned principal has a quota");
    assert_eq!(quota_count, 1);
    let unassigned: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE principal_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("unassigned source remains unassigned");
    assert_eq!(unassigned, 1);
}
