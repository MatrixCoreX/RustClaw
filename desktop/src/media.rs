use crate::{commands::DesktopState, profile::api_path, Result};
use futures_util::StreamExt;
use http::{HeaderMap, Method, Request, Response};
use tauri::Manager;
use uuid::Uuid;

pub use crate::webview_origin::local_page;

pub async fn serve(
    app: tauri::AppHandle,
    label: String,
    request: Request<Vec<u8>>,
) -> Result<Response<Vec<u8>>> {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return Err("media_method_denied".into());
    }
    let state = app.state::<DesktopState>();
    let uri = request.uri();
    let tail = uri.path().strip_prefix('/').ok_or("media_path_denied")?;
    let (id, rest) = tail.split_once('/').ok_or("media_path_denied")?;
    let id = Uuid::parse_str(id).map_err(|_| "media_path_denied")?;
    let mut path = format!("/{rest}");
    if let Some(query) = uri.query() {
        path.push('?');
        path.push_str(query);
    }
    api_path(&path)?;
    let session = state.session(id).await?;
    // Dynamic AiAPP assets use this scheme. Native media playback uses the
    // token-bound loopback relay because WebKitGTK rejects custom media schemes.
    let scope = state
        .aipps
        .lock()
        .await
        .get(&label)
        .cloned()
        .ok_or("aipp_scope_denied")?;
    if scope.session_id != id
        || !rest.starts_with(&format!("v1/aipps/{}/assets/", scope.skill_name))
    {
        return Err("aipp_scope_denied".into());
    }
    crate::aipp::validate_scope(&session, &scope).await?;
    let headers = HeaderMap::new();
    let mut response = session
        .request(request.method().clone(), &path, headers, None)
        .await?;
    let mut builder = Response::builder().status(response.status);
    for name in [
        "content-type",
        "content-range",
        "accept-ranges",
        "etag",
        "last-modified",
    ] {
        if let Some(value) = response.headers.get(name) {
            builder = builder.header(name, value);
        }
    }
    builder = builder
        .header("access-control-allow-origin", "*")
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff");
    builder = builder.header(
        "content-security-policy",
        crate::webview_origin::asset_csp(),
    );
    let mut bytes = Vec::new();
    let limit = 8 * 1024 * 1024;
    loop {
        let chunk = tokio::select! {
            _ = session.cancelled.cancelled() => return Err("connection_closed".into()),
            chunk = response.body.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk?;
        if bytes.len() + chunk.len() > limit {
            return Err("media_range_required".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    builder
        .header("content-length", bytes.len())
        .body(bytes)
        .map_err(|_| "media_response_invalid".into())
}
