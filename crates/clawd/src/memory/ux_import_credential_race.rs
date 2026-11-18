use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use crate::memory::capacity_test_fixtures::Fixture;

struct Gate {
    blocked: SyncSender<()>,
    committed: Receiver<()>,
}
thread_local! { static GATE: RefCell<Option<Gate>> = const { RefCell::new(None) }; }

fn wait_for_commit(_attempt: i32) -> bool {
    GATE.with(|slot| {
        let Some(gate) = slot.borrow_mut().take() else {
            return false;
        };
        gate.blocked.send(()).is_ok()
            && gate.committed.recv_timeout(Duration::from_secs(30)).is_ok()
    })
}

pub(super) fn revoke_during_write<T, F>(fixture: &Fixture, action: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce(&crate::AppState) -> anyhow::Result<T> + Send + 'static,
{
    {
        let first = fixture.state().core.db.get().unwrap();
        let second = fixture.state().core.db.get().unwrap();
        first.busy_handler(Some(wait_for_commit)).unwrap();
        second.busy_handler(Some(wait_for_commit)).unwrap();
    }
    let writer = fixture.competing_writer();
    writer
        .execute(
            "UPDATE credential_bindings SET status='revoked' WHERE principal_id=?1",
            [&fixture.principal],
        )
        .unwrap();
    let (blocked_tx, blocked_rx) = mpsc::sync_channel(1);
    let (committed_tx, committed_rx) = mpsc::sync_channel(1);
    let state = fixture.state().clone();
    let owner = fixture.principal.clone();
    let worker = std::thread::spawn(move || {
        GATE.with(|slot| {
            assert!(slot
                .borrow_mut()
                .replace(Gate {
                    blocked: blocked_tx,
                    committed: committed_rx
                })
                .is_none())
        });
        {
            let db = state.core.db.get().unwrap();
            crate::repo::auth::require_active_principal_credential(&db, &owner)
                .expect("revocation is not committed before the writer wait");
        }
        let result = action(&state);
        GATE.with(|slot| {
            slot.borrow_mut().take();
        });
        result
    });
    let reached = blocked_rx.recv_timeout(Duration::from_secs(30));
    let committed = writer.execute_batch("COMMIT");
    let _ = committed_tx.send(());
    let result = worker.join().expect("join owned credential race thread");
    committed.expect("commit actual revocation before releasing the writer gate");
    reached.expect("operation must reach its real SQLite writer wait");
    {
        let first = fixture.state().core.db.get().unwrap();
        let second = fixture.state().core.db.get().unwrap();
        first.busy_timeout(Duration::from_millis(50)).unwrap();
        second.busy_timeout(Duration::from_millis(50)).unwrap();
    }
    result
}
