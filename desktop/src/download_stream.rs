use crate::{session::Session, Result};
use futures_util::StreamExt;
use http::{HeaderMap, Method};
use std::path::Path;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub async fn save_response(
    session: &Session,
    path: &str,
    target: &Path,
    cancel: &CancellationToken,
    mut progress: impl FnMut(u64, Option<u64>, bool),
) -> Result<()> {
    let parent = target.parent().ok_or("download_path_invalid")?;
    let tmp = parent.join(format!(".agent-download-{}.part", Uuid::new_v4()));
    let outcome: Result<()> = async {
        let mut response = session
            .request(Method::GET, &path, HeaderMap::new(), None)
            .await?;
        if response.status != 200 || response.headers.contains_key(http::header::CONTENT_RANGE) {
            return Err("download_http_failed".into());
        }
        let expected = response
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&tmp)
            .await
            .map_err(|_| "download_file_unavailable")?;
        let mut written = 0_u64;
        let mut last_progress = std::time::Instant::now();
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err("download_cancelled".into()),
                chunk = tokio::time::timeout(std::time::Duration::from_secs(120), response.body.next()) => chunk.map_err(|_| "download_stalled")?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            let chunk = chunk?;
            file.write_all(&chunk)
                .await
                .map_err(|_| "download_disk_full_or_unavailable")?;
            written += chunk.len() as u64;
            if last_progress.elapsed() >= std::time::Duration::from_millis(250) {
                progress(written, expected, false);
                last_progress = std::time::Instant::now();
            }
        }
        if expected.is_some_and(|size| size != written) {
            return Err("download_incomplete".into());
        }
        file.sync_all()
            .await
            .map_err(|_| "download_disk_full_or_unavailable")?;
        drop(file);
        if cancel.is_cancelled() {
            return Err("download_cancelled".into());
        }
        tokio::fs::rename(&tmp, &target)
            .await
            .map_err(|_| "download_finish_failed")?;
        progress(written, expected, true);
        Ok(())
    }
    .await;
    if outcome.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    outcome
}
