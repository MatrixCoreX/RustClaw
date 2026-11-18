use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use rusqlite::{params, Connection};

use super::capacity_test_fixtures::{pressure, Fixture};

struct ClaimGate {
    blocked: SyncSender<()>,
    committed: Receiver<()>,
}

thread_local! {
    static CLAIM_GATE: RefCell<Option<ClaimGate>> = const { RefCell::new(None) };
}

fn wait_for_pressure_commit(_attempt: i32) -> bool {
    CLAIM_GATE.with(|slot| {
        let Some(gate) = slot.borrow_mut().take() else {
            return false;
        };
        gate.blocked.send(()).is_ok()
            && gate.committed.recv_timeout(Duration::from_secs(30)).is_ok()
    })
}

/// The worker reads committed normal capacity, then demonstrably blocks on
/// BEGIN IMMEDIATE. The competing writer commits the pause before releasing
/// that exact busy callback. No sleep, production hook or test-only scheduler
/// replacement can stand in for the actual claim transaction here.
pub(super) fn pause_inside_claim<T, F>(fixture: &Fixture, claim: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce(&crate::AppState) -> anyhow::Result<T> + Send + 'static,
{
    {
        let first = fixture.state().core.db.get().unwrap();
        let second = fixture.state().core.db.get().unwrap();
        pressure(&first, "normal");
        first.busy_handler(Some(wait_for_pressure_commit)).unwrap();
        second.busy_handler(Some(wait_for_pressure_commit)).unwrap();
    }
    let writer = fixture.competing_writer();
    pressure(&writer, "automatic_generation_paused");
    let (blocked_tx, blocked_rx) = mpsc::sync_channel(1);
    let (committed_tx, committed_rx) = mpsc::sync_channel(1);
    let state = fixture.state().clone();
    let worker = std::thread::spawn(move || {
        CLAIM_GATE.with(|slot| {
            assert!(slot
                .borrow_mut()
                .replace(ClaimGate {
                    blocked: blocked_tx,
                    committed: committed_rx,
                })
                .is_none());
        });
        {
            let db = state.core.db.get().unwrap();
            let visible: String = db
                .query_row("SELECT state FROM memory_storage_pressure", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                visible, "normal",
                "the competing writer has not committed yet"
            );
        }
        let result = claim(&state);
        CLAIM_GATE.with(|slot| {
            slot.borrow_mut().take();
        });
        result
    });
    let reached_write_gate = blocked_rx.recv_timeout(Duration::from_secs(30));
    // Always release and join our own worker before reporting a test failure.
    let committed = writer.execute_batch("COMMIT");
    let _ = committed_tx.send(());
    let result = worker.join().expect("isolated claim thread");
    committed.expect("commit pressure before allowing BEGIN IMMEDIATE");
    reached_write_gate.expect("actual claim must reach the competing writer gate");
    {
        let first = fixture.state().core.db.get().unwrap();
        let second = fixture.state().core.db.get().unwrap();
        first.busy_timeout(Duration::from_millis(50)).unwrap();
        second.busy_timeout(Duration::from_millis(50)).unwrap();
    }
    result
}

pub(super) fn another_principal(fixture: &Fixture) -> String {
    let key = crate::repo::auth::create_auth_key(fixture.state(), "user").unwrap();
    let principal = crate::resolve_auth_identity_by_key(fixture.state(), &key)
        .unwrap()
        .unwrap()
        .principal_id;
    let db = fixture.state().core.db.get().unwrap();
    super::retention::ensure_principal_quota(
        &db,
        &fixture.state().policy.memory,
        &principal,
        crate::now_ts_u64() as i64,
    )
    .unwrap();
    principal
}

pub(super) fn copy_queued_job_for(db: &Connection, original: &str, id: &str, principal: &str) {
    assert_eq!(
        db.execute(
            "INSERT INTO memory_jobs(
            job_id, job_kind, principal_id, scope_kind, scope_ref, source_digest,
            eligibility_json, settings_revision, policy_digest, provider_name,
            provider_type, model_name, model_capability_digest, status,
            checkpoint_json, not_before_ts, created_at_ts, updated_at_ts
         ) SELECT ?2, job_kind, ?3, 'principal', ?3, source_digest, eligibility_json,
                  settings_revision, policy_digest, provider_name, provider_type,
                  model_name, model_capability_digest, 'queued', checkpoint_json,
                  not_before_ts + 1, created_at_ts + 1, updated_at_ts
           FROM memory_jobs WHERE job_id = ?1",
            params![original, id, principal],
        )
        .unwrap(),
        1
    );
}

#[test]
fn busy_gate_observes_normal_before_writer_wait_and_pause_inside_transaction() {
    let fixture = Fixture::file();
    let inside = pause_inside_claim(&fixture, |state| {
        let mut db = state.core.db.get().unwrap();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let visible: String =
            tx.query_row("SELECT state FROM memory_storage_pressure", [], |row| {
                row.get(0)
            })?;
        tx.rollback()?;
        Ok(visible)
    })
    .unwrap();
    assert_eq!(inside, "automatic_generation_paused");
}
