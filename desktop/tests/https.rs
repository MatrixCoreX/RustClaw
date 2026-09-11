mod support;
use agent_desktop::{
    credentials::LoginSecret,
    profile::{Connection, Profile},
    session::Session,
    transfers::{RequestSpec, Transfers},
    transport::{bytes_body, small_json, Transport},
};
use http::{HeaderMap, Method};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::TcpListener;
use uuid::Uuid;
use zeroize::Zeroizing;

async fn tls_server() -> (Connection, support::Evidence, tokio::task::JoinHandle<()>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let pem = cert.cert.pem();
    let digest = hex::encode(Sha256::digest(cert.cert.der()));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into()),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let evidence = support::Evidence::default();
    let observed = evidence.clone();
    let task = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            let evidence = evidence.clone();
            tokio::spawn(async move {
                if let Ok(socket) = acceptor.accept(socket).await {
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(
                            hyper_util::rt::TokioIo::new(socket),
                            hyper::service::service_fn(move |r| {
                                support::handler(r, evidence.clone())
                            }),
                        )
                        .await;
                }
            });
        }
    });
    (
        Connection::Https {
            origin: format!("https://localhost:{port}"),
            ca_pem: Some(pem),
            ca_sha256: Some(digest),
        },
        observed,
        task,
    )
}

#[tokio::test]
async fn https_profile_never_downgrades_to_a_plain_http_listener() {
    let (connection, _, tls) = tls_server().await;
    let evidence = support::Evidence::default();
    let (port, plain) = support::http_server(evidence.clone()).await;
    let mut target = connection;
    if let Connection::Https { origin, .. } = &mut target {
        *origin = format!("https://localhost:{port}");
    }
    assert!(Transport::connect(&target, "").await.is_err());
    assert_eq!(
        evidence.requests.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        evidence.secrets.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    plain.abort();
    tls.abort();
}

#[tokio::test]
async fn tls_pin_hostname_redirect_and_device_cookie_isolation() {
    let (connection, evidence, server) = tls_server().await;
    let mut wrong = connection.clone();
    if let Connection::Https { origin, .. } = &mut wrong {
        *origin = origin.replace("localhost", "127.0.0.1");
    }
    assert!(Transport::connect(&wrong, "").await.is_err());
    let mut bad_pin = connection.clone();
    if let Connection::Https { ca_sha256, .. } = &mut bad_pin {
        *ca_sha256 = Some("0".repeat(64));
    }
    assert!(Transport::connect(&bad_pin, "").await.is_err());
    assert_eq!(
        evidence.secrets.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let profile = Profile {
        saved_login: false,
        id: Uuid::new_v4(),
        alias: "Test".into(),
        connection: connection.clone(),
    };
    let session = Session::connect(profile, Zeroizing::new(String::new()))
        .await
        .unwrap();
    let login = session
        .login(
            LoginSecret {
                mode: "password".into(),
                username: "tester".into(),
                secret: "test-password".into(),
            },
            false,
        )
        .await
        .unwrap();
    assert!(login.session.identity.unwrap().get("user_key").is_none());
    let response = session
        .request(
            Method::POST,
            "/v1/write",
            HeaderMap::new(),
            Some(bytes_body("payload")),
        )
        .await
        .unwrap();
    let (status, body) = small_json(response).await.unwrap();
    assert_eq!(status, 200);
    assert_eq!(body["data"]["bytes"], 7);
    assert!(session
        .request(Method::GET, "/v1/redirect", HeaderMap::new(), None)
        .await
        .is_err());
    let other = Transport::connect(&connection, "").await.unwrap();
    let (status, _) = small_json(
        other
            .send(Method::GET, "/v1/auth/me", HeaderMap::new(), None)
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(status, 401);
    session.close().await;
    assert!(session
        .request(Method::GET, "/v1/auth/me", HeaderMap::new(), None)
        .await
        .is_err());
    server.abort();
}

#[tokio::test]
async fn native_transfer_streams_sse_and_upload_with_session_binding() {
    let (connection, _, server) = tls_server().await;
    let session = Session::connect(
        Profile {
            saved_login: false,
            id: Uuid::new_v4(),
            alias: "Stream".into(),
            connection,
        },
        Zeroizing::new(String::new()),
    )
    .await
    .unwrap();
    session
        .login(
            LoginSecret {
                mode: "key".into(),
                username: String::new(),
                secret: "test-user-key".into(),
            },
            false,
        )
        .await
        .unwrap();
    let transfers = Transfers::default();
    let now = Instant::now();
    let id = transfers
        .start(
            session.clone(),
            RequestSpec {
                path: "/v1/events".into(),
                method: "GET".into(),
                headers: HashMap::new(),
                has_body: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(transfers.headers(session.id, id).await.unwrap().status, 200);
    assert_eq!(
        transfers.read(session.id, id).await.unwrap(),
        "data: first\n\n"
    );
    assert!(now.elapsed() < Duration::from_millis(500));
    assert!(transfers.read(Uuid::new_v4(), id).await.is_err());
    assert_eq!(
        transfers.read(session.id, id).await.unwrap(),
        "data: second\n\n"
    );
    assert!(transfers.read(session.id, id).await.unwrap().is_empty());
    let id = transfers
        .start(
            session.clone(),
            RequestSpec {
                path: "/v1/write".into(),
                method: "POST".into(),
                headers: HashMap::new(),
                has_body: true,
            },
        )
        .await
        .unwrap();
    for _ in 0..64 {
        transfers
            .upload(session.id, id, bytes::Bytes::from(vec![7; 65536]))
            .await
            .unwrap();
    }
    transfers.finish_upload(session.id, id).await.unwrap();
    assert_eq!(transfers.headers(session.id, id).await.unwrap().status, 200);
    let body: serde_json::Value =
        serde_json::from_slice(&transfers.read(session.id, id).await.unwrap()).unwrap();
    assert_eq!(body["data"]["bytes"], 4 * 1024 * 1024);
    transfers.cancel_session(session.id).await;
    session.close().await;
    server.abort();
}

#[tokio::test]
async fn download_stream_is_atomic_and_cancellation_preserves_existing_file() {
    let (connection, _, server) = tls_server().await;
    let session = Session::connect(
        Profile {
            saved_login: false,
            id: Uuid::new_v4(),
            alias: "Download test".into(),
            connection,
        },
        Zeroizing::new(String::new()),
    )
    .await
    .unwrap();
    session
        .login(
            LoginSecret {
                mode: "key".into(),
                username: String::new(),
                secret: "test-user-key".into(),
            },
            false,
        )
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("附件.bin");
    std::fs::write(&target, b"original").unwrap();
    let cancel = session.cancelled.child_token();
    let mut completed = false;
    agent_desktop::download_stream::save_response(
        &session,
        "/v1/download",
        &target,
        &cancel,
        |written, total, finished| {
            assert_eq!(total, Some(4 * 1024 * 1024));
            if !finished {
                assert_eq!(std::fs::read(&target).unwrap(), b"original");
            } else {
                assert_eq!(written, 4 * 1024 * 1024);
                completed = true;
            }
        },
    )
    .await
    .unwrap();
    assert!(completed);
    assert_eq!(
        Sha256::digest(std::fs::read(&target).unwrap()),
        Sha256::digest(vec![73; 4 * 1024 * 1024])
    );
    std::fs::write(&target, b"keep this").unwrap();
    let error = agent_desktop::download_stream::save_response(
        &session,
        "/v1/download",
        &target,
        &cancel,
        |_, _, finished| {
            assert!(!finished);
            cancel.cancel();
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error, "download_cancelled");
    assert_eq!(std::fs::read(&target).unwrap(), b"keep this");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    session.close().await;
    server.abort();
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "Run explicitly with an unavailable isolated DBUS_SESSION_BUS_ADDRESS"]
async fn unavailable_system_vault_keeps_login_in_memory_without_plaintext_fallback() {
    assert!(std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .unwrap()
        .contains("agent-desktop-missing-secret-service"));
    let (connection, _, server) = tls_server().await;
    let session = Session::connect(
        Profile {
            saved_login: false,
            id: Uuid::new_v4(),
            alias: "Vault unavailable".into(),
            connection,
        },
        Zeroizing::new(String::new()),
    )
    .await
    .unwrap();
    let login = session
        .login(
            LoginSecret {
                mode: "key".into(),
                username: String::new(),
                secret: "test-user-key".into(),
            },
            true,
        )
        .await
        .unwrap();
    assert!(!login.remembered);
    assert!(login.warning.is_some());
    assert_eq!(login.session.identity.unwrap()["role"], "admin");
    assert!(agent_desktop::credentials::load(session.profile.id).is_err());
    session.close().await;
    server.abort();
}
