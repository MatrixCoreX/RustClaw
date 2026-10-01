use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

#[derive(Debug, thiserror::Error)]
pub enum ChannelMediaDownloadError {
    #[error("channel_media_download_limit_invalid")]
    InvalidLimit,
    #[error("channel_media_download_path_invalid")]
    InvalidPath,
    #[error("channel_media_download_too_large:{actual_bytes}:{max_bytes}")]
    TooLarge { actual_bytes: u64, max_bytes: u64 },
    #[error("channel_media_download_body_failed:{0}")]
    Body(#[source] reqwest::Error),
    #[error("channel_media_download_io_failed:{0}")]
    Io(#[source] std::io::Error),
}

pub async fn persist_bounded_response(
    mut response: reqwest::Response,
    destination: &Path,
    max_bytes: u64,
) -> Result<u64, ChannelMediaDownloadError> {
    if max_bytes == 0 {
        return Err(ChannelMediaDownloadError::InvalidLimit);
    }
    if let Some(actual_bytes) = response.content_length() {
        if actual_bytes > max_bytes {
            return Err(ChannelMediaDownloadError::TooLarge {
                actual_bytes,
                max_bytes,
            });
        }
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or(ChannelMediaDownloadError::InvalidPath)?;
    let filename = destination
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or(ChannelMediaDownloadError::InvalidPath)?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(ChannelMediaDownloadError::Io)?;
    let temporary = temporary_download_path(parent, filename);
    let mut output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .await
        .map_err(ChannelMediaDownloadError::Io)?;
    let result = async {
        let mut written = 0_u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(ChannelMediaDownloadError::Body)?
        {
            let next = written.saturating_add(chunk.len() as u64);
            if next > max_bytes {
                return Err(ChannelMediaDownloadError::TooLarge {
                    actual_bytes: next,
                    max_bytes,
                });
            }
            output
                .write_all(&chunk)
                .await
                .map_err(ChannelMediaDownloadError::Io)?;
            written = next;
        }
        output
            .flush()
            .await
            .map_err(ChannelMediaDownloadError::Io)?;
        drop(output);
        replace_download(&temporary, destination).await?;
        Ok(written)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

fn temporary_download_path(parent: &Path, filename: &str) -> PathBuf {
    parent.join(format!(
        ".{filename}.{}.part",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn replace_download(
    temporary: &Path,
    destination: &Path,
) -> Result<(), ChannelMediaDownloadError> {
    match tokio::fs::rename(temporary, destination).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            tokio::fs::remove_file(destination)
                .await
                .map_err(ChannelMediaDownloadError::Io)?;
            tokio::fs::rename(temporary, destination)
                .await
                .map_err(ChannelMediaDownloadError::Io)
        }
        Err(error) => Err(ChannelMediaDownloadError::Io(error)),
    }
}

#[cfg(test)]
#[path = "channel_media_download_tests.rs"]
mod tests;
