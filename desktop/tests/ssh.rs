mod support;
use agent_desktop::{
    credentials::LoginSecret,
    profile::{Connection, Profile},
    session::Session,
    transport::{bytes_body, small_json, Transport},
};
use http::{HeaderMap, Method};
use russh::{
    keys::{Algorithm, HashAlg, PrivateKey, PublicKey},
    server, Channel,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone)]
struct SshFixture {
    port: u16,
    auth_attempts: Arc<AtomicUsize>,
    client_key: PublicKey,
}
impl server::Handler for SshFixture {
    type Error = russh::Error;
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> std::result::Result<server::Auth, Self::Error> {
        self.auth_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "test-ssh" && key == &self.client_key {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> std::result::Result<server::Auth, Self::Error> {
        self.auth_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "test-ssh" && password == "ssh-test-password" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        host: &str,
        port: u32,
        _origin: &str,
        _origin_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> std::result::Result<(), Self::Error> {
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, u32::from(self.port));
        reply.accept().await;
        let port = self.port;
        tokio::spawn(async move {
            let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let mut channel = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut channel, &mut socket).await;
        });
        Ok(())
    }
}

#[tokio::test]
async fn ssh_verifies_host_before_auth_and_transports_webd_cookie_csrf() {
    let (webd_port, http_task) = support::http_server(support::Evidence::default()).await;
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let client_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let auth_attempts = Arc::new(AtomicUsize::new(0));
    let handler = SshFixture {
        port: webd_port,
        auth_attempts: auth_attempts.clone(),
        client_key: client_key.public_key().clone(),
    };
    let server_task = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let config = config.clone();
            let handler = handler.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    let connection = Connection::Ssh {
        host: "127.0.0.1".into(),
        port,
        username: "test-ssh".into(),
        host_key_sha256: fingerprint,
        webd_port,
    };
    let mut wrong = connection.clone();
    if let Connection::Ssh {
        host_key_sha256, ..
    } = &mut wrong
    {
        *host_key_sha256 = format!("SHA256:{}", "A".repeat(43));
    }
    assert!(Transport::connect(&wrong, "ssh-test-password")
        .await
        .is_err());
    assert_eq!(auth_attempts.load(Ordering::SeqCst), 0);
    let session = Session::connect(
        Profile {
            saved_login: false,
            id: Uuid::new_v4(),
            alias: "SSH test".into(),
            connection: connection.clone(),
        },
        Zeroizing::new("ssh-test-password".into()),
    )
    .await
    .unwrap();
    assert_eq!(auth_attempts.load(Ordering::SeqCst), 1);
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
                Some(bytes_body("over ssh")),
            )
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(status, 200);
    assert_eq!(body["data"]["bytes"], 8);
    assert!(body["data"]["origin"]
        .as_str()
        .unwrap()
        .starts_with("http://127.0.0.1:"));
    session.close().await;
    let pem = client_key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .unwrap();
    let key_transport = Transport::connect_with_key(&connection, "", Some(&pem), None)
        .await
        .unwrap();
    let response = key_transport
        .send(Method::GET, "/webd/session", HeaderMap::new(), None)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(auth_attempts.load(Ordering::SeqCst), 2);
    key_transport.close().await;
    server_task.abort();
    http_task.abort();
}
