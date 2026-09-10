use crate::{
    profile::{api_path, Connection},
    Result,
};
use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt};
use http::{HeaderMap, Method};
use http_body_util::{BodyExt, StreamBody};
use hyper_util::rt::TokioIo;
use reqwest::{
    cookie::{CookieStore, Jar},
    Url,
};
use russh::{
    client,
    keys::{HashAlg, PublicKeyOrCertificate},
};
use std::{pin::Pin, sync::Arc, time::Duration};
use tokio::sync::{watch, Mutex};

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;
pub struct WireResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: ByteStream,
}
pub enum Wire {
    Https(reqwest::Client),
    Ssh(Mutex<client::Handle<PinnedHost>>),
}
pub struct Transport {
    pub origin: Url,
    pub jar: Arc<Jar>,
    wire: Wire,
}
pub struct PinnedHost {
    expected: String,
}
impl client::Handler for PinnedHost {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        Ok(key.public_key().fingerprint(HashAlg::Sha256).to_string() == self.expected)
    }
}

impl Transport {
    pub async fn connect(connection: &Connection, ssh_password: &str) -> Result<Self> {
        Self::connect_with_key(connection, ssh_password, None, None).await
    }
    pub async fn connect_with_key(
        connection: &Connection,
        ssh_password: &str,
        private_key: Option<&str>,
        key_passphrase: Option<&str>,
    ) -> Result<Self> {
        connection.validate()?;
        let origin = connection.origin()?;
        let jar = Arc::new(Jar::default());
        let wire = match connection {
            Connection::Https { ca_pem, .. } => {
                let mut builder = reqwest::Client::builder()
                    .https_only(true)
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .min_tls_version(reqwest::tls::Version::TLS_1_2)
                    .connect_timeout(Duration::from_secs(15))
                    .cookie_provider(jar.clone());
                if let Some(pem) = ca_pem {
                    // A private profile trusts only its paired CA, never an additional global root.
                    builder = builder.tls_built_in_root_certs(false).add_root_certificate(
                        reqwest::Certificate::from_pem(pem.as_bytes())
                            .map_err(|_| "certificate_invalid")?,
                    );
                }
                Wire::Https(builder.build().map_err(|_| "tls_configuration_invalid")?)
            }
            Connection::Ssh {
                host,
                port,
                username,
                host_key_sha256,
                ..
            } => {
                let config = client::Config {
                    inactivity_timeout: None,
                    keepalive_interval: Some(Duration::from_secs(20)),
                    keepalive_max: 3,
                    ..Default::default()
                };
                let mut handle = tokio::time::timeout(
                    Duration::from_secs(15),
                    client::connect(
                        Arc::new(config),
                        (host.trim_matches(['[', ']']), *port),
                        PinnedHost {
                            expected: host_key_sha256.clone(),
                        },
                    ),
                )
                .await
                .map_err(|_| "connection_timeout")?
                .map_err(|e| match e {
                    russh::Error::UnknownKey => "ssh_identity_changed",
                    _ => "ssh_connection_failed",
                })?;
                let auth = if let Some(private_key) = private_key {
                    if private_key.len() > 32768 {
                        return Err("ssh_private_key_invalid".into());
                    }
                    let key = russh::keys::decode_secret_key(
                        private_key,
                        key_passphrase.filter(|s| !s.is_empty()),
                    )
                    .map_err(|_| "ssh_private_key_invalid")?;
                    let hash = tokio::time::timeout(
                        Duration::from_secs(15),
                        handle.best_supported_rsa_hash(),
                    )
                    .await
                    .map_err(|_| "ssh_auth_timeout")?
                    .map_err(|_| "ssh_auth_failed")?
                    .flatten();
                    let key = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash);
                    tokio::time::timeout(
                        Duration::from_secs(30),
                        handle.authenticate_publickey(username, key),
                    )
                    .await
                } else {
                    tokio::time::timeout(
                        Duration::from_secs(30),
                        handle.authenticate_password(username, ssh_password),
                    )
                    .await
                }
                .map_err(|_| "ssh_auth_timeout")?
                .map_err(|_| "ssh_auth_failed")?;
                if !auth.success() {
                    return Err("ssh_auth_failed".into());
                }
                Wire::Ssh(Mutex::new(handle))
            }
        };
        let transport = Self { origin, jar, wire };
        // No API credentials are sent until the authenticated transport succeeds.
        let probe = transport
            .send(Method::GET, "/webd/session", HeaderMap::new(), None)
            .await?;
        if !(200..300).contains(&probe.status) {
            return Err("webd_unavailable".into());
        }
        Ok(transport)
    }

    pub async fn send(
        &self,
        method: Method,
        path: &str,
        mut headers: HeaderMap,
        body: Option<ByteStream>,
    ) -> Result<WireResponse> {
        api_path(path)?;
        let target = self.origin.join(path).map_err(|_| "api_path_denied")?;
        if target.origin() != self.origin.origin() {
            return Err("request_origin_denied".into());
        }
        headers.insert(
            http::header::ORIGIN,
            self.origin
                .origin()
                .ascii_serialization()
                .parse()
                .map_err(|_| "address_invalid")?,
        );
        headers.insert("x-agent-client", http::HeaderValue::from_static("desktop"));
        // Reset the header wait whenever upload bytes advance. A large active upload
        // has no total duration cap; only a stalled connection times out.
        let (activity, last_activity) = watch::channel(std::time::Instant::now());
        let body = body.map(|stream| {
            Box::pin(stream.inspect_ok(move |_| {
                activity.send_replace(std::time::Instant::now());
            })) as ByteStream
        });
        match &self.wire {
            Wire::Https(client) => {
                let mut request = client.request(method, target).headers(headers);
                if let Some(body) = body {
                    request = request.body(reqwest::Body::wrap_stream(
                        body.map_err(std::io::Error::other),
                    ));
                }
                let response = await_headers(request.send(), last_activity)
                    .await?
                    .map_err(classify_http_error)?;
                reject_redirect(response.status().as_u16())?;
                Ok(WireResponse {
                    status: response.status().as_u16(),
                    headers: response.headers().clone(),
                    body: Box::pin(
                        response
                            .bytes_stream()
                            .map_err(|_| "response_stream_interrupted".into()),
                    ),
                })
            }
            Wire::Ssh(handle) => {
                let channel = tokio::time::timeout(Duration::from_secs(15), async {
                    handle
                        .lock()
                        .await
                        .channel_open_direct_tcpip(
                            "127.0.0.1",
                            u32::from(self.origin.port().unwrap_or(80)),
                            "127.0.0.1",
                            0,
                        )
                        .await
                })
                .await
                .map_err(|_| "connection_timeout")?
                .map_err(|_| "ssh_tunnel_failed")?;
                let (mut sender, connection) =
                    hyper::client::conn::http1::handshake(TokioIo::new(channel.into_stream()))
                        .await
                        .map_err(|_| "ssh_http_handshake_failed")?;
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                let host = match self.origin.port() {
                    Some(port) => format!("127.0.0.1:{port}"),
                    None => "127.0.0.1".into(),
                };
                headers.insert(
                    http::header::HOST,
                    host.parse().map_err(|_| "address_invalid")?,
                );
                if let Some(cookies) = self.jar.cookies(&target) {
                    headers.insert(http::header::COOKIE, cookies);
                }
                let stream: ByteStream =
                    body.unwrap_or_else(|| Box::pin(futures_util::stream::empty()));
                let body = StreamBody::new(
                    stream
                        .map_ok(hyper::body::Frame::data)
                        .map_err(std::io::Error::other),
                );
                let mut request = http::Request::builder()
                    .method(method)
                    .uri(path)
                    .body(body)
                    .map_err(|_| "request_invalid")?;
                *request.headers_mut() = headers;
                let response = await_headers(sender.send_request(request), last_activity)
                    .await?
                    .map_err(|_| "ssh_http_failed")?;
                reject_redirect(response.status().as_u16())?;
                self.jar.set_cookies(
                    &mut response.headers().get_all(http::header::SET_COOKIE).iter(),
                    &target,
                );
                let (parts, body) = response.into_parts();
                Ok(WireResponse {
                    status: parts.status.as_u16(),
                    headers: parts.headers,
                    body: Box::pin(
                        body.into_data_stream()
                            .map_err(|_| "response_stream_interrupted".into()),
                    ),
                })
            }
        }
    }

    pub async fn close(&self) {
        if let Wire::Ssh(handle) = &self.wire {
            let _ = handle
                .lock()
                .await
                .disconnect(russh::Disconnect::ByApplication, "", "")
                .await;
        }
    }
}

async fn await_headers<T>(
    future: impl std::future::Future<Output = T>,
    activity: watch::Receiver<std::time::Instant>,
) -> Result<T> {
    tokio::pin!(future);
    let mut check = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = check.tick() => if activity.borrow().elapsed() > Duration::from_secs(120) { return Err("response_headers_timeout".into()); },
        }
    }
}

fn reject_redirect(status: u16) -> Result<()> {
    if (300..400).contains(&status) {
        Err("redirect_denied".into())
    } else {
        Ok(())
    }
}
fn classify_http_error(error: reqwest::Error) -> String {
    // Do not return request URLs, TLS stack details, or authentication material to logs/UI.
    if error.is_timeout() {
        "connection_timeout"
    } else if error.is_connect() {
        "tls_or_connection_failed"
    } else {
        "transport_failed"
    }
    .into()
}
pub fn bytes_body(bytes: impl Into<Bytes>) -> ByteStream {
    let bytes = bytes.into();
    Box::pin(futures_util::stream::once(async move { Ok(bytes) }))
}
pub async fn small_json(mut response: WireResponse) -> Result<(u16, serde_json::Value)> {
    let mut bytes = Vec::new();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(30), response.body.next())
        .await
        .map_err(|_| "response_body_timeout")?
    {
        let chunk = chunk?;
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("response_too_large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((
        response.status,
        serde_json::from_slice(&bytes).map_err(|_| "response_json_invalid")?,
    ))
}
