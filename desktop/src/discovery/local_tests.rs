use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn local_probe_recognizes_only_loopback_contract_without_credentials() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let n = socket.read(&mut request).await.unwrap();
        let body = r#"{"ok":true,"data":{"logged_in":false,"csrf_token":null,"username":null,"role":null}}"#;
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        String::from_utf8_lossy(&request[..n]).to_ascii_lowercase()
    });
    let candidates = probe(
        vec![endpoint, "192.168.1.10:8788".parse().unwrap()],
        CancellationToken::new(),
    )
    .await;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].address, format!("http://{endpoint}"));
    assert_eq!(candidates[0].kind, "local");
    assert!(!candidates[0].verified);
    let request = task.await.unwrap();
    for forbidden in ["cookie:", "authorization:", "x-agent-key:"] {
        assert!(!request.contains(forbidden));
    }
}

#[test]
fn installation_detection_does_not_execute_programs_or_accept_one_generic_binary() {
    let temp = tempfile::tempdir().unwrap();
    assert!(!installation::directory_hint(temp.path()));
    for name in ["clawd", "webd"] {
        let filename = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        };
        let path = temp.path().join(filename);
        std::fs::write(&path, "never execute this fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(installation::directory_hint(temp.path()), name == "webd");
    }
}
