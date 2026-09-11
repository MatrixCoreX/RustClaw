use super::{client, protocol::*};
use crate::{
    credentials::LoginSecret,
    profile::{Connection, Profile},
    session::Session,
};
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::convert::Infallible;
use uuid::Uuid;
use zeroize::Zeroizing;

#[tokio::test]
async fn native_client_zero_balance_path_requires_login_and_does_not_register_an_account() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server_origin = origin.clone();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let origin = server_origin.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let origin = origin.clone();
                    async move {
                        let path = request.uri().path().to_owned();
                        let bytes = request.into_body().collect().await.unwrap().to_bytes();
                        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                        let data = match path.as_str() {
                            "/webd/session" => json!({"logged_in":false}),
                            "/v1/auth/ui-key/verify" | "/v1/auth/me" => json!({"role":"admin"}),
                            "/v1/nni/assets/owner/capabilities" => {
                                json!({"schema_version":1,"protocol":PROTOCOL,
                                "ledger_id":"isolated-fixture","node_url":origin,"service":"assets",
                                "actions":["balances","history","operation_status","transfer"]})
                            }
                            "/v1/nni/assets/owner/operations/request" => {
                                assert_eq!(body["intent"]["kind"], "transfer");
                                let mut terms = body["intent"].clone();
                                terms["fee_units"] = "0".into();
                                json!({"signing_payload":json!({"schema_version":1,"protocol":PROTOCOL,
                                    "ledger_id":"isolated-fixture","node_url":origin,"service":"assets",
                                    "account":body["account"],"operation_id":body["operation_id"],
                                    "challenge_id":Uuid::new_v4(),"nonce":"ab".repeat(32),
                                    "expires_at_unix":client::now()+120,"terms":terms}).to_string()})
                            }
                            "/v1/nni/assets/owner/read/public" => {
                                assert_eq!(body["intent"]["kind"], "balances");
                                assert!(body.get("signature").is_none());
                                assert!(body.get("password").is_none());
                                json!({"account":body["account"],
                                "ledger_id":"isolated-fixture","node_url":origin,"aic_balance_units":"0",
                                "usd_balance_units":"0","page":1,"total_pages":1,"records":[]})
                            }
                            _ => panic!("unexpected request: {path}"),
                        };
                        Ok::<_, Infallible>(
                            Response::builder()
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from(
                                    json!({"ok":true,"data":data}).to_string(),
                                )))
                                .unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    let session = Session::connect(
        Profile {
            id: Uuid::new_v4(),
            alias: "Isolated owner test".into(),
            connection: Connection::Local { origin },
            saved_login: false,
        },
        Zeroizing::new(String::new()),
    )
    .await
    .unwrap();
    assert!(client::capabilities(&session, Service::Assets, "balances")
        .await
        .is_err());
    session
        .login(
            LoginSecret {
                mode: "key".into(),
                username: String::new(),
                secret: "test-key".into(),
            },
            false,
        )
        .await
        .unwrap();
    let cap = client::capabilities(&session, Service::Assets, "balances")
        .await
        .unwrap();
    let owner = crate::wallet::keys::public(&[1; 32]).unwrap();
    let result: ReadResult = client::public_read(&session, &cap, &owner, &Intent::Balances)
        .await
        .unwrap();
    result.validate_public(&cap, &owner, 1).unwrap();
    assert_eq!(result.aic_balance_units, "0");
    assert_eq!(result.total_pages, 1);
    let intent = Intent::Transfer {
        asset: "USD".into(),
        amount_units: "100000000".into(),
        recipient: crate::wallet::keys::public(&[2; 32]).unwrap(),
        memo: String::new(),
        max_fee_bps: 0,
    };
    let (payload, raw) = client::challenge(&session, &cap, &owner, Uuid::new_v4(), &intent)
        .await
        .unwrap();
    assert!(!raw.contains("password"));
    let signature = "a".repeat(128);
    let proof = client::verify_body(&payload, &signature);
    assert_eq!(proof["signature"], signature);
    assert!(proof.get("password").is_none());
    assert!(proof.get("intent").is_none());
    session.close().await;
    server.abort();
    let _ = server.await;
}
