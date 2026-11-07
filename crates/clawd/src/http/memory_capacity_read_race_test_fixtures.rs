use crate::memory::capacity_test_fixtures::Fixture;
use rusqlite::Connection;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Debug)]
struct GateState {
    armed: AtomicBool,
    timed_out: AtomicBool,
    authenticated: Mutex<Option<oneshot::Sender<()>>>,
    resume: Mutex<mpsc::Receiver<()>>,
}

#[derive(Clone, Debug)]
pub(super) struct CheckinGate(Arc<GateState>);

impl r2d2::HandleEvent for CheckinGate {
    fn handle_checkin(&self, _: r2d2::event::CheckinEvent) {
        if self.0.armed.swap(false, Ordering::SeqCst) {
            if let Some(sender) = self.0.authenticated.lock().unwrap().take() {
                let _ = sender.send(());
            }
            // A fixture handshake deadline, never a runtime task timeout.
            if self
                .0
                .resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(30))
                .is_err()
            {
                self.0.timed_out.store(true, Ordering::SeqCst);
            }
        }
    }
}

pub(super) struct Release(Option<mpsc::Sender<()>>);
impl Release {
    pub(super) fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.release();
    }
}

pub(super) struct Setup {
    pub(super) fixture: Fixture,
    pub(super) direct: Connection,
    pub(super) sibling_key: String,
    pub(super) other_key: String,
    pub(super) other_principal: String,
    pub(super) gate: CheckinGate,
    pub(super) authenticated: Option<oneshot::Receiver<()>>,
    pub(super) release: Release,
}

impl Setup {
    pub(super) fn new(admin: bool) -> Self {
        let mut fixture = Fixture::file();
        let state = fixture.state();
        let role = if admin { "admin" } else { "user" };
        let sibling_key = crate::repo::auth::create_auth_key(state, role).unwrap();
        let other_key = crate::repo::auth::create_auth_key(state, "user").unwrap();
        let other_principal = crate::resolve_auth_identity_by_key(state, &other_key)
            .unwrap()
            .unwrap()
            .principal_id;
        let path = {
            let db = state.core.db.get().unwrap();
            let transaction = db.unchecked_transaction().unwrap();
            transaction
                .execute(
                    "UPDATE auth_keys SET role=?1 WHERE user_key=?2",
                    rusqlite::params![role, fixture.key],
                )
                .unwrap();
            transaction
                .execute(
                    "UPDATE auth_keys SET principal_id=?1 WHERE user_key=?2",
                    rusqlite::params![fixture.principal, sibling_key],
                )
                .unwrap();
            transaction
                .execute(
                    "UPDATE credential_bindings SET principal_id=?1 WHERE credential_digest=?2",
                    rusqlite::params![
                        fixture.principal,
                        crate::repo::auth::credential_digest(&sibling_key)
                    ],
                )
                .unwrap();
            transaction.commit().unwrap();
            crate::memory::ux::ensure_memory_ux_schema(&db).unwrap();
            crate::memory::retention::initialize_storage_pressure(&db, &state.policy.memory, 100)
                .unwrap();
            db.query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
        };
        let direct = Connection::open(&path).unwrap();
        direct.busy_timeout(Duration::from_secs(5)).unwrap();
        assert!(crate::repo::auth::require_active_principal_credential(
            &direct,
            &fixture.principal
        )
        .is_ok());
        let (authenticated, observed) = oneshot::channel();
        let (resume, wait) = mpsc::channel();
        let gate = CheckinGate(Arc::new(GateState {
            armed: AtomicBool::new(false),
            timed_out: AtomicBool::new(false),
            authenticated: Mutex::new(Some(authenticated)),
            resume: Mutex::new(wait),
        }));
        let manager =
            r2d2_sqlite::SqliteConnectionManager::file(path).with_init(|db: &mut Connection| {
                db.busy_timeout(Duration::from_secs(5))?;
                db.pragma_update(None, "foreign_keys", "ON")?;
                db.pragma_update(None, "query_only", true)?;
                Ok(())
            });
        fixture.state_mut().core.db = r2d2::Pool::builder()
            .max_size(1)
            .event_handler(Box::new(gate.clone()))
            .build(manager)
            .unwrap();
        Self {
            fixture,
            direct,
            sibling_key,
            other_key,
            other_principal,
            gate,
            authenticated: Some(observed),
            release: Release(Some(resume)),
        }
    }

    pub(super) fn arm(&self) {
        self.gate.0.armed.store(true, Ordering::SeqCst);
    }
    pub(super) fn assert_handshake_valid(&self) {
        assert!(
            !self.gate.0.armed.load(Ordering::SeqCst),
            "actual authentication must release its checked-out connection"
        );
        assert!(
            !self.gate.0.timed_out.load(Ordering::SeqCst),
            "a fixture timeout is not an auth counterexample"
        );
    }
    pub(super) fn version(&self) -> i64 {
        self.direct
            .pragma_query_value(None, "data_version", |r| r.get(0))
            .unwrap()
    }

    pub(super) fn late_private_content(&self) -> usize {
        let content = "private state created only after the credential change".repeat(32);
        // Availability measures the source transcript payload, not a mock counter.
        self.direct
            .execute(
                "INSERT INTO memories(memory_id,user_id,chat_id,user_key,principal_id,
                scope_kind,scope_ref,channel,role,content,created_at,created_at_ts,
                memory_type,safety_flag,origin,row_revision,legacy_scope_inferred)
             VALUES ('post-auth-private',1,7,?1,?2,'principal',?2,'ui','user',?3,'102',102,
                'episodic_event','normal','user_confirmed',1,0)",
                rusqlite::params![self.sibling_key, self.fixture.principal, content],
            )
            .unwrap();
        content.len()
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        self.release.release();
        // Close the independent file handle before Fixture removes its directory.
        let direct = std::mem::replace(&mut self.direct, Connection::open_in_memory().unwrap());
        direct.close().unwrap();
    }
}
