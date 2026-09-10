use super::*;
use mdns_sd::ServiceInfo;
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SESSION: &str =
    r#"{"ok":true,"data":{"logged_in":false,"csrf_token":null,"username":null,"role":null}}"#;

#[test]
fn scan_targets_stay_in_private_attached_subnets_with_hard_limits() {
    let (hosts, limited) = subnet::targets(&[(
        "192.168.5.20".parse().unwrap(),
        "255.255.255.0".parse().unwrap(),
    )]);
    assert_eq!(hosts.len(), 253);
    assert!(!limited);
    for forbidden in [
        "192.168.5.0",
        "192.168.5.20",
        "192.168.5.255",
        "192.168.6.1",
    ] {
        assert!(!hosts.contains(&forbidden.parse().unwrap()));
    }
    let (hosts, limited) = subnet::targets(&[
        ("10.8.2.50".parse().unwrap(), "255.0.0.0".parse().unwrap()),
        (
            "172.20.2.50".parse().unwrap(),
            "255.255.0.0".parse().unwrap(),
        ),
        (
            "192.168.2.50".parse().unwrap(),
            "255.255.255.0".parse().unwrap(),
        ),
        ("8.8.8.8".parse().unwrap(), "255.255.255.0".parse().unwrap()),
    ]);
    assert!(limited && hosts.len() <= 508);
    assert!(hosts.iter().all(Ipv4Addr::is_private));
    assert!(hosts.iter().all(|ip| ip.octets()[2] == 2));
    let (hosts, _) = subnet::targets(&[(
        "192.168.5.1".parse().unwrap(),
        "255.255.255.252".parse().unwrap(),
    )]);
    assert_eq!(hosts, vec!["192.168.5.2".parse::<Ipv4Addr>().unwrap()]);
    assert!(subnet::targets(&[(
        "192.168.5.1".parse().unwrap(),
        "255.0.255.0".parse().unwrap()
    )])
    .0
    .is_empty());
}

fn advertised(host: &str, ip: &str, scheme: &str) -> mdns_sd::ResolvedService {
    let properties = HashMap::from([
        ("scheme".to_owned(), scheme.to_owned()),
        ("api".to_owned(), "webd-v1".to_owned()),
        ("tls".to_owned(), "private_ca".to_owned()),
    ]);
    ServiceInfo::new(SERVICE_TYPE, "Test device", host, ip, 443, properties)
        .unwrap()
        .as_resolved_service()
}

#[test]
fn mdns_hints_never_establish_trust_and_reject_nonlocal_or_wrong_protocol() {
    let info = advertised("device.local.", "192.168.5.2", "https");
    let candidate = mdns::candidate(&info).unwrap();
    assert!(!candidate.verified);
    assert!(candidate.private_ca);
    assert_eq!(candidate.address, "https://device.local:443");
    for info in [
        advertised("device.example.", "192.168.5.2", "https"),
        advertised("device.local.", "8.8.8.8", "https"),
        advertised("device.local.", "192.168.5.2", "http"),
    ] {
        assert!(mdns::candidate(&info).is_none());
    }
    let mut candidates = vec![candidate.clone()];
    let mut scanned = candidate;
    scanned.source = "subnet";
    scanned.address = "https://192.168.5.2".into();
    merge(&mut candidates, scanned);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].source, "mdns");
}

#[test]
fn only_exact_anonymous_session_contract_identifies_a_candidate() {
    assert!(subnet::session_contract(SESSION.as_bytes()));
    for body in [
        r#"{"ok":true}"#,
        r#"{"ok":true,"data":{"logged_in":false}}"#,
        "<html>device login</html>",
        &SESSION.replace("false", "true"),
        &SESSION.replace("\"role\":null", "\"role\":\"admin\""),
    ] {
        assert!(!subnet::session_contract(body.as_bytes()));
    }
}

async fn http_stub(
    status: u16,
    content_type: &str,
    body: String,
) -> (SocketAddr, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nSet-Cookie: discovery-test=must-not-persist\r\nLocation: http://127.0.0.1:9/forbidden\r\nConnection: close\r\n\r\n{body}", body.len());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = vec![0; 4096];
        let n = socket.read(&mut bytes).await.unwrap();
        let _ = socket.write_all(response.as_bytes()).await;
        String::from_utf8_lossy(&bytes[..n]).to_ascii_lowercase()
    });
    (address, task)
}

#[tokio::test]
async fn anonymous_probe_is_bounded_rejects_redirects_html_and_sends_no_credentials() {
    let client = subnet::discovery_client().unwrap();
    for (status, mime, body, expected) in [
        (200, "application/json", SESSION.into(), true),
        (200, "application/json", SESSION.into(), true),
        (302, "application/json", SESSION.into(), false),
        (200, "text/html", SESSION.into(), false),
        (200, "application/json", "x".repeat(4097), false),
    ] {
        let (address, task) = http_stub(status, mime, body).await;
        assert_eq!(subnet::probe_session(&client, address).await, expected);
        let request = task.await.unwrap();
        assert!(request.starts_with("get /webd/session http/1.1\r\n"));
        for forbidden in ["cookie:", "authorization:", "x-agent-key:", "origin:"] {
            assert!(!request.contains(forbidden));
        }
    }
}

#[tokio::test]
async fn cancellation_stops_discovery_promptly() {
    let token = CancellationToken::new();
    token.cancel();
    let report = tokio::time::timeout(Duration::from_secs(1), discover(true, token))
        .await
        .unwrap();
    assert!(report.cancelled);
    assert_eq!(report.scanned_hosts, 0);
    assert!(report.candidates.is_empty());
}

#[tokio::test]
async fn discovery_redirect_never_contacts_its_target() {
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = source.local_addr().unwrap();
    let response = format!("HTTP/1.1 302 Found\r\nLocation: http://{}/forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", target.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = source.accept().await.unwrap();
        let mut request = [0; 2048];
        let _ = socket.read(&mut request).await;
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    assert!(!subnet::probe_session(&subnet::discovery_client().unwrap(), endpoint).await);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), target.accept())
            .await
            .is_err()
    );
    task.await.unwrap();
}
