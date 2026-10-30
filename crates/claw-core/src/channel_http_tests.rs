use super::core_api_client_builder;
use crate::channel_delivery::ChannelDeliverySource;
use crate::channel_delivery_client::{request_task_delivery, ChannelDeliveryClientError};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const FIXTURE_KEY: &str = "fixture-channel-credential";

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(status: u16, cross_origin: Option<&str>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let destination = format!("{}/destination", cross_origin.unwrap_or(&base));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let mut buffer = [0_u8; 2048];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(count, 0, "incomplete fixture request");
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                    assert!(request.len() < 16384, "oversized fixture request");
                }
            })
            .await
            .unwrap();
            let redirected = request.starts_with(b"GET /destination ")
                || request.starts_with(b"POST /destination ");
            captured.lock().unwrap().push(request);
            let body = json!({"ok": true, "error": null, "data": {
                "schema_version": 1, "status": "accepted", "accepted": true,
                "delivered": false, "receipt": null, "error_code": null,
                "message_key": null, "retryable": false
            }})
            .to_string();
            let response = if status != 200 && !redirected {
                format!("HTTP/1.1 {status} Redirect\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            } else {
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
            };
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    Server {
        base,
        requests,
        task,
    }
}

async fn assert_core_redirects_rejected(delivery: bool) {
    let client = core_api_client_builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let mut failures = Vec::new();
    for status in [301, 302, 303, 307, 308] {
        for cross_origin in [false, true] {
            let target = server(200, None).await;
            let origin = server(status, cross_origin.then_some(target.base.as_str())).await;
            let actual_status = if delivery {
                match request_task_delivery(&client, &origin.base, "fixture-task", FIXTURE_KEY,
                        ChannelDeliverySource::ImmediateDaemon).await {
                        Err(ChannelDeliveryClientError::HttpStatus(code)) => code,
                        Ok(_) => 200,
                        other => panic!("redirected delivery status={status} cross_origin={cross_origin}: {other:?}"),
                    }
            } else {
                client
                    .get(format!("{}/v1/tasks/fixture-task", origin.base))
                    .header(crate::product_identity::AUTH_KEY_HEADER, FIXTURE_KEY)
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .as_u16()
            };
            let origin_count = origin.requests.lock().unwrap().len();
            let target_count = target.requests.lock().unwrap().len();
            let forwarded_key = origin
                .requests
                .lock()
                .unwrap()
                .iter()
                .skip(1)
                .chain(target.requests.lock().unwrap().iter())
                .any(|request| {
                    String::from_utf8_lossy(request)
                        .to_ascii_lowercase()
                        .contains(&format!("x-agent-key: {FIXTURE_KEY}\r\n"))
                });
            if actual_status != status || origin_count != 1 || target_count != 0 {
                failures.push(format!("status={status} cross_origin={cross_origin} delivery={delivery} actual={actual_status} origin_requests={origin_count} target_requests={target_count} forwarded_key={forwarded_key}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn channel_core_get_rejects_all_redirects() {
    assert_core_redirects_rejected(false).await;
}

#[tokio::test]
async fn channel_core_delivery_rejects_all_redirects() {
    assert_core_redirects_rejected(true).await;
}

#[tokio::test]
async fn direct_channel_delivery_preserves_key_body_and_response() {
    let origin = server(200, None).await;
    let client = core_api_client_builder().no_proxy().build().unwrap();
    let response = request_task_delivery(
        &client,
        &origin.base,
        "fixture-task",
        FIXTURE_KEY,
        ChannelDeliverySource::ImmediateDaemon,
    )
    .await
    .unwrap();
    assert!(response.accepted);
    let requests = origin.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let raw = String::from_utf8(requests[0].clone()).unwrap();
    assert!(raw.starts_with("POST /v1/tasks/fixture-task/delivery HTTP/1.1\r\n"));
    assert!(raw
        .to_ascii_lowercase()
        .contains(&format!("x-agent-key: {FIXTURE_KEY}\r\n")));
    let body: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["source"], "immediate_daemon");
}

#[tokio::test]
async fn ordinary_media_client_can_follow_redirect_without_core_credentials() {
    let target = server(200, None).await;
    let origin = server(302, Some(&target.base)).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(&origin.base)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let requests = target.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!String::from_utf8_lossy(&requests[0])
        .to_ascii_lowercase()
        .contains("x-agent-key:"));
}
