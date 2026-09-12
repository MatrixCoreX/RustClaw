use super::{
    client,
    direct::{market_path, DirectSession},
    history::HistoryRequest,
    nodes::{canonical_origin, Node, Nodes},
    owner_transport::OwnerTransport,
    protocol::*,
};
use crate::wallet::keys;
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[test]
fn node_storage_is_stable_and_rejects_insecure_or_ambiguous_origins() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nodes.json");
    let mut nodes = Nodes::new(path.clone()).unwrap();
    let saved = nodes.add("https://node.example.test/").unwrap();
    assert_eq!(nodes.add("https://node.example.test").unwrap().id, saved.id);
    assert_eq!(
        Nodes::new(path).unwrap().document.nodes.last().unwrap().id,
        saved.id
    );
    for value in [
        "http://192.168.1.2",
        "http://localhost",
        "https://user@node.example.test",
        "https://node.example.test/api",
        "https://node.example.test/?x=1",
        "https://node.example.test/#x",
        "file:///tmp/node",
    ] {
        assert!(canonical_origin(value).is_err(), "{value}");
    }
    assert_eq!(
        canonical_origin("http://127.0.0.1:4444/").unwrap(),
        "http://127.0.0.1:4444"
    );
    assert!(canonical_origin("http://[::1]:4444/").is_ok());
}

#[test]
fn public_paths_cover_chart_ranges_and_never_allow_general_proxy_access() {
    for interval in [60, 300, 900, 3600, 14400, 86400, 604800, 31536000] {
        let path = market_path(&format!(
            "/v1/nni/bancor/candles?interval_seconds={interval}&limit=300&end_time_unix=1800000000"
        ))
        .unwrap();
        assert!(path.starts_with("/v1/nni/server/bancor/candles?"));
        assert!(path.ends_with("price_kind=pool_marginal_usd_per_aic"));
    }
    for path in [
        "//evil.test/v1/nni/bancor/market",
        "/v1/admin/config",
        "/v1/nni/bancor/quote",
        "/v1/nni/bancor/market?node_url=https://evil.test",
        "/v1/nni/bancor/candles?limit=300&limit=300",
        "/v1/nni/bancor/candles?interval_seconds=1",
    ] {
        assert!(market_path(path).is_err(), "{path}");
    }
}

#[tokio::test]
async fn direct_owner_transport_needs_no_device_login_and_pins_origin_ledger_and_context() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let endpoint = origin.clone();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let captured = captured.clone();
            let endpoint = endpoint.clone();
            tokio::spawn(async move {
                let service = service_fn(move |req: Request<Incoming>| {
                    let captured = captured.clone();
                    let endpoint = endpoint.clone();
                    async move {
                        assert!(
                            !req.headers().contains_key("authorization")
                                && !req.headers().contains_key("cookie")
                        );
                        let path = req.uri().path().to_string();
                        let binding = req
                            .headers()
                            .get("x-agent-owner-context")
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_string();
                        assert_eq!(binding.len(), 64);
                        let bytes = req.into_body().collect().await.unwrap().to_bytes();
                        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                        assert!(
                            body.get("password").is_none() && body.get("private_key").is_none()
                        );
                        captured.lock().unwrap().push((path.clone(), binding));
                        let data = if path.ends_with("capabilities") {
                            json!({"schema_version":1,"protocol":PROTOCOL,"ledger_id":"direct-fixture","node_url":endpoint,"service":"assets","actions":["balances","history","operation_status","transfer"]})
                        } else {
                            json!({"account":body["account"],"ledger_id":"direct-fixture","node_url":endpoint,"aic_balance_units":"0","usd_balance_units":"0","page":1,"total_pages":1,"records":[]})
                        };
                        Ok::<_, Infallible>(Response::new(Full::new(Bytes::from(
                            json!({"ok":true,"data":data}).to_string(),
                        ))))
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    let mut session = DirectSession::new(Node {
        id: Uuid::new_v4(),
        origin,
    })
    .unwrap();
    let cap = client::capabilities(&session, Service::Assets, "balances")
        .await
        .unwrap();
    let public = keys::public(&[1; 32]).unwrap();
    let data: ReadResult = client::public_read(&session, &cap, &public, &Intent::Balances)
        .await
        .unwrap();
    data.validate_public(&cap, &public, 1).unwrap();
    let rows = requests.lock().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1, rows[1].1);
    assert_eq!(rows[1].0, "/v1/nni/server/assets/owner/read/public");
    drop(rows);
    session.ledger = Some("different-ledger".into());
    assert!(
        matches!(client::capabilities(&session,Service::Assets,"balances").await, Err(e) if e == "wallet_node_changed")
    );
    session.cancelled.cancel();
    assert!(session
        .owner_request(
            http::Method::GET,
            "/v1/nni/assets/owner/capabilities?service=assets",
            None
        )
        .await
        .is_err());
    server.abort();
}

#[test]
fn full_history_filters_and_exact_amounts_are_validated_before_rendering() {
    let owner = keys::public(&[1; 32]).unwrap();
    let recipient = keys::public(&[2; 32]).unwrap();
    let path = format!("/v1/nni/assets/transfers?owner_pubkey={owner}&limit=100&page=2&source=transfer&direction=outgoing");
    let query = HistoryRequest::parse(&path).unwrap().unwrap();
    assert!(query.path().contains("transaction_class=peer_transfer"));
    let mut data = json!({"schema_version":1,"status":"explorer_transactions","page":2,"per_page":100,"total":101,"total_pages":2,
        "filter":{"transaction_class":"peer_transfer","direction":"outgoing"},"transactions":[{"transaction_id":"fixture-transfer","transaction_kind":"asset_transfer","transaction_class":"peer_transfer","created_at_unix":1800000000,"memo":"memo","flows":[{"flow_index":0,"asset":"USD","amount_units":"100000000","amount":"1.00000000","from":{"account_kind":"asset_owner","address":owner},"to":{"account_kind":"asset_owner","address":recipient}}]}]});
    let result = query.project(&data).unwrap();
    assert_eq!(result["source_filter"], "transfer");
    assert_eq!(result["total_transactions"], 101);
    data["transactions"][0]["flows"][0]["amount"] = json!("2.00000000");
    assert!(query.project(&data).is_err());
    data["transactions"][0]["flows"][0]["amount"] = json!("1.00000000");
    data["filter"]["direction"] = json!("incoming");
    assert!(query.project(&data).is_err());
}
