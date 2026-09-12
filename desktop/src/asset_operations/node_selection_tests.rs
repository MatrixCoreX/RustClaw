use super::*;
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct Fixture {
    node: Node,
    requests: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new(delay_ms: u64, ledger: &'static str, status: u16) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let node = Node {
            id: Uuid::new_v4(),
            origin: format!("http://{}", listener.local_addr().unwrap()),
        };
        let origin = node.origin.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let captured = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let origin = origin.clone();
                let captured = captured.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request: Request<Incoming>| {
                        let origin = origin.clone();
                        let captured = captured.clone();
                        async move {
                            assert_eq!(request.method(), http::Method::GET);
                            assert_eq!(
                                request.uri().path(),
                                "/v1/nni/server/assets/owner/capabilities"
                            );
                            assert!(!request.headers().contains_key("authorization"));
                            assert!(!request.headers().contains_key("cookie"));
                            let url =
                                reqwest::Url::parse(&format!("{origin}{}", request.uri())).unwrap();
                            let service = url
                                .query_pairs()
                                .find(|(k, _)| k == "service")
                                .unwrap()
                                .1
                                .into_owned();
                            captured.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            let ledger = if ledger == "inconsistent" && service == "bancor" {
                                "different"
                            } else {
                                ledger
                            };
                            let body = json!({"ok":true,"data":{"schema_version":1,"protocol":"asset_owner_v1",
                                "ledger_id":ledger,"node_url":origin,"service":service,"actions":["balances"]}});
                            Ok::<_, Infallible>(
                                Response::builder()
                                    .status(status)
                                    .header("location", "http://127.0.0.1:1")
                                    .body(Full::new(Bytes::from(body.to_string())))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
        });
        Self {
            node,
            requests,
            server,
        }
    }
}

#[tokio::test]
async fn prefers_fast_valid_node_and_rejects_faster_redirect_and_wrong_ledger() {
    let slow = Fixture::new(300, "ledger-a", 200).await;
    let fast = Fixture::new(40, "ledger-a", 200).await;
    let wrong = Fixture::new(1, "ledger-b", 200).await;
    let redirect = Fixture::new(1, "ledger-a", 302).await;
    let chosen = prefer(
        vec![
            slow.node.clone(),
            wrong.node.clone(),
            redirect.node.clone(),
            fast.node.clone(),
        ],
        slow.node.id,
        Some("ledger-a"),
    )
    .await
    .unwrap();
    assert_eq!(chosen.session.node.id, fast.node.id);
    assert!(chosen.response_ms >= 40);
    assert_eq!(fast.requests.load(Ordering::SeqCst), 2);
    // A manual connection still chooses the explicit slower node.
    assert_eq!(
        check(slow.node.clone()).await.unwrap().session.node.id,
        slow.node.id
    );
}

#[tokio::test]
async fn anchors_legacy_selection_and_refuses_ambiguous_ledgers_without_an_anchor() {
    let anchor = Fixture::new(60, "ledger-a", 200).await;
    let other = Fixture::new(1, "ledger-b", 200).await;
    let nodes = vec![anchor.node.clone(), other.node.clone()];
    assert_eq!(
        prefer(nodes.clone(), anchor.node.id, None)
            .await
            .unwrap()
            .session
            .node
            .id,
        anchor.node.id
    );
    assert!(
        matches!(prefer(nodes, Uuid::new_v4(), None).await, Err(e) if e == "wallet_node_ledger_ambiguous")
    );
    assert!(
        matches!(prefer(vec![other.node.clone()], anchor.node.id, Some("ledger-a")).await, Err(e) if e == "wallet_node_no_healthy")
    );
}

#[tokio::test]
async fn excludes_inconsistent_services_and_bounds_unresponsive_nodes() {
    let hanging = Fixture::new(8000, "ledger-a", 200).await;
    let inconsistent = Fixture::new(1, "inconsistent", 200).await;
    let fast = Fixture::new(20, "ledger-a", 200).await;
    let started = Instant::now();
    let chosen = prefer(
        vec![
            hanging.node.clone(),
            inconsistent.node.clone(),
            fast.node.clone(),
        ],
        hanging.node.id,
        Some("ledger-a"),
    )
    .await
    .unwrap();
    assert_eq!(chosen.session.node.id, fast.node.id);
    assert!(started.elapsed() < Duration::from_secs(6));
    assert!(
        matches!(prefer(vec![inconsistent.node.clone()], inconsistent.node.id, None).await, Err(e) if e == "wallet_node_no_healthy")
    );
}

#[test]
fn upgrades_legacy_node_document_without_losing_saved_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nodes.json");
    let mut nodes = super::super::nodes::Nodes::new(path.clone()).unwrap();
    let saved = nodes.document.selected;
    assert!(nodes.document.ledger_id.is_none());
    nodes.document.ledger_id = Some("ledger-a".into());
    nodes.persist().unwrap();
    let reloaded = super::super::nodes::Nodes::new(path).unwrap();
    assert_eq!(reloaded.document.selected, saved);
    assert_eq!(reloaded.document.ledger_id.as_deref(), Some("ledger-a"));
}
