use super::*;
use crate::memory::import_upload::test_fixtures::{error, policy, Setup};

#[test]
fn ingress_projects_existing_webd_values_without_second_defaults() {
    let mut config = WebdConfig::default();
    config.request_limits.max_json_body_bytes = 1024;
    config.max_incoming_body_bytes = 256;
    config.request_limits.body_read_timeout_seconds = 17;
    config.request_limits.upload_body_read_timeout_seconds = 29;
    let ingress = IngressPolicy::from_config(&config);
    assert_eq!(ingress.chunk_bytes().unwrap(), 256);
    assert_eq!(ingress.body_read_seconds, 17);
    assert_eq!(ingress.upload_read_seconds, 29);
    let parsing = ingress.parsing(31).unwrap();
    assert_eq!(parsing.upload.chunk_bytes, 256);
    assert_eq!(parsing.value_bytes, 1024);
    assert_eq!(parsing.upload.parser_lease_seconds, 31);
}

#[test]
fn invalid_ingress_or_lease_configuration_fails_without_silent_clamping() {
    let good = IngressPolicy::from_config(&WebdConfig::default());
    for value in [
        IngressPolicy {
            json_bytes: 0,
            ..good
        },
        IngressPolicy {
            upload_bytes: 0,
            ..good
        },
        IngressPolicy {
            body_read_seconds: 0,
            ..good
        },
        IngressPolicy {
            upload_read_seconds: 0,
            ..good
        },
    ] {
        error(value.parsing(10), UploadError::IngressUnavailable);
    }
    error(good.parsing(0), UploadError::IngressUnavailable);
}

#[test]
fn policy_read_reports_the_real_persisted_budget_without_initializing_missing_state() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let ingress = IngressPolicy::from_config(&WebdConfig::default());
    let limit = super::super::super::storage_budget::Snapshot::load(&db)
        .unwrap()
        .limit;
    let read = transport_policy(&db, &env.actor, ingress, 10).unwrap();
    assert_eq!(read.max_file_bytes, limit.to_string());
    assert_eq!(read.file_limit_scope, "memory_domain_budget");
    assert_eq!(read.max_items, None);
    assert_eq!(read.max_content_characters, None);
    db.execute("DELETE FROM memory_storage_budget", []).unwrap();
    let err = transport_policy(&db, &env.actor, ingress, 10).unwrap_err();
    assert_eq!(failure(&err).unwrap().code, "memory_storage_policy_missing");
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_storage_budget", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn revoked_original_identity_cannot_read_policy_through_a_surviving_key() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    crate::memory::import_upload::test_fixtures::revoke(&db, &env.actor, "binding", &env.other);
    assert!(
        crate::repo::auth::require_active_principal_credential(&db, &env.actor.principal_id)
            .is_ok()
    );
    assert!(transport_policy(
        &db,
        &env.actor,
        IngressPolicy::from_config(&WebdConfig::default()),
        10
    )
    .is_err());
}

#[test]
fn growth_failures_keep_typed_machine_codes_and_not_provider_prose() {
    use crate::memory::growth_availability::{GrowthDecision, GrowthError};
    for (decision, kind, code) in [
        (
            GrowthDecision::StorageStateMissing,
            FailureKind::Unavailable,
            "memory_storage_state_missing",
        ),
        (
            GrowthDecision::DomainBudgetReached,
            FailureKind::StorageFull,
            "memory_storage_write_blocked",
        ),
    ] {
        let cause: anyhow::Error = GrowthError(decision).into();
        assert_eq!(cause.to_string(), code);
        let report = failure(&cause).unwrap();
        assert_eq!(report.kind, kind);
        assert_eq!(report.code, code);
        assert_eq!(report.retry_after_seconds, None);
    }
    assert!(failure(&anyhow::anyhow!("provider sentence not a machine protocol")).is_none());
}

#[test]
fn parser_rejects_an_outer_transaction_before_claim_or_staging() {
    let env = Setup::new(false);
    let upload = env.upload(b"{}", 64);
    env.put_all(&upload.upload_id, b"{}", 64);
    let db = env.fixture.state().core.db.get().unwrap();
    let transaction = db.unchecked_transaction().unwrap();
    error(
        parse_and_preview(
            &transaction,
            &env.actor,
            &upload.upload_id,
            ParsePolicy {
                upload: policy(64),
                value_bytes: 1024,
            },
            100,
        ),
        UploadError::OuterTransaction,
    );
    assert_eq!(
        status(&transaction, &env.actor, &upload.upload_id)
            .unwrap()
            .state,
        UploadState::Uploading
    );
    let rows: i64 = transaction
        .query_row("SELECT COUNT(*) FROM memory_import_upload_items", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0);
}
