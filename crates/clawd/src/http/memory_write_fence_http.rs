use super::*;
use std::{
    fmt,
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
use tower::ServiceExt;
use tracing::{
    field::{Field, Visit},
    Event, Subscriber,
};
use tracing_subscriber::{layer::Context, prelude::*, Layer};

#[derive(Clone, Copy)]
pub(super) enum Change {
    Key,
    Binding,
    Role,
    Reassign,
    StalePolicy,
    LateRevoke,
    SqlFault,
    None,
}
struct QueueLayer(Arc<Mutex<Option<oneshot::Sender<()>>>>);
struct Fields(bool);
impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "memory_write_stage" && value == "queued" {
            self.0 = true;
        }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn fmt::Debug) {}
}
impl<S: Subscriber> Layer<S> for QueueLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut fields = Fields(false);
        event.record(&mut fields);
        if fields.0 {
            if let Some(sender) = self.0.lock().unwrap().take() {
                let _ = sender.send(());
            }
        }
    }
}
struct Gate(Option<std::sync::mpsc::Sender<()>>);
impl Gate {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}

pub(super) fn scenario(action: support::Action, change: Change) {
    use support::Action;
    let fixture = support::Fixture::new(action);
    let (method, path, body) = fixture.request(action);
    if matches!(change, Change::LateRevoke | Change::SqlFault) {
        let db = fixture.state.core.db.get().unwrap();
        let sql = if matches!(change, Change::LateRevoke) {
            format!("CREATE TRIGGER late_settings_revoke AFTER INSERT ON memory_runtime_settings
                WHEN NEW.principal_id='{}' BEGIN UPDATE auth_keys SET enabled=0 WHERE user_key='{}'; END;",
                fixture.actor.principal_id,fixture.actor.user_key)
        } else {
            match action {
                Action::Reindex => "CREATE TRIGGER late_reindex_fault BEFORE INSERT ON memory_embedding_jobs BEGIN SELECT RAISE(ABORT,'late_reindex_fault'); END;".into(),
                Action::Cancel => "CREATE TRIGGER late_cancel_fault BEFORE UPDATE OF state ON memory_vector_snapshots WHEN NEW.state='corrupt' BEGIN SELECT RAISE(ABORT,'late_cancel_fault'); END;".into(),
                Action::Link => "CREATE TRIGGER late_project_fault BEFORE INSERT ON memory_project_aliases BEGIN SELECT RAISE(ABORT,'late_project_fault'); END;".into(),
                _ => panic!("unsupported fault fixture"),
            }
        };
        db.execute_batch(&sql).unwrap();
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (release,wait)=std::sync::mpsc::channel();
        let mut gate=Gate(Some(release));
        let (started,ready)=oneshot::channel();
        let blocker=tokio::task::spawn_blocking(move || {let _=started.send(());let _=wait.recv();});
        tokio::time::timeout(std::time::Duration::from_secs(30),ready).await.unwrap().unwrap();
        let (queued,observed)=oneshot::channel();
        let subscriber=tracing_subscriber::registry().with(QueueLayer(Arc::new(Mutex::new(Some(queued)))));
        let dispatch=tracing::Dispatch::new(subscriber);
        let request=axum::http::Request::builder().method(method).uri(&path)
            .header(claw_core::product_identity::AUTH_KEY_HEADER,&fixture.actor.user_key)
            .header("content-type","application/json").body(axum::body::Body::from(body.to_string())).unwrap();
        let mut future=Box::pin(super::super::router().with_state(fixture.state.clone()).oneshot(request));
        let future=std::future::poll_fn(move |context|tracing::dispatcher::with_default(&dispatch,||future.as_mut().poll(context)));
        let request=tokio::spawn(future);
        // This test-only deadline protects the post-authentication handshake.
        tokio::time::timeout(std::time::Duration::from_secs(30),observed).await
            .expect("the actual authenticated HTTP request must enter its write queue").unwrap();
        {
            let db=fixture.state.core.db.get().unwrap();
            match change {
                Change::Key => {assert_eq!(db.execute("UPDATE auth_keys SET enabled=0 WHERE user_key=?1",[&fixture.actor.user_key]).unwrap(),1);}
                Change::Binding => {assert_eq!(db.execute("UPDATE credential_bindings SET status='revoked' WHERE credential_digest=?1",[crate::repo::auth::credential_digest(&fixture.actor.user_key)]).unwrap(),1);}
                Change::Role => {assert_eq!(db.execute("UPDATE auth_keys SET role='user' WHERE user_key=?1",[&fixture.actor.user_key]).unwrap(),1);}
                Change::Reassign => {
                    assert_eq!(db.execute("UPDATE auth_keys SET principal_id=?1 WHERE user_key=?2",rusqlite::params![fixture.other,fixture.actor.user_key]).unwrap(),1);
                    assert_eq!(db.execute("UPDATE credential_bindings SET principal_id=?1 WHERE credential_digest=?2",rusqlite::params![fixture.other,crate::repo::auth::credential_digest(&fixture.actor.user_key)]).unwrap(),1);
                }
                _ => {}
            }
            assert!(crate::repo::auth::require_active_principal_credential(&db,&fixture.actor.principal_id).is_ok(),"a sibling live key is not authority for this request");
        }
        if matches!(change,Change::StalePolicy) {fixture.change_settings();}
        // Preserve the legitimate competing edit; reject only this queued request's effects.
        let before=fixture.state();
        gate.release();
        let response=request.await.unwrap().unwrap();
        let status=response.status();
        let bytes=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
        let reply:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
        blocker.await.unwrap();
        if matches!(change,Change::None) {
            assert_eq!(status,StatusCode::OK,"{action:?}: {reply}");
            assert_eq!(reply["ok"],true);
            assert_ne!(fixture.state(),before,"a real accepted mutation must have observable effects");
            fixture.assert_success(action,&reply);
        } else {
            let (expected,code)=match change {
                Change::Key|Change::Binding|Change::LateRevoke => (StatusCode::UNAUTHORIZED,"request_credential_revoked"),
                Change::Role|Change::Reassign => (StatusCode::FORBIDDEN,"request_identity_changed"),
                Change::StalePolicy => (StatusCode::CONFLICT,"memory_settings_revision_conflict"),
                Change::SqlFault => (StatusCode::BAD_REQUEST,match action {
                    Action::Reindex=>"memory_vector_reindex_failed",
                    Action::Cancel=>"memory_vector_control_failed",
                    Action::Link=>"memory_project_link_failed",
                    _=>unreachable!(),
                }),
                Change::None=>unreachable!(),
            };
            assert_eq!(status,expected,"{action:?}: {reply}");
            assert_eq!(reply["ok"],false);
            assert_eq!(reply["error"],code);
            assert!(reply["data"].is_null());
            assert_eq!(fixture.state(),before,"rejection must preserve canonical/index/job/settings/project/audit state");
            for secret in [&fixture.actor.user_key,"actor-private-text","other-private-text","recent-private-text","late_reindex_fault","late_cancel_fault","late_project_fault"] {
                assert!(!reply.to_string().contains(secret));
            }
            if matches!(change,Change::LateRevoke) {
                let db=fixture.state.core.db.get().unwrap();
                assert_eq!(db.query_row("SELECT enabled FROM auth_keys WHERE user_key=?1",[&fixture.actor.user_key],|row|row.get::<_,i64>(0)).unwrap(),1);
            }
        }
    });
}
pub(super) fn empty_reindex_nested() {
    let fixture = support::Fixture::new(support::Action::Reindex);
    let db = fixture.state.core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_retrieval_index WHERE principal_id=?1",
        [&fixture.actor.principal_id],
    )
    .unwrap();
    let settings = memory::settings::resolve_principal_memory_settings(
        &db,
        &fixture.actor.principal_id,
        fixture.state.policy.memory.long_term_enabled,
    )
    .unwrap();
    let before: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_vector_snapshots", [], |row| {
            row.get(0)
        })
        .unwrap();
    db.execute_batch("CREATE TABLE parent_marker(value TEXT); BEGIN IMMEDIATE; INSERT INTO parent_marker VALUES('before');").unwrap();
    let (snapshot, _, rows) = memory::embedding_jobs::enqueue_reindex(
        &db,
        &fixture.state.policy.memory,
        &fixture.actor.principal_id,
        &settings.policy_digest,
        true,
    )
    .unwrap();
    assert_eq!(rows, 0);
    assert!(!db.is_autocommit());
    assert_eq!(
        db.query_row(
            "SELECT state FROM memory_vector_snapshots WHERE snapshot_id=?1",
            [snapshot],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "active"
    );
    db.execute_batch("INSERT INTO parent_marker VALUES('after'); ROLLBACK;")
        .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM memory_vector_snapshots", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        before
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM parent_marker", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
