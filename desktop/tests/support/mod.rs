use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Request, Response};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::net::TcpListener;

pub type MockBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;
#[derive(Clone, Default)]
pub struct Evidence {
    pub requests: Arc<AtomicUsize>,
    pub secrets: Arc<AtomicUsize>,
}
pub async fn handler(
    request: Request<Incoming>,
    evidence: Evidence,
) -> std::result::Result<Response<MockBody>, Infallible> {
    evidence.requests.fetch_add(1, Ordering::SeqCst);
    let path = request.uri().path().to_owned();
    let headers = request.headers().clone();
    let key = headers
        .get("x-agent-key")
        .is_some_and(|v| v == "test-user-key");
    let cookie = headers
        .get("cookie")
        .is_some_and(|v| v.to_str().unwrap_or("").contains("session=test-session"));
    if key || cookie {
        evidence.secrets.fetch_add(1, Ordering::SeqCst);
    }
    let full = |text: String| Full::new(Bytes::from(text)).boxed();
    let json = |value: serde_json::Value| {
        Response::builder()
            .header("content-type", "application/json")
            .body(full(value.to_string()))
            .unwrap()
    };
    let body = request.into_body().collect().await.unwrap().to_bytes();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    Ok(match path.as_str() {
        "/webd/session" => json(serde_json::json!({"ok":true,"data":{"logged_in":cookie}})),
        "/webd/login"
            if parsed["username"] == "tester" && parsed["password"] == "test-password" =>
        {
            let mut response =
                json(serde_json::json!({"ok":true,"data":{"csrf_token":"a".repeat(32)}}));
            response.headers_mut().insert(
                "set-cookie",
                "session=test-session; Path=/; HttpOnly; SameSite=Lax"
                    .parse()
                    .unwrap(),
            );
            response
        }
        "/v1/auth/ui-key/verify" if parsed["user_key"] == "test-user-key" => {
            json(serde_json::json!({"ok":true,"data":{"user_id":1,"chat_id":1,"role":"admin"}}))
        }
        "/v1/auth/me" if key || cookie => json(
            serde_json::json!({"ok":true,"data":{"user_id":1,"chat_id":1,"role":"admin","user_key":"test-user-key"}}),
        ),
        "/v1/write"
            if key
                || (cookie
                    && headers
                        .get("x-agent-csrf-token")
                        .is_some_and(|v| v.to_str().unwrap() == "a".repeat(32))) =>
        {
            json(
                serde_json::json!({"ok":true,"data":{"bytes":body.len(),"origin":headers["origin"].to_str().unwrap()}}),
            )
        }
        "/v1/redirect" => Response::builder()
            .status(302)
            .header("location", "http://127.0.0.1:9/stolen")
            .body(full(String::new()))
            .unwrap(),
        "/v1/download" if key || cookie => {
            let (tx, rx) = tokio::sync::mpsc::channel::<
                std::result::Result<hyper::body::Frame<Bytes>, Infallible>,
            >(1);
            tokio::spawn(async move {
                for _ in 0..64 {
                    if tx
                        .send(Ok(hyper::body::Frame::data(Bytes::from(vec![73; 65536]))))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            });
            Response::builder()
                .header("content-type", "application/octet-stream")
                .header("content-length", 4 * 1024 * 1024)
                .body(
                    http_body_util::StreamBody::new(tokio_stream::wrappers::ReceiverStream::new(
                        rx,
                    ))
                    .boxed(),
                )
                .unwrap()
        }
        "/v1/events" => {
            let (tx, rx) = tokio::sync::mpsc::channel::<
                std::result::Result<hyper::body::Frame<Bytes>, Infallible>,
            >(1);
            tokio::spawn(async move {
                let _ = tx
                    .send(Ok(hyper::body::Frame::data(Bytes::from_static(
                        b"data: first\n\n",
                    ))))
                    .await;
                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                let _ = tx
                    .send(Ok(hyper::body::Frame::data(Bytes::from_static(
                        b"data: second\n\n",
                    ))))
                    .await;
            });
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(
                    http_body_util::StreamBody::new(tokio_stream::wrappers::ReceiverStream::new(
                        rx,
                    ))
                    .boxed(),
                )
                .unwrap()
        }
        _ => Response::builder()
            .status(401)
            .body(full("{\"ok\":false}".into()))
            .unwrap(),
    })
}
pub async fn http_server(evidence: Evidence) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let evidence = evidence.clone();
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(socket),
                        service_fn(move |r| handler(r, evidence.clone())),
                    )
                    .await;
            });
        }
    });
    (port, task)
}
