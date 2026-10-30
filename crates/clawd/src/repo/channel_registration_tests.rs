use std::{
    path::PathBuf,
    sync::{Arc, Barrier},
};

use super::*;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
    db: Connection,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("registration-journal-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("fixture.db");
        let db = Self::open(&path);
        db.execute_batch(include_str!(
            "../../../../migrations/006_pending_channel_bind_sessions.sql"
        ))
        .unwrap();
        Self { root, path, db }
    }

    fn open(path: &std::path::Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.pragma_update(None, "foreign_keys", "ON").unwrap();
        db.busy_timeout(std::time::Duration::from_secs(10)).unwrap();
        db
    }

    fn close_file(&mut self) {
        let connection = std::mem::replace(&mut self.db, Connection::open_in_memory().unwrap());
        connection.close().unwrap();
    }

    fn session(&self, channel: &str) -> PendingChannelBindSession {
        let token = format!("fixture-bind-{}", uuid::Uuid::new_v4());
        self.db.execute(
            "INSERT INTO pending_channel_bind_sessions(channel, user_key, bind_token, status,
             created_at, updated_at, expires_at, install_device_code)
             VALUES (?1, 'fixture-key', ?2, 'pending', '1', '1', '9999999999', 'fixture-device-secret')",
            params![channel, token],
        ).unwrap();
        super::super::auth::get_pending_channel_bind_session_by_id(
            &self.db,
            self.db.last_insert_rowid(),
        )
        .unwrap()
        .unwrap()
    }

    fn prepared(&mut self, channel: &str) -> PendingChannelBindSession {
        let session = self.session(channel);
        prepare_registration(&mut self.db, &session, &digest()).unwrap();
        record_registration_device(
            &mut self.db,
            &session,
            session.install_device_code.as_deref().unwrap(),
            &digest(),
        )
        .unwrap();
        session
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn digest() -> String {
    "a".repeat(64)
}

fn acquire(db: &mut Connection, session: &PendingChannelBindSession) -> RegistrationClaim {
    match claim_registration_application(db, session, &digest()).unwrap() {
        RegistrationClaimOutcome::Acquired(claim) => claim,
        other => panic!("claim_not_acquired:{other:?}"),
    }
}

#[test]
fn journal_readiness_has_no_initial_write_and_requires_current_uncertain_recovery() {
    let mut fixture = Fixture::new();
    assert!(!configuration_requires_recovery(&fixture.db, "feishu").unwrap());
    let table_exists: bool = fixture.db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'channel_registration_applications')",
        [], |row| row.get(0),
    ).unwrap();
    assert!(!table_exists);
    let session = fixture.prepared("feishu");
    assert!(!configuration_requires_recovery(&fixture.db, "feishu").unwrap());
    let claim = acquire(&mut fixture.db, &session);
    assert!(configuration_requires_recovery(&fixture.db, "feishu").unwrap());
    assert!(!configuration_requires_recovery(&fixture.db, "lark").unwrap());
    finish_registration_application(
        &fixture.db,
        &claim,
        RegistrationOutcome::ConfigurationWriteFailed,
    )
    .unwrap();
    assert!(configuration_requires_recovery(&fixture.db, "feishu").unwrap());
    advance_configuration_revision(&mut fixture.db, "lark").unwrap();
    assert!(configuration_requires_recovery(&fixture.db, "feishu").unwrap());
    advance_configuration_revision(&mut fixture.db, "feishu").unwrap();
    assert!(!configuration_requires_recovery(&fixture.db, "feishu").unwrap());
    assert_eq!(
        registration_gate(&fixture.db, &session).unwrap(),
        RegistrationGate::RecoveryRequired
    );
}

#[test]
fn journal_requires_preparation_and_a_pinned_attempt() {
    let mut fixture = Fixture::new();
    let mut session = fixture.session("feishu");
    assert_eq!(
        registration_gate(&fixture.db, &session).unwrap(),
        RegistrationGate::Unprepared
    );
    prepare_registration(&mut fixture.db, &session, &digest()).unwrap();
    validate_registration_preparation(&fixture.db, &session, &digest()).unwrap();
    assert!(validate_registration_preparation(&fixture.db, &session, &"b".repeat(64)).is_err());
    assert_eq!(
        registration_gate(&fixture.db, &session).unwrap(),
        RegistrationGate::Superseded
    );
    record_registration_device(
        &mut fixture.db,
        &session,
        "fixture-device-secret",
        &digest(),
    )
    .unwrap();
    assert!(matches!(
        registration_gate(&fixture.db, &session).unwrap(),
        RegistrationGate::Ready { .. }
    ));
    session.install_device_code = Some("fixture-replaced-secret".into());
    assert!(validate_registration_preparation(&fixture.db, &session, &digest()).is_err());
    assert_eq!(
        registration_gate(&fixture.db, &session).unwrap(),
        RegistrationGate::Superseded
    );
    assert!(record_registration_device(
        &mut fixture.db,
        &session,
        "fixture-replaced-secret",
        &digest()
    )
    .is_err());
}

#[test]
fn journal_claim_survives_last_connection_close_and_never_becomes_an_automatic_retry() {
    let mut fixture = Fixture::new();
    let session = fixture.prepared("feishu");
    let claim = acquire(&mut fixture.db, &session);
    fixture.close_file();
    let mut reopened = Fixture::open(&fixture.path);
    assert_eq!(
        registration_gate(&reopened, &session).unwrap(),
        RegistrationGate::RecoveryRequired
    );
    assert!(matches!(
        claim_registration_application(&mut reopened, &session, &digest()).unwrap(),
        RegistrationClaimOutcome::Gate(RegistrationGate::RecoveryRequired)
    ));
    finish_registration_application(&reopened, &claim, RegistrationOutcome::ServiceStarted)
        .unwrap();
    reopened.close().unwrap();
    let completed = Fixture::open(&fixture.path);
    assert_eq!(
        registration_gate(&completed, &session).unwrap(),
        RegistrationGate::Applied
    );
}

#[test]
fn journal_partial_failure_and_start_failure_have_distinct_durable_outcomes() {
    for (outcome, expected) in [
        (
            RegistrationOutcome::ConfigurationWriteFailed,
            RegistrationGate::RecoveryRequired,
        ),
        (
            RegistrationOutcome::ServiceStartFailed,
            RegistrationGate::Applied,
        ),
        (
            RegistrationOutcome::ServiceStarted,
            RegistrationGate::Applied,
        ),
    ] {
        let mut fixture = Fixture::new();
        let session = fixture.prepared("lark");
        let claim = acquire(&mut fixture.db, &session);
        finish_registration_application(&fixture.db, &claim, outcome).unwrap();
        finish_registration_application(&fixture.db, &claim, outcome).unwrap();
        fixture.close_file();
        let mut reopened = Fixture::open(&fixture.path);
        assert_eq!(registration_gate(&reopened, &session).unwrap(), expected);
        assert!(matches!(
            claim_registration_application(&mut reopened, &session, &digest()).unwrap(),
            RegistrationClaimOutcome::Gate(_)
        ));
        assert!(
            finish_registration_application(
                &reopened,
                &claim,
                RegistrationOutcome::ConfigurationWriteFailed
            )
            .is_err()
                || expected == RegistrationGate::RecoveryRequired
        );
    }
}

#[test]
fn journal_manual_revision_invalidates_only_unapplied_work_on_that_platform() {
    let mut fixture = Fixture::new();
    let first = fixture.prepared("feishu");
    let second = fixture.prepared("lark");
    advance_configuration_revision(&mut fixture.db, "feishu").unwrap();
    assert_eq!(
        registration_gate(&fixture.db, &first).unwrap(),
        RegistrationGate::Superseded
    );
    let claim = acquire(&mut fixture.db, &second);
    finish_registration_application(&fixture.db, &claim, RegistrationOutcome::ServiceStarted)
        .unwrap();
    advance_configuration_revision(&mut fixture.db, "lark").unwrap();
    assert_eq!(
        registration_gate(&fixture.db, &second).unwrap(),
        RegistrationGate::Applied
    );
}

#[test]
fn journal_duplicate_preparation_cannot_rewind_revision_or_authority() {
    let mut fixture = Fixture::new();
    let session = fixture.prepared("feishu");
    let before = revision(&fixture.db, "feishu").unwrap();
    assert_eq!(
        prepare_registration(&mut fixture.db, &session, &digest())
            .unwrap_err()
            .downcast_ref::<RegistrationJournalError>(),
        Some(&RegistrationJournalError::AlreadyPrepared)
    );
    assert_eq!(revision(&fixture.db, "feishu").unwrap(), before);
    let next = fixture.prepared("feishu");
    assert_eq!(
        registration_gate(&fixture.db, &session).unwrap(),
        RegistrationGate::Superseded
    );
    assert!(matches!(
        registration_gate(&fixture.db, &next).unwrap(),
        RegistrationGate::Ready { .. }
    ));
}

#[test]
fn journal_rejects_wrong_lease_and_configuration_without_claiming() {
    let mut fixture = Fixture::new();
    let session = fixture.prepared("feishu");
    assert!(matches!(
        claim_registration_application(&mut fixture.db, &session, &"b".repeat(64)).unwrap(),
        RegistrationClaimOutcome::Gate(RegistrationGate::Superseded)
    ));
    let claim = acquire(&mut fixture.db, &session);
    let mut wrong = claim.clone();
    wrong.token = uuid::Uuid::new_v4().to_string();
    assert!(finish_registration_application(
        &fixture.db,
        &wrong,
        RegistrationOutcome::ServiceStarted
    )
    .is_err());
    assert_eq!(
        registration_gate(&fixture.db, &session).unwrap(),
        RegistrationGate::RecoveryRequired
    );
    finish_registration_application(&fixture.db, &claim, RegistrationOutcome::ServiceStarted)
        .unwrap();
}

#[test]
fn journal_parallel_connections_acquire_exactly_one_claim() {
    let mut fixture = Fixture::new();
    let session = fixture.prepared("feishu");
    let barrier = Arc::new(Barrier::new(2));
    let threads = (0..2)
        .map(|_| {
            let path = fixture.path.clone();
            let session = session.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = Fixture::open(&path);
                barrier.wait();
                claim_registration_application(&mut db, &session, &digest()).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let outcomes = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, RegistrationClaimOutcome::Acquired(_)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                RegistrationClaimOutcome::Gate(RegistrationGate::RecoveryRequired)
            ))
            .count(),
        1
    );
}

#[test]
fn journal_reset_removes_the_claim_but_does_not_reuse_its_authority() {
    let mut fixture = Fixture::new();
    let session = fixture.prepared("feishu");
    let claim = acquire(&mut fixture.db, &session);
    fixture
        .db
        .execute(
            "DELETE FROM pending_channel_bind_sessions WHERE id = ?1",
            [session.id],
        )
        .unwrap();
    assert_eq!(
        registration_gate(&fixture.db, &session).unwrap(),
        RegistrationGate::Unprepared
    );
    assert!(finish_registration_application(
        &fixture.db,
        &claim,
        RegistrationOutcome::ServiceStarted
    )
    .is_err());
    let next = fixture.prepared("feishu");
    assert!(next.id > session.id);
    assert!(matches!(
        registration_gate(&fixture.db, &next).unwrap(),
        RegistrationGate::Ready { .. }
    ));
}

#[test]
fn journal_does_not_store_raw_identity_or_credential_material() {
    let mut fixture = Fixture::new();
    let session = fixture.prepared("feishu");
    let claim = acquire(&mut fixture.db, &session);
    finish_registration_application(
        &fixture.db,
        &claim,
        RegistrationOutcome::ConfigurationWriteFailed,
    )
    .unwrap();
    let raw = fixture
        .db
        .query_row(
            "SELECT json_array(flow_digest, channel, revision, config_digest, device_digest,
         phase, claim_token, outcome_code, updated_at) FROM channel_registration_applications",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    for marker in [
        &session.user_key,
        &session.bind_token,
        session.install_device_code.as_ref().unwrap(),
    ] {
        assert!(!raw.contains(marker));
    }
    assert!(!raw.contains("fixture-app-secret"));
    let other = fixture.session("lark");
    assert!(prepare_registration(&mut fixture.db, &other, "fixture-app-secret").is_err());
}
