const NNI_ARTIFACT_CHUNK_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct NniDeliveryError(&'static str);

// Only stable machine codes cross into the public result; never expose bodies,
// scoped grants, local paths or provider diagnostics to an anonymous browser.
fn nni_upload_error_code(status: reqwest::StatusCode, body: &Value) -> &'static str {
    match body.get("error_code").and_then(Value::as_str) {
        Some("storage_unavailable") => "storage_unavailable",
        Some("upload_grant_invalid") => "upload_grant_invalid",
        Some("upload_lease_invalid") => "upload_lease_invalid",
        Some("artifact_digest_invalid") => "artifact_digest_invalid",
        _ if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE => "artifact_too_large",
        _ => "artifact_upload_failed",
    }
}

async fn nni_upload_request_json(
    request: impl Fn() -> reqwest::RequestBuilder,
) -> anyhow::Result<Value> {
    for retry in 0..3 {
        match request().send().await {
            Ok(response) => {
                let status = response.status();
                // Receiver responses are small machine acknowledgements.
                let mut stream = response.bytes_stream();
                let mut bytes = Vec::new();
                use futures_util::StreamExt;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    anyhow::ensure!(bytes.len() + chunk.len() <= 16 * 1024,
                        NniDeliveryError("artifact_upload_failed"));
                    bytes.extend_from_slice(&chunk);
                }
                let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                if status.is_success() {
                    anyhow::ensure!(body.get("ok").and_then(Value::as_bool) == Some(true),
                        NniDeliveryError("artifact_upload_failed"));
                    return Ok(body.get("data").cloned().unwrap_or(Value::Null));
                }
                if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(NniDeliveryError(nni_upload_error_code(status, &body)).into());
                }
            }
            Err(_) => {}
        }
        if retry < 2 { tokio::time::sleep(Duration::from_secs(1 << retry)).await; }
    }
    Err(NniDeliveryError("artifact_upload_failed").into())
}

fn nni_upload_headers(
    request: reqwest::RequestBuilder,
    assignment: &NniWorkAssignment,
    delivery: &NniArtifactDelivery,
) -> reqwest::RequestBuilder {
    request.bearer_auth(&delivery.upload_token)
        .header("x-work-task-id", &assignment.task_id)
        .header("x-work-attempt", assignment.attempt)
}

fn nni_upload_acknowledged(ack: &Value, sha256: &str) -> bool {
    matches!(ack.get("status").and_then(Value::as_str), Some("uploaded" | "already_uploaded"))
        && ack.get("sha256").and_then(Value::as_str) == Some(sha256)
}

async fn nni_upload_artifact_chunks(
    client: &reqwest::Client,
    assignment: &NniWorkAssignment,
    delivery: &NniArtifactDelivery,
    manifest: &crate::task_artifacts::TaskArtifactManifest,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    use tokio::io::AsyncSeekExt;
    use sha2::Digest;
    let upload_kind = nni_artifact_upload_kind(&manifest.kind, &manifest.mime_type)
        .ok_or(NniDeliveryError("artifact_kind_invalid"))?;
    let url = format!("{}/api/media/v1/worker/jobs/{}/artifacts/{}", delivery.origin,
        delivery.job_id, manifest.id);
    let start = nni_upload_request_json(|| {
        nni_upload_headers(client.post(format!("{url}/start")), assignment, delivery)
            .header("x-artifact-kind", upload_kind)
            .header("x-artifact-filename", percent_encode_filename(&manifest.filename))
            .header("x-artifact-sha256", &manifest.sha256)
            .header("x-artifact-size", manifest.size_bytes)
            .header(reqwest::header::CONTENT_TYPE, &manifest.mime_type)
            .header(reqwest::header::CONTENT_LENGTH, 0)
    }).await?;
    if nni_upload_acknowledged(&start, &manifest.sha256) { return Ok(()); }
    anyhow::ensure!(start.get("chunk_bytes").and_then(Value::as_u64) == Some(NNI_ARTIFACT_CHUNK_BYTES)
        && start.get("size_bytes").and_then(Value::as_u64) == Some(manifest.size_bytes),
        NniDeliveryError("artifact_upload_failed"));
    let part_count = manifest.size_bytes.div_ceil(NNI_ARTIFACT_CHUNK_BYTES);
    let received = start.get("received_parts").and_then(Value::as_array)
        .ok_or(NniDeliveryError("artifact_upload_failed"))?;
    anyhow::ensure!(received.len() as u64 <= part_count && received.iter()
        .all(|v| v.as_u64().is_some_and(|index| index < part_count)),
        NniDeliveryError("artifact_upload_failed"));
    let mut file = tokio::fs::File::open(path).await?;
    for index in 0..part_count {
        if received.iter().any(|v| v.as_u64() == Some(index)) { continue; }
        let offset = index * NNI_ARTIFACT_CHUNK_BYTES;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let size = NNI_ARTIFACT_CHUNK_BYTES.min(manifest.size_bytes - offset);
        let mut bytes = vec![0u8; size as usize];
        file.read_exact(&mut bytes).await?;
        let digest = hex::encode(sha2::Sha256::digest(&bytes));
        let part = nni_upload_request_json(|| {
            nni_upload_headers(client.put(format!("{url}/parts/{index}")), assignment, delivery)
                .header("x-chunk-sha256", &digest)
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .header(reqwest::header::CONTENT_LENGTH, size)
                .body(bytes.clone())
        }).await?;
        anyhow::ensure!(part.get("sha256").and_then(Value::as_str) == Some(digest.as_str())
            && part.get("part").and_then(Value::as_u64) == Some(index),
            NniDeliveryError("artifact_upload_failed"));
    }
    let finish = nni_upload_request_json(|| nni_upload_headers(
        client.post(format!("{url}/finish")), assignment, delivery)
        .header(reqwest::header::CONTENT_LENGTH, 0)).await?;
    anyhow::ensure!(nni_upload_acknowledged(&finish, &manifest.sha256),
        NniDeliveryError("artifact_digest_invalid"));
    Ok(())
}
