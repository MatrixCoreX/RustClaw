mod support;
use agent_desktop::{
    credentials::LoginSecret,
    profile::{loopback_origin, Connection, Profile},
    session::Session,
    transport::{bytes_body, small_json, Transport},
};
use http::{HeaderMap, Method};
use uuid::Uuid;
use zeroize::Zeroizing;

#[test]
fn local_http_is_literal_loopback_only_and_cannot_admit_lan_or_hostname_targets() {
    for origin in [
        "http://127.0.0.1:8788",
        "http://[::1]:8788",
        "http://127.0.0.1",
    ] {
        assert!(loopback_origin(origin).is_ok(), "{origin}");
    }
    for origin in [
        "http://192.168.1.2:8788",
        "http://localhost:8788",
        "http://device.local:8788",
        "http://0.0.0.0:8788",
        "http://[::]:8788",
        "http://[::ffff:192.168.1.2]",
        "https://127.0.0.1",
        "http://user@127.0.0.1",
        "http://127.0.0.1:0",
        "http://127.0.0.1/admin",
        "http://127.0.0.1/?redirect=evil",
        "http://127.0.0.1/#evil",
        "http://127.0.0.1\\@evil.test",
        "http://127.0.0.1.evil.test",
    ] {
        assert!(loopback_origin(origin).is_err(), "{origin}");
    }
}

#[tokio::test]
async fn local_http_requires_login_retains_csrf_and_never_follows_redirects() {
    let (port, server) = support::http_server(support::Evidence::default()).await;
    let connection = Connection::Local {
        origin: format!("http://127.0.0.1:{port}"),
    };
    let session = Session::connect(
        Profile {
            id: Uuid::new_v4(),
            alias: "Local test".into(),
            connection: connection.clone(),
            saved_login: false,
        },
        Zeroizing::new(String::new()),
    )
    .await
    .unwrap();
    assert!(session
        .request(Method::GET, "/v1/auth/me", HeaderMap::new(), None)
        .await
        .is_err());
    session
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
    let (status, body) = small_json(
        session
            .request(
                Method::POST,
                "/v1/write",
                HeaderMap::new(),
                Some(bytes_body("local")),
            )
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(status, 200);
    assert_eq!(body["data"]["bytes"], 5);
    assert_eq!(body["data"]["origin"], format!("http://127.0.0.1:{port}"));
    assert!(session
        .request(Method::GET, "/v1/redirect", HeaderMap::new(), None)
        .await
        .is_err());
    let other = Transport::connect(&connection, "").await.unwrap();
    assert_eq!(
        other
            .send(Method::GET, "/v1/auth/me", HeaderMap::new(), None)
            .await
            .unwrap()
            .status,
        401
    );
    session.close().await;
    assert!(session
        .request(Method::GET, "/v1/auth/me", HeaderMap::new(), None)
        .await
        .is_err());
    server.abort();
}
