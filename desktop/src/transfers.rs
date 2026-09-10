use crate::{
    profile::api_path,
    session::Session,
    transport::{ByteStream, WireResponse},
    Result, CHUNK_BYTES,
};
use bytes::Bytes;
use futures_util::StreamExt;
use http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestSpec {
    pub path: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub has_body: bool,
}
#[derive(Serialize)]
pub struct ResponseHead {
    pub status: u16,
    pub headers: HashMap<String, String>,
}
pub struct Transfer {
    pub session_id: Uuid,
    pub cancel: CancellationToken,
    upload: Mutex<Option<mpsc::Sender<Result<Bytes>>>>,
    response: Mutex<Option<oneshot::Receiver<Result<WireResponse>>>>,
    body: Mutex<Option<BodyCursor>>,
}
struct BodyCursor {
    stream: ByteStream,
    pending: Bytes,
}
#[derive(Default)]
pub struct Transfers {
    entries: Mutex<HashMap<Uuid, Arc<Transfer>>>,
}

pub fn renderer_headers(headers: HashMap<String, String>) -> Result<HeaderMap> {
    if headers.len() > 16 {
        return Err("request_header_denied".into());
    }
    let mut out = HeaderMap::new();
    for (name, value) in headers {
        // Authentication, cookies, proxy headers, Origin and Host belong to the native session.
        if !matches!(
            name.to_ascii_lowercase().as_str(),
            "accept"
                | "content-type"
                | "range"
                | "if-range"
                | "if-none-match"
                | "last-event-id"
                | "x-idempotency-key"
        ) || value.len() > 8192
        {
            return Err("request_header_denied".into());
        }
        out.insert(
            http::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| "request_header_invalid")?,
            value.parse().map_err(|_| "request_header_invalid")?,
        );
    }
    Ok(out)
}

impl Transfers {
    pub async fn start(&self, session: Arc<Session>, spec: RequestSpec) -> Result<Uuid> {
        api_path(&spec.path)?;
        if spec.path.starts_with("/webd/") || spec.path == "/v1/auth/ui-key/verify" {
            return Err("auth_command_required".into());
        }
        let method: Method = spec.method.parse().map_err(|_| "request_method_invalid")?;
        if !matches!(
            method,
            Method::GET
                | Method::HEAD
                | Method::POST
                | Method::PUT
                | Method::PATCH
                | Method::DELETE
                | Method::OPTIONS
        ) || (matches!(method, Method::GET | Method::HEAD) && spec.has_body)
        {
            return Err("request_method_denied".into());
        }
        let headers = renderer_headers(spec.headers)?;
        let mut entries = self.entries.lock().await;
        entries.retain(|_, t| !t.cancel.is_cancelled());
        if entries.len() >= 64 {
            return Err("transfer_limit".into());
        }
        let id = Uuid::new_v4();
        let cancel = session.cancelled.child_token();
        let (tx, rx) = mpsc::channel(4);
        let (response_tx, response_rx) = oneshot::channel();
        let transfer = Arc::new(Transfer {
            session_id: session.id,
            cancel: cancel.clone(),
            upload: Mutex::new(if spec.has_body { Some(tx) } else { None }),
            response: Mutex::new(Some(response_rx)),
            body: Mutex::new(None),
        });
        entries.insert(id, transfer);
        let body: Option<ByteStream> = spec
            .has_body
            .then(|| Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)) as ByteStream);
        tokio::spawn(async move {
            let response = tokio::select! {
                _ = cancel.cancelled() => Err("transfer_cancelled".into()),
                r = session.request(method, &spec.path, headers, body) => r,
            };
            let _ = response_tx.send(response);
        });
        Ok(id)
    }
    async fn get(&self, session_id: Uuid, id: Uuid) -> Result<Arc<Transfer>> {
        let transfer = self
            .entries
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or("transfer_not_found")?;
        if transfer.session_id != session_id || transfer.cancel.is_cancelled() {
            return Err("stale_connection".into());
        }
        Ok(transfer)
    }
    pub async fn headers(&self, session_id: Uuid, id: Uuid) -> Result<ResponseHead> {
        let transfer = self.get(session_id, id).await?;
        let rx = transfer
            .response
            .lock()
            .await
            .take()
            .ok_or("response_already_opened")?;
        let response = tokio::select! {
            _ = transfer.cancel.cancelled() => return Err("transfer_cancelled".into()),
            r = rx => r.map_err(|_| "transfer_failed")??,
        };
        let headers = response
            .headers
            .iter()
            .filter_map(|(k, v)| {
                if matches!(
                    k.as_str(),
                    "content-type"
                        | "content-length"
                        | "content-range"
                        | "accept-ranges"
                        | "content-disposition"
                        | "etag"
                        | "last-modified"
                        | "retry-after"
                ) {
                    v.to_str().ok().map(|v| (k.to_string(), v.to_string()))
                } else {
                    None
                }
            })
            .collect();
        *transfer.body.lock().await = Some(BodyCursor {
            stream: response.body,
            pending: Bytes::new(),
        });
        Ok(ResponseHead {
            status: response.status,
            headers,
        })
    }
    pub async fn read(&self, session_id: Uuid, id: Uuid) -> Result<Bytes> {
        let transfer = self.get(session_id, id).await?;
        let mut body = transfer.body.lock().await;
        let cursor = body.as_mut().ok_or("response_not_opened")?;
        if cursor.pending.is_empty() {
            loop {
                let chunk = tokio::select! {
                    _ = transfer.cancel.cancelled() => return Err("transfer_cancelled".into()),
                    chunk = cursor.stream.next() => chunk,
                };
                match chunk {
                    Some(Ok(bytes)) if bytes.is_empty() => continue,
                    Some(Ok(bytes)) => cursor.pending = bytes,
                    Some(Err(error)) => {
                        self.cancel(session_id, id).await;
                        return Err(error);
                    }
                    None => {
                        self.cancel(session_id, id).await;
                        return Ok(Bytes::new());
                    }
                }
                break;
            }
        }
        let size = cursor.pending.len().min(CHUNK_BYTES);
        Ok(cursor.pending.split_to(size))
    }
    pub async fn upload(&self, session_id: Uuid, id: Uuid, bytes: Bytes) -> Result<()> {
        if bytes.len() > CHUNK_BYTES {
            return Err("upload_chunk_too_large".into());
        }
        let transfer = self.get(session_id, id).await?;
        let upload = transfer.upload.lock().await;
        let sender = upload.as_ref().ok_or("upload_closed")?;
        tokio::select! {
            _ = transfer.cancel.cancelled() => Err("transfer_cancelled".into()),
            result = tokio::time::timeout(Duration::from_secs(60), sender.send(Ok(bytes))) => result.map_err(|_| "upload_stalled")?.map_err(|_| "upload_closed".into()),
        }
    }
    pub async fn finish_upload(&self, session_id: Uuid, id: Uuid) -> Result<()> {
        self.get(session_id, id).await?.upload.lock().await.take();
        Ok(())
    }
    pub async fn cancel(&self, session_id: Uuid, id: Uuid) {
        let mut entries = self.entries.lock().await;
        if entries.get(&id).is_some_and(|t| t.session_id == session_id) {
            if let Some(t) = entries.remove(&id) {
                t.cancel.cancel();
            }
        }
    }
    pub async fn cancel_session(&self, session_id: Uuid) {
        self.entries.lock().await.retain(|_, t| {
            if t.session_id == session_id {
                t.cancel.cancel();
                false
            } else {
                true
            }
        });
    }
}
