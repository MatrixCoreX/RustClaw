use std::path::PathBuf;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

async fn response_with_body(body: Vec<u8>, content_length: Option<u64>) -> reqwest::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept fixture request");
        let mut request = [0_u8; 2048];
        let _ = stream.read(&mut request).await;
        let length_header = content_length
            .map(|length| format!("Content-Length: {length}\r\n"))
            .unwrap_or_default();
        let response = format!("HTTP/1.1 200 OK\r\n{length_header}Connection: close\r\n\r\n");
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write fixture headers");
        stream.write_all(&body).await.expect("write fixture body");
    });
    reqwest::get(format!("http://{address}/media"))
        .await
        .expect("request fixture")
}

fn fixture_path(label: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "channel-media-download-{label}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let destination = root.join("inbox/media.bin");
    (root, destination)
}

#[tokio::test]
async fn bounded_response_streams_to_an_atomic_destination() {
    let body = vec![b'a'; 128 * 1024];
    let response = response_with_body(body.clone(), Some(body.len() as u64)).await;
    let (root, destination) = fixture_path("success");

    let written = persist_bounded_response(response, &destination, 256 * 1024)
        .await
        .expect("persist response");

    assert_eq!(written, body.len() as u64);
    assert_eq!(
        tokio::fs::read(&destination).await.expect("read output"),
        body
    );
    assert_eq!(
        std::fs::read_dir(destination.parent().expect("output parent"))
            .expect("list output parent")
            .count(),
        1
    );
    std::fs::remove_dir_all(root).expect("remove fixture");
}

#[tokio::test]
async fn bounded_response_rejects_declared_oversize_before_creating_a_file() {
    let response = response_with_body(Vec::new(), Some(4096)).await;
    let (root, destination) = fixture_path("declared-limit");

    let error = persist_bounded_response(response, &destination, 1024)
        .await
        .expect_err("reject oversized response");

    assert!(matches!(
        error,
        ChannelMediaDownloadError::TooLarge {
            actual_bytes: 4096,
            max_bytes: 1024
        }
    ));
    assert!(!destination.exists());
    assert!(!root.exists());
}

#[tokio::test]
async fn bounded_response_removes_partial_file_when_stream_exceeds_limit() {
    let response = response_with_body(vec![b'x'; 4096], None).await;
    let (root, destination) = fixture_path("stream-limit");

    let error = persist_bounded_response(response, &destination, 1024)
        .await
        .expect_err("reject oversized stream");

    assert!(matches!(
        error,
        ChannelMediaDownloadError::TooLarge {
            max_bytes: 1024,
            ..
        }
    ));
    assert!(!destination.exists());
    if root.exists() {
        let parent = destination.parent().expect("output parent");
        assert_eq!(
            std::fs::read_dir(parent)
                .expect("list output parent")
                .count(),
            0
        );
        std::fs::remove_dir_all(root).expect("remove fixture");
    }
}
