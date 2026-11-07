use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

use crate::memory::capacity_test_fixtures::{pressure, scalar, Fixture};

struct Credential(String);

impl Credential {
    fn new() -> Self {
        let key = format!("MEMORY_CAPACITY_FIXTURE_{}", uuid::Uuid::new_v4().simple());
        std::env::set_var(&key, "synthetic-test-token");
        Self(key)
    }
}

impl Drop for Credential {
    fn drop(&mut self) {
        std::env::remove_var(&self.0);
    }
}

struct LoopbackEmbedding {
    endpoint: String,
    calls: Arc<AtomicUsize>,
    arrived: Option<oneshot::Receiver<()>>,
    release: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl LoopbackEmbedding {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/embeddings", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (arrived_tx, arrived_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0_u8; 1024];
                let count = socket.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0, "fixture request must have a complete body");
                request.extend_from_slice(&buffer[..count]);
                if let Some(position) = request.windows(4).position(|item| item == b"\r\n\r\n") {
                    let header_end = position + 4;
                    let headers = std::str::from_utf8(&request[..position]).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .expect("fixture content length");
                    if request.len() >= header_end + length {
                        let body: serde_json::Value =
                            serde_json::from_slice(&request[header_end..header_end + length])
                                .unwrap();
                        assert_eq!(body["model"], "capacity-http-fixture");
                        assert_eq!(body["input"].as_array().unwrap().len(), 1);
                        break;
                    }
                }
            }
            observed.fetch_add(1, Ordering::SeqCst);
            let _ = arrived_tx.send(());
            let _ = release_rx.await;
            let body = r#"{"data":[{"index":0,"embedding":[1.0,0.0,0.0]}]}"#;
            let response = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        Self {
            endpoint,
            calls,
            arrived: Some(arrived_rx),
            release: Some(release_tx),
            task,
        }
    }

    fn release(&mut self) {
        if let Some(sender) = self.release.take() {
            let _ = sender.send(());
        }
    }

    async fn wait_for_request(&mut self) {
        // This deadline protects only the test handshake, not provider work.
        tokio::time::timeout(Duration::from_secs(30), self.arrived.take().unwrap())
            .await
            .expect("fixture provider request must arrive")
            .unwrap();
    }
}

impl Drop for LoopbackEmbedding {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn remote_fixture(endpoint: &str) -> (Fixture, Credential) {
    let mut fixture = Fixture::memory();
    fixture.seed_embedding();
    assert!(
        super::run_one_embedding_batch(fixture.state(), "initial-local-worker")
            .await
            .unwrap()
    );
    let credential = Credential::new();
    {
        let config = &mut fixture.state_mut().policy.memory;
        config.embedding_provider_kind = "remote_http".to_string();
        config.embedding_endpoint_ref = endpoint.to_string();
        config.embedding_credential_ref = credential.0.clone();
        config.embedding_model = "capacity-http-fixture".to_string();
        config.embedding_version = "capacity-http-v1".to_string();
        config.embedding_dims = 3;
    }
    {
        let db = fixture.state().core.db.get().unwrap();
        let actor = crate::resolve_auth_identity_by_key(fixture.state(), &fixture.key)
            .unwrap()
            .unwrap();
        let settings = crate::memory::settings::update_memory_settings(
            &db,
            &actor,
            &crate::memory::settings::MemorySettingsUpdateRequest {
                scope: crate::memory::settings::MemorySettingScope::Principal,
                target_principal_id: None,
                conversation_id: None,
                use_mode: None,
                generate_mode: None,
                external_context_policy: Some(
                    crate::memory::settings::ExternalContextPolicy::Allow,
                ),
                expected_revision: Some(0),
                long_term_enabled: None,
            },
            fixture.state().policy.memory.long_term_enabled,
        )
        .unwrap();
        let (_, _, count) = super::enqueue_reindex(
            &db,
            &fixture.state().policy.memory,
            &fixture.principal,
            &settings.policy_digest,
            true,
        )
        .unwrap();
        assert_eq!(count, 1);
    }
    (fixture, credential)
}

#[tokio::test]
async fn paused_remote_embedding_never_reaches_the_loopback_provider() {
    let mut server = LoopbackEmbedding::start().await;
    let (fixture, _credential) = remote_fixture(&server.endpoint).await;
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "explicit_write_blocked");
    }
    server.release();
    let ran = super::run_one_embedding_batch(fixture.state(), "remote-worker")
        .await
        .unwrap();
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        (
            ran,
            server.calls.load(Ordering::SeqCst),
            scalar(
                &db,
                "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status = 'queued'"
            ),
            scalar(&db, "SELECT COUNT(*) FROM memory_vector_rows")
        ),
        (false, 0, 1, 1)
    );
}

#[tokio::test]
async fn pressure_change_does_not_kill_already_inflight_provider_work() {
    let mut server = LoopbackEmbedding::start().await;
    let (fixture, _credential) = remote_fixture(&server.endpoint).await;
    let state = fixture.state().clone();
    let worker =
        tokio::spawn(
            async move { super::run_one_embedding_batch(&state, "inflight-worker").await },
        );
    server.wait_for_request().await;
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "explicit_write_blocked");
    }
    server.release();
    assert!(worker.await.unwrap().unwrap());
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status = 'completed'"
        ),
        2
    );
    assert_eq!(scalar(&db, "SELECT COUNT(*) FROM memory_vector_rows"), 2);
    let state: String = db
        .query_row("SELECT state FROM memory_storage_pressure", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(state, "explicit_write_blocked");
}

#[path = "embedding_recall_race_tests.rs"]
mod recall_races;

#[path = "embedding_commit_tests.rs"]
mod response_commit;
