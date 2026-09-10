use crate::{profile::api_path, session::Session, Result};
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};
use http::{HeaderMap, Method, Request, Response};
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full, StreamBody};
use hyper::{
    body::{Frame, Incoming},
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, Semaphore},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

type Body = UnsyncBoxBody<Bytes, std::io::Error>;
struct Grant {
    session: Weak<Session>,
    path: String,
    cancel: CancellationToken,
    created: Instant,
}
pub struct MediaRelay {
    authority: String,
    grants: Mutex<HashMap<String, Arc<Grant>>>,
}

pub fn media_path(path: &str) -> Result<()> {
    api_path(path)?;
    let path = path.split('?').next().unwrap_or("");
    if (path.starts_with("/v1/tasks/")
        && path.contains("/artifacts/")
        && path.ends_with("/content"))
        || (path.starts_with("/v1/aipps/")
            && path.contains("/items/")
            && path.ends_with("/preview"))
    {
        Ok(())
    } else {
        Err("media_path_denied".into())
    }
}

impl MediaRelay {
    pub async fn start() -> Result<Arc<Self>> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "media_listener_unavailable")?;
        let relay = Arc::new(Self {
            authority: listener
                .local_addr()
                .map_err(|_| "media_listener_unavailable")?
                .to_string(),
            grants: Mutex::new(HashMap::new()),
        });
        let running = Arc::downgrade(&relay);
        tokio::spawn(async move {
            let slots = Arc::new(Semaphore::new(16));
            while let Ok((socket, peer)) = listener.accept().await {
                if !peer.ip().is_loopback() {
                    continue;
                }
                let Ok(slot) = slots.clone().try_acquire_owned() else {
                    continue;
                };
                let Some(relay) = running.upgrade() else {
                    break;
                };
                tokio::spawn(async move {
                    let _slot = slot;
                    let mut server = hyper::server::conn::http1::Builder::new();
                    server
                        .keep_alive(false)
                        .max_headers(32)
                        .timer(TokioTimer::new())
                        .header_read_timeout(Duration::from_secs(5));
                    let _ = server
                        .serve_connection(
                            TokioIo::new(socket),
                            service_fn(move |request| {
                                let relay = relay.clone();
                                async move {
                                    Ok::<_, Infallible>(
                                        relay.serve(request).await.unwrap_or_else(|_| denied()),
                                    )
                                }
                            }),
                        )
                        .await;
                });
            }
        });
        Ok(relay)
    }
    pub async fn grant(&self, session: Arc<Session>, path: String) -> Result<String> {
        media_path(&path)?;
        if session.cancelled.is_cancelled() || session.info().await.identity.is_none() {
            return Err("login_required".into());
        }
        let mut grants = self.grants.lock().await;
        grants.retain(|_, g| {
            !g.cancel.is_cancelled() && g.created.elapsed() < Duration::from_secs(10800)
        });
        if grants.len() >= 8 {
            return Err("media_limit".into());
        }
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        grants.insert(
            token.clone(),
            Arc::new(Grant {
                session: Arc::downgrade(&session),
                path,
                cancel: session.cancelled.child_token(),
                created: Instant::now(),
            }),
        );
        Ok(format!("http://{}/{token}", self.authority))
    }
    pub async fn revoke(&self, url: &str) {
        if let Some(token) = url.strip_prefix(&format!("http://{}/", self.authority)) {
            if let Some(grant) = self.grants.lock().await.remove(token) {
                grant.cancel.cancel();
            }
        }
    }
    async fn serve(&self, request: Request<Incoming>) -> Result<Response<Body>> {
        if !matches!(*request.method(), Method::GET | Method::HEAD)
            || request.uri().query().is_some()
            || request.headers().get("host").and_then(|v| v.to_str().ok())
                != Some(self.authority.as_str())
            || request.headers().get("origin").is_some_and(|v| {
                !matches!(
                    v.to_str(),
                    Ok("tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost")
                )
            })
        {
            return Err("media_access_denied".into());
        }
        let token = request
            .uri()
            .path()
            .strip_prefix('/')
            .ok_or("media_access_denied")?;
        let grant = self
            .grants
            .lock()
            .await
            .get(token)
            .cloned()
            .ok_or("media_access_denied")?;
        if grant.cancel.is_cancelled() || grant.created.elapsed() >= Duration::from_secs(10800) {
            return Err("media_access_denied".into());
        }
        let session = grant.session.upgrade().ok_or("stale_connection")?;
        let mut headers = HeaderMap::new();
        if let Some(range) = request.headers().get("range") {
            let value = range.to_str().map_err(|_| "media_range_invalid")?;
            if value.len() > 100
                || !value.starts_with("bytes=")
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_digit() || b"bytes=-".contains(&c))
            {
                return Err("media_range_invalid".into());
            }
            headers.insert("range", range.clone());
        }
        let response = session
            .request(request.method().clone(), &grant.path, headers, None)
            .await?;
        let mut builder = Response::builder()
            .status(response.status)
            .header("cache-control", "no-store")
            .header("referrer-policy", "no-referrer")
            .header("content-security-policy", "default-src 'none'; sandbox")
            .header("x-content-type-options", "nosniff");
        for name in [
            "content-type",
            "content-length",
            "content-range",
            "accept-ranges",
        ] {
            if let Some(value) = response.headers.get(name) {
                builder = builder.header(name, value);
            }
        }
        let body = futures_util::stream::unfold((response.body, grant), |(mut body, grant)| async move {
            let next = tokio::select! {
                _ = grant.cancel.cancelled() => return None,
                next = tokio::time::timeout(Duration::from_secs(120), body.next()) => next.unwrap_or(Some(Err("media_stalled".into()))),
            };
            next.map(|bytes| (bytes.map_err(std::io::Error::other), (body, grant)))
        }).map_ok(Frame::data);
        builder
            .body(StreamBody::new(Box::pin(body)).boxed_unsync())
            .map_err(|_| "media_response_invalid".into())
    }
}
fn denied() -> Response<Body> {
    Response::builder()
        .status(403)
        .header("cache-control", "no-store")
        .body(
            Full::new(Bytes::new())
                .map_err(|never| match never {})
                .boxed_unsync(),
        )
        .unwrap()
}
