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
    None,
    InTransaction,
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

pub(super) fn scenario(action: fixtures::Action, change: Change) {
    let mut fixture = fixtures::Fixture::new();
    if matches!(change, Change::None) {
        {
            let db = fixture.state.core.db.get().unwrap();
            assert_eq!(
                db.execute(
                    "UPDATE auth_keys SET role='user' WHERE user_key=?1",
                    [&fixture.actor.user_key]
                )
                .unwrap(),
                1
            );
        }
        fixture.actor.role = "user".into();
    }
    let (path, body) = fixture.request(action);
    if matches!(change, Change::InTransaction) {
        let db = fixture.state.core.db.get().unwrap();
        db.execute_batch(&format!(
            "CREATE TRIGGER revoke_actor_in_transaction AFTER INSERT ON memory_revisions
            WHEN NEW.operation='correct' AND NEW.principal_id='{}'
            BEGIN UPDATE auth_keys SET enabled=0 WHERE user_key='{}'; END;",
            fixture.actor.principal_id, fixture.actor.user_key
        ))
        .unwrap();
    }
    let before = fixtures::private_state(&fixture.state.core.db.get().unwrap());
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
        // Deadlines protect only the test's handshake, never production work.
        tokio::time::timeout(std::time::Duration::from_secs(30),ready).await.unwrap().unwrap();
        let (queued,observed)=oneshot::channel();
        let subscriber=tracing_subscriber::registry().with(QueueLayer(Arc::new(Mutex::new(Some(queued)))));
        let dispatch=tracing::Dispatch::new(subscriber);
        let request=axum::http::Request::builder().method("POST").uri(path)
            .header(claw_core::product_identity::AUTH_KEY_HEADER,&fixture.actor.user_key)
            .header("content-type","application/json").body(axum::body::Body::from(body.to_string())).unwrap();
        let app=super::super::router().with_state(fixture.state.clone());
        let mut future=Box::pin(app.oneshot(request));
        let future=std::future::poll_fn(move |context|tracing::dispatcher::with_default(&dispatch,||future.as_mut().poll(context)));
        let request=tokio::spawn(future);
        tokio::time::timeout(std::time::Duration::from_secs(30),observed).await
            .expect("actual HTTP handler must pass authentication and reach its queued writer").unwrap();
        {
            let db=fixture.state.core.db.get().unwrap();
            use rusqlite::params;
            match change {
                Change::Key => {assert_eq!(db.execute("UPDATE auth_keys SET enabled=0 WHERE user_key=?1",[&fixture.actor.user_key]).unwrap(),1);}
                Change::Binding => {assert_eq!(db.execute("UPDATE credential_bindings SET status='revoked' WHERE credential_digest=?1",[crate::repo::auth::credential_digest(&fixture.actor.user_key)]).unwrap(),1);}
                Change::Role => {assert_eq!(db.execute("UPDATE auth_keys SET role='user' WHERE user_key=?1",[&fixture.actor.user_key]).unwrap(),1);}
                Change::Reassign => {
                    assert_eq!(db.execute("UPDATE auth_keys SET principal_id=?1 WHERE user_key=?2",params![fixture.other,fixture.actor.user_key]).unwrap(),1);
                    assert_eq!(db.execute("UPDATE credential_bindings SET principal_id=?1 WHERE credential_digest=?2",params![fixture.other,crate::repo::auth::credential_digest(&fixture.actor.user_key)]).unwrap(),1);
                }
                Change::None | Change::InTransaction => {}
            }
            assert!(crate::repo::auth::require_active_principal_credential(&db,&fixture.actor.principal_id).is_ok(),"the target still has another live credential");
        }
        gate.release();
        let response=request.await.unwrap().unwrap();
        let status=response.status();
        let bytes=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
        let reply:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
        blocker.await.unwrap();
        if matches!(change,Change::None) {
            assert_eq!(status,StatusCode::OK,"{reply}");
            assert_eq!(reply["ok"],true);
            assert_eq!(reply["data"]["status"],"corrected");
            assert_ne!(fixtures::private_state(&fixture.state.core.db.get().unwrap()),before);
        } else {
            let (expected,code)=if matches!(change,Change::Role|Change::Reassign) {
                (StatusCode::FORBIDDEN,"request_identity_changed")
            } else {(StatusCode::UNAUTHORIZED,"request_credential_revoked")};
            assert_eq!(status,expected,"{reply}");
            assert_eq!(reply["ok"],false);
            assert_eq!(reply["error"],code);
            assert!(reply["data"].is_null());
            assert_eq!(fixtures::private_state(&fixture.state.core.db.get().unwrap()),before);
            let text=reply.to_string();
            for secret in [&fixture.actor.user_key,"actor-private-text","other-private-text","recent-private-text"] {
                assert!(!text.contains(secret));
            }
            if matches!(change,Change::InTransaction) {
                let db=fixture.state.core.db.get().unwrap();
                assert_eq!(db.query_row("SELECT enabled FROM auth_keys WHERE user_key=?1",[&fixture.actor.user_key],|row|row.get::<_,i64>(0)).unwrap(),1);
            }
        }
    });
}
