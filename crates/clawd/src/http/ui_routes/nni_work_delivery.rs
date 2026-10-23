// Host-owned execution policy and artifact delivery for distributed capabilities.
// Task metadata requests delivery; only this local policy authorizes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniArtifactDelivery {
    schema_version: u32,
    job_id: String,
    origin: String,
    upload_token: String,
    required_kinds: Vec<String>,
    max_file_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniWorkPolicy {
    schema_version: u32,
    principal_id: String,
    capabilities: Vec<String>,
    #[serde(default)]
    argument_constraints: BTreeMap<String, BTreeMap<String, Value>>,
    artifact_origins: Vec<String>,
    max_file_bytes: u64,
}

fn nni_read_work_policy(state: &AppState) -> anyhow::Result<NniWorkPolicy> {
    let path = std::env::var_os("APP_NNI_WORK_POLICY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| nni_work_dir(state).join("policy.json"));
    let bytes = std::fs::read(path)?;
    anyhow::ensure!(bytes.len() <= 16 * 1024, "work_policy_too_large");
    let policy: NniWorkPolicy = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(policy.schema_version == 1 && !policy.principal_id.is_empty()
        && policy.capabilities.len() <= 64 && policy.artifact_origins.len() <= 16
        && policy.max_file_bytes <= 512 * 1024 * 1024, "work_policy_invalid");
    Ok(policy)
}

fn nni_validate_artifact_delivery(delivery: &NniArtifactDelivery) -> bool {
    let Ok(url) = reqwest::Url::parse(&delivery.origin) else { return false; };
    delivery.schema_version == 1
        && uuid::Uuid::parse_str(&delivery.job_id).is_ok()
        && delivery.upload_token.len() == 64
        && delivery.upload_token.bytes().all(|b| b.is_ascii_hexdigit())
        && url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
        && url.origin().ascii_serialization() == delivery.origin
        && !delivery.required_kinds.is_empty() && delivery.required_kinds.len() <= 2
        && delivery.required_kinds.iter().all(|k| matches!(k.as_str(), "video" | "audio"))
        && delivery.required_kinds.iter().collect::<std::collections::BTreeSet<_>>().len() == delivery.required_kinds.len()
        && delivery.max_file_bytes > 0 && delivery.max_file_bytes <= 512 * 1024 * 1024
}

fn nni_capability_policy(state: &AppState, payload: &NniWorkCapabilityPayload) -> anyhow::Result<NniWorkPolicy> {
    let policy = nni_read_work_policy(state)?;
    anyhow::ensure!(policy.capabilities.contains(&payload.capability), "work_capability_not_granted");
    if let Some(constraints) = policy.argument_constraints.get(&payload.capability) {
        anyhow::ensure!(constraints.iter().all(|(key, value)| payload.args.get(key) == Some(value)),
            "work_arguments_not_granted");
    }
    if let Some(delivery) = &payload.artifact_delivery {
        anyhow::ensure!(nni_validate_artifact_delivery(delivery)
            && policy.artifact_origins.contains(&delivery.origin)
            && delivery.max_file_bytes <= policy.max_file_bytes, "work_delivery_not_granted");
    }
    Ok(policy)
}

async fn nni_upload_task_artifacts(
    state: &AppState,
    assignment: &NniWorkAssignment,
    local_task_id: &str,
    result: Option<&Value>,
) -> anyhow::Result<()> {
    let payload: NniWorkCapabilityPayload = serde_json::from_str(&assignment.payload_canonical)?;
    let Some(delivery) = &payload.artifact_delivery else { return Ok(()); };
    let _policy = nni_capability_policy(state, &payload)?;
    let manifests = crate::task_artifacts::manifests_from_result(result);
    // No redirects or implicit proxy settings may forward the scoped bearer grant.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()?;
    let mut missing = false;
    for kind in &delivery.required_kinds {
        let Some(manifest) = manifests.iter().find(|item| &item.kind == kind) else {
            missing = true;
            continue;
        };
        if manifest.size_bytes > delivery.max_file_bytes {
            return Err(NniDeliveryError("artifact_too_large").into());
        }
        anyhow::ensure!(manifest.size_bytes > 0, NniDeliveryError("artifact_size_invalid"));
        let path = crate::task_artifacts::validated_delivery_artifact_path(
            &state.skill_rt.workspace_root, local_task_id, manifest,
        ).ok_or(NniDeliveryError("artifact_path_invalid"))?;
        nni_upload_artifact_chunks(&client, assignment, delivery, manifest, &path).await?;
    }
    anyhow::ensure!(!missing, NniDeliveryError("required_artifact_missing"));
    Ok(())
}

include!("nni_work_upload_chunks.rs");

fn percent_encode_filename(value: &str) -> String {
    value.as_bytes().iter().map(|byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            char::from(*byte).to_string()
        } else { format!("%{byte:02X}") }
    }).collect()
}

#[cfg(test)]
#[path = "nni_work_delivery_tests.rs"]
mod nni_work_delivery_tests;
