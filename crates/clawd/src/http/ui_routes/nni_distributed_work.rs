const NNI_WORK_STATE_SCHEMA_VERSION: u32 = 1;
const NNI_WORK_PROTOCOL_VERSION: u32 = 1;
const NNI_WORK_RESULT_STREAM_BYTES: usize = 16 * 1024;
const NNI_WORK_REPORT_JSON_BYTES: usize = 48 * 1024;
const NNI_WORK_COMMAND_TIMEOUT_DEFAULT_SECONDS: u64 = 300;
const NNI_WORK_COMMAND_TIMEOUT_MAX_SECONDS: u64 = 300;
const NNI_WORK_CAPABILITY_ARGS_BYTES: usize = 32 * 1024;
const NNI_WORK_SIGNED_PAYLOAD_BYTES: usize = 256 * 1024;
const NNI_WORK_ASSIGNMENT_SCHEMA_VERSION: u32 = 2;
const NNI_WORK_ASSIGNMENT_SIGNATURE_ALGORITHM: &str = "secp256k1-sha256";
const NNI_WORK_ASSIGNMENT_SIGNATURE_DOMAIN: &str = "nni-distributed-work-assignment-v2\n";
const NNI_WORK_CONTROL_SIGNATURE_DOMAIN: &str = "nni-distributed-work-control-v2\n";
const NNI_WORK_HEARTBEAT_CHALLENGE_DOMAIN: &str = "nni-heartbeat-work-v2\0";
const NNI_WORK_ASSIGNMENT_VERIFY_KEYS_ENV: &str = "APP_NNI_WORK_ASSIGNMENT_VERIFY_KEYS";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniWorkAssignmentEnvelope {
    schema_version: u32,
    algorithm: String,
    signing_key_id: String,
    payload_canonical: String,
    signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniWorkControl {
    schema_version: u32,
    heartbeat_task_id: String,
    device_pubkey: String,
    work_report_status: String,
    work_report_error_code: Option<String>,
    work_poll_status: String,
    work_poll_error_code: Option<String>,
    work_assignment: Option<NniWorkAssignmentEnvelope>,
    issued_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NniActiveWorkLease {
    task_id: String,
    attempt: u32,
    lease_token_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NniWorkerCapabilities {
    protocol_version: u32,
    platform: String,
    arch: String,
    supported_task_types: Vec<String>,
    worker_instance_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_lease: Option<NniActiveWorkLease>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniWorkAssignment {
    schema_version: u32,
    heartbeat_task_id: String,
    task_id: String,
    task_type: String,
    target_device_pubkey: String,
    target_platform: String,
    target_arch: String,
    attempt: u32,
    lease_token: String,
    lease_expires_at_unix: u64,
    payload_canonical: String,
    payload_sha256: String,
    issued_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniWorkCommandPayload {
    program: String,
    args: Vec<String>,
    timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniWorkCapabilityPayload {
    capability: String,
    args: Value,
}

#[derive(Debug, Clone)]
enum NniWorkPayload {
    Command(NniWorkCommandPayload),
    Capability(NniWorkCapabilityPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniWorkResult {
    status: String,
    exit_code: Option<i32>,
    duration_ms: u64,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
    error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniLocalWorkState {
    schema_version: u32,
    node_url: String,
    status: String,
    assignment: NniWorkAssignment,
    #[serde(default)]
    local_task_id: Option<String>,
    #[serde(default)]
    started_at_unix: Option<i64>,
    #[serde(default)]
    result: Option<NniWorkResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NniHeartbeatWorkReport {
    task_id: String,
    attempt: u32,
    lease_token: String,
    result: NniWorkResult,
}

fn nni_work_execution_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn nni_work_pull_enabled() -> bool {
    std::env::var("APP_NNI_WORK_PULL_ENABLED")
        .ok()
        .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

fn nni_work_assignment_verifying_keys(
) -> Result<BTreeMap<String, k256::ecdsa::VerifyingKey>, &'static str> {
    let raw = std::env::var(NNI_WORK_ASSIGNMENT_VERIFY_KEYS_ENV).unwrap_or_default();
    nni_parse_work_assignment_verifying_keys(&raw)
}

fn nni_parse_work_assignment_verifying_keys(
    raw: &str,
) -> Result<BTreeMap<String, k256::ecdsa::VerifyingKey>, &'static str> {
    use sha2::{Digest, Sha256};

    let mut keys = BTreeMap::new();
    for encoded in raw.split(',').map(str::trim).filter(|value| !value.is_empty()) {
        if encoded.len() != 66 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("assignment_verify_key_invalid");
        }
        let bytes = hex::decode(encoded).map_err(|_| "assignment_verify_key_invalid")?;
        let key = k256::ecdsa::VerifyingKey::from_sec1_bytes(&bytes)
            .map_err(|_| "assignment_verify_key_invalid")?;
        let key_id = hex::encode(Sha256::digest(&bytes));
        if keys.insert(key_id, key).is_some() {
            return Err("assignment_verify_key_duplicate");
        }
    }
    if keys.is_empty() {
        return Err("assignment_verify_key_required");
    }
    Ok(keys)
}

fn nni_work_allowed_programs() -> BTreeSet<String> {
    std::env::var("APP_NNI_WORK_ALLOWED_PROGRAMS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| nni_work_program_name_valid(value))
        .map(str::to_string)
        .collect()
}

fn nni_work_program_name_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| {
                byte.is_ascii_alphanumeric()
                    || (index > 0 && matches!(byte, b'.' | b'_' | b'+' | b'-'))
            })
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "bash" | "csh" | "dash" | "doas" | "env" | "fish" | "ksh" | "sh" | "su"
                | "sudo" | "tcsh" | "zsh"
        )
}

fn nni_work_command_timeout_max_seconds() -> u64 {
    std::env::var("APP_NNI_WORK_COMMAND_TIMEOUT_MAX_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| (1..=NNI_WORK_COMMAND_TIMEOUT_MAX_SECONDS).contains(value))
        .unwrap_or(NNI_WORK_COMMAND_TIMEOUT_DEFAULT_SECONDS)
}

fn nni_work_platform() -> Option<&'static str> {
    match std::env::consts::OS {
        "linux" => Some("linux"),
        "macos" => Some("macos"),
        _ => None,
    }
}

fn nni_work_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    }
}

fn nni_work_dir(state: &AppState) -> PathBuf {
    state.skill_rt.workspace_root.join("data").join("nni").join("work")
}

fn nni_work_state_path(state: &AppState) -> PathBuf {
    nni_work_dir(state).join("current.json")
}

fn nni_work_instance_path(state: &AppState) -> PathBuf {
    nni_work_dir(state).join("worker-instance-id")
}

fn nni_work_audit_path(state: &AppState) -> PathBuf {
    nni_work_dir(state).join("events.jsonl")
}

fn nni_write_private_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("nni_work_private_path_invalid"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|value| value.to_str()).unwrap_or("state"),
        uuid::Uuid::new_v4().simple()
    ));
    fs::write(&temporary, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

fn nni_worker_instance_id(state: &AppState) -> anyhow::Result<String> {
    let path = nni_work_instance_path(state);
    if let Ok(raw) = fs::read_to_string(&path) {
        let value = raw.trim();
        if uuid::Uuid::parse_str(value).is_ok() {
            return Ok(value.to_string());
        }
    }
    let value = uuid::Uuid::new_v4().to_string();
    nni_write_private_file(&path, format!("{value}\n").as_bytes())?;
    Ok(value)
}

fn read_nni_local_work_state(state: &AppState) -> anyhow::Result<Option<NniLocalWorkState>> {
    let raw = match fs::read_to_string(nni_work_state_path(state)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let parsed: NniLocalWorkState = serde_json::from_str(&raw)?;
    if parsed.schema_version != NNI_WORK_STATE_SCHEMA_VERSION {
        anyhow::bail!("nni_work_state_schema_unsupported");
    }
    Ok(Some(parsed))
}

fn write_nni_local_work_state(state: &AppState, value: &NniLocalWorkState) -> anyhow::Result<()> {
    nni_write_private_file(&nni_work_state_path(state), &serde_json::to_vec_pretty(value)?)
}

fn append_nni_work_audit(state: &AppState, local: &NniLocalWorkState) -> anyhow::Result<()> {
    let path = nni_work_audit_path(state);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("nni_work_audit_path_invalid"))?;
    fs::create_dir_all(parent)?;
    let event = json!({
        "schema_version": 1,
        "task_id": local.assignment.task_id,
        "attempt": local.assignment.attempt,
        "task_type": local.assignment.task_type,
        "status": local.result.as_ref().map(|result| result.status.as_str()).unwrap_or("unknown"),
        "exit_code": local.result.as_ref().and_then(|result| result.exit_code),
        "duration_ms": local.result.as_ref().map(|result| result.duration_ms),
        "error_code": local.result.as_ref().and_then(|result| result.error_code.as_deref()),
        "reported_at_ts": current_unix_ts(),
    });
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    writeln!(file, "{}", serde_json::to_string(&event)?)?;
    Ok(())
}

fn nni_work_sha256(value: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn nni_verify_heartbeat_work_challenge(
    challenge: &str,
    nonce: Option<&str>,
    capabilities: Option<&NniWorkerCapabilities>,
    report: Option<&NniHeartbeatWorkReport>,
) -> Result<(), &'static str> {
    use sha2::{Digest, Sha256};

    if capabilities.is_none() && report.is_none() {
        return Ok(());
    }
    let nonce = nonce.ok_or("heartbeat_work_challenge_nonce_missing")?;
    if nonce.len() != 64 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("heartbeat_work_challenge_nonce_invalid");
    }
    let capabilities_json = capabilities
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| "heartbeat_work_capabilities_encode_failed")?
        .unwrap_or_else(|| "null".to_string());
    let report_json = report
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| "heartbeat_work_report_encode_failed")?
        .unwrap_or_else(|| "null".to_string());
    let capabilities_sha256 = hex::encode(Sha256::digest(capabilities_json.as_bytes()));
    let report_sha256 = hex::encode(Sha256::digest(report_json.as_bytes()));
    let expected = hex::encode(Sha256::digest(
        format!(
            "{NNI_WORK_HEARTBEAT_CHALLENGE_DOMAIN}{}\0{}\0{}",
            nonce.to_ascii_lowercase(),
            capabilities_sha256,
            report_sha256,
        )
        .as_bytes(),
    ));
    if challenge != expected {
        return Err("heartbeat_work_challenge_binding_invalid");
    }
    Ok(())
}

fn nni_current_worker_capabilities(state: &AppState) -> Option<NniWorkerCapabilities> {
    if !nni_work_pull_enabled() || nni_work_assignment_verifying_keys().is_err() {
        return None;
    }
    let local = read_nni_local_work_state(state).ok().flatten();
    if local
        .as_ref()
        .is_some_and(|value| value.status == "completed_pending_report")
    {
        return None;
    }
    let mut supported_task_types = vec!["capability_v1".to_string()];
    if !nni_work_allowed_programs().is_empty() {
        supported_task_types.insert(0, "command_v1".to_string());
    }
    let active_lease = local.as_ref().and_then(|value| {
        matches!(value.status.as_str(), "assigned" | "running").then(|| NniActiveWorkLease {
            task_id: value.assignment.task_id.clone(),
            attempt: value.assignment.attempt,
            lease_token_sha256: nni_work_sha256(&value.assignment.lease_token),
        })
    });
    Some(NniWorkerCapabilities {
        protocol_version: NNI_WORK_PROTOCOL_VERSION,
        platform: nni_work_platform()?.to_string(),
        arch: nni_work_arch()?.to_string(),
        supported_task_types,
        worker_instance_id: nni_worker_instance_id(state).ok()?,
        active_lease,
    })
}

fn nni_verify_work_assignment_envelope(
    envelope: &NniWorkAssignmentEnvelope,
    expected_heartbeat_task_id: &str,
    keys: &BTreeMap<String, k256::ecdsa::VerifyingKey>,
) -> Result<NniWorkAssignment, &'static str> {
    nni_verify_work_signed_envelope(envelope, NNI_WORK_ASSIGNMENT_SIGNATURE_DOMAIN, keys)?;
    let assignment: NniWorkAssignment = serde_json::from_str(&envelope.payload_canonical)
        .map_err(|_| "assignment_payload_invalid")?;
    let now = u64::try_from(current_unix_ts()).unwrap_or_default();
    if assignment.schema_version != NNI_WORK_ASSIGNMENT_SCHEMA_VERSION
        || assignment.heartbeat_task_id != expected_heartbeat_task_id
        || assignment.issued_at_unix > now.saturating_add(60)
        || assignment.issued_at_unix > assignment.lease_expires_at_unix
    {
        return Err("assignment_binding_invalid");
    }
    Ok(assignment)
}

fn nni_verify_work_signed_envelope(
    envelope: &NniWorkAssignmentEnvelope,
    domain: &str,
    keys: &BTreeMap<String, k256::ecdsa::VerifyingKey>,
) -> Result<(), &'static str> {
    use k256::ecdsa::signature::Verifier;

    if envelope.schema_version != NNI_WORK_ASSIGNMENT_SCHEMA_VERSION
        || envelope.algorithm != NNI_WORK_ASSIGNMENT_SIGNATURE_ALGORITHM
        || envelope.signing_key_id.len() != 64
        || !envelope
            .signing_key_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || envelope.signature.len() != 128
        || !envelope.signature.bytes().all(|byte| byte.is_ascii_hexdigit())
        || envelope.payload_canonical.len() > NNI_WORK_SIGNED_PAYLOAD_BYTES
    {
        return Err("assignment_envelope_invalid");
    }
    let key = keys
        .get(&envelope.signing_key_id.to_ascii_lowercase())
        .ok_or("assignment_signing_key_unknown")?;
    let signature_bytes = hex::decode(&envelope.signature)
        .map_err(|_| "assignment_signature_invalid")?;
    let signature = k256::ecdsa::Signature::from_slice(&signature_bytes)
        .map_err(|_| "assignment_signature_invalid")?;
    let signed = format!("{}{}", domain, envelope.payload_canonical);
    key.verify(signed.as_bytes(), &signature)
        .map_err(|_| "assignment_signature_invalid")
}

fn nni_work_control_error_code_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn nni_verify_work_control_envelope(
    envelope: &NniWorkAssignmentEnvelope,
    expected_heartbeat_task_id: &str,
    expected_device_pubkey: &str,
    keys: &BTreeMap<String, k256::ecdsa::VerifyingKey>,
) -> Result<NniWorkControl, &'static str> {
    nni_verify_work_signed_envelope(envelope, NNI_WORK_CONTROL_SIGNATURE_DOMAIN, keys)?;
    let control: NniWorkControl = serde_json::from_str(&envelope.payload_canonical)
        .map_err(|_| "work_control_payload_invalid")?;
    let now = u64::try_from(current_unix_ts()).unwrap_or_default();
    if control.schema_version != NNI_WORK_ASSIGNMENT_SCHEMA_VERSION
        || control.heartbeat_task_id != expected_heartbeat_task_id
        || control.device_pubkey != expected_device_pubkey
        || !matches!(
            control.work_report_status.as_str(),
            "none" | "accepted" | "already_reported" | "error"
        )
        || !matches!(
            control.work_poll_status.as_str(),
            "disabled" | "idle" | "leased" | "error"
        )
        || control
            .work_report_error_code
            .as_deref()
            .is_some_and(|value| !nni_work_control_error_code_valid(value))
        || control
            .work_poll_error_code
            .as_deref()
            .is_some_and(|value| !nni_work_control_error_code_valid(value))
        || control.issued_at_unix > now.saturating_add(60)
    {
        return Err("work_control_binding_invalid");
    }
    Ok(control)
}

fn nni_pending_work_report_for_node(
    state: &AppState,
    node_url: &str,
    device_pubkey: &str,
) -> Option<NniHeartbeatWorkReport> {
    let local = match read_nni_local_work_state(state) {
        Ok(Some(local)) => local,
        Ok(None) => return None,
        Err(error) => {
            append_nni_log_event_best_effort(
                state,
                "distributed_work_state_read_failed",
                json!({"error": error.to_string()}),
            );
            return None;
        }
    };
    if local.status != "completed_pending_report"
        || local.node_url.trim_end_matches('/') != node_url.trim_end_matches('/')
        || local.assignment.target_device_pubkey != device_pubkey
    {
        return None;
    }
    let result = local.result?;
    Some(NniHeartbeatWorkReport {
        task_id: local.assignment.task_id,
        attempt: local.assignment.attempt,
        lease_token: local.assignment.lease_token,
        result,
    })
}

fn nni_validate_work_assignment(
    assignment: &NniWorkAssignment,
) -> Result<NniWorkPayload, &'static str> {
    if assignment.schema_version != NNI_WORK_ASSIGNMENT_SCHEMA_VERSION
        || !matches!(assignment.task_type.as_str(), "command_v1" | "capability_v1")
    {
        return Err("unsupported_assignment");
    }
    if Some(assignment.target_platform.as_str()) != nni_work_platform()
        || Some(assignment.target_arch.as_str()) != nni_work_arch()
    {
        return Err("platform_mismatch");
    }
    if assignment.attempt == 0
        || !assignment.task_id.starts_with("nni-work-")
        || assignment.lease_token.len() != 64
        || !assignment.lease_token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("assignment_identity_invalid");
    }
    if assignment.lease_expires_at_unix
        <= u64::try_from(current_unix_ts()).unwrap_or_default()
    {
        return Err("assignment_lease_expired");
    }
    if nni_work_sha256(&assignment.payload_canonical) != assignment.payload_sha256 {
        return Err("payload_digest_mismatch");
    }
    match assignment.task_type.as_str() {
        "command_v1" => {
            let payload: NniWorkCommandPayload = serde_json::from_str(&assignment.payload_canonical)
                .map_err(|_| "payload_invalid")?;
            if !nni_work_program_name_valid(&payload.program)
                || payload.args.len() > 32
                || payload
                    .args
                    .iter()
                    .any(|value| value.as_bytes().contains(&0) || value.len() > 512)
                || payload.args.iter().map(|value| value.len()).sum::<usize>() > 4 * 1024
                || payload.timeout_seconds == 0
                || payload.timeout_seconds > NNI_WORK_COMMAND_TIMEOUT_MAX_SECONDS
            {
                return Err("payload_invalid");
            }
            Ok(NniWorkPayload::Command(payload))
        }
        "capability_v1" => {
            let payload: NniWorkCapabilityPayload =
                serde_json::from_str(&assignment.payload_canonical)
                    .map_err(|_| "payload_invalid")?;
            let direct = json!({
                "entrypoint": "run_capability",
                "capability": payload.capability,
                "args": payload.args,
            });
            if serde_json::to_vec(&payload.args)
                .map_or(true, |value| value.len() > NNI_WORK_CAPABILITY_ARGS_BYTES)
                || crate::worker::run_capability::parse_direct_capability_request(&direct).is_err()
            {
                return Err("payload_invalid");
            }
            Ok(NniWorkPayload::Capability(payload))
        }
        _ => Err("unsupported_assignment"),
    }
}

fn nni_failed_work_result(error_code: &str, stderr: impl Into<String>) -> NniWorkResult {
    let mut stderr = stderr.into();
    nni_truncate_utf8_bytes(&mut stderr, NNI_WORK_RESULT_STREAM_BYTES);
    NniWorkResult {
        status: "failed".to_string(),
        exit_code: None,
        duration_ms: 0,
        stdout: String::new(),
        stderr,
        stdout_truncated: false,
        stderr_truncated: false,
        error_code: Some(error_code.to_string()),
    }
}

fn nni_truncate_utf8_bytes(value: &mut String, maximum_bytes: usize) {
    if value.len() <= maximum_bytes {
        return;
    }
    let mut end = maximum_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

fn nni_bound_work_result_for_heartbeat(mut result: NniWorkResult) -> NniWorkResult {
    nni_truncate_utf8_bytes(&mut result.stdout, NNI_WORK_RESULT_STREAM_BYTES);
    nni_truncate_utf8_bytes(&mut result.stderr, NNI_WORK_RESULT_STREAM_BYTES);
    while serde_json::to_vec(&result).map_or(usize::MAX, |value| value.len())
        > NNI_WORK_REPORT_JSON_BYTES
    {
        if result.stdout.is_empty() && result.stderr.is_empty() {
            break;
        }
        if result.stdout.len() >= result.stderr.len() && !result.stdout.is_empty() {
            let next = result.stdout.len().saturating_sub(1024);
            nni_truncate_utf8_bytes(&mut result.stdout, next);
            result.stdout_truncated = true;
        } else {
            let next = result.stderr.len().saturating_sub(1024);
            nni_truncate_utf8_bytes(&mut result.stderr, next);
            result.stderr_truncated = true;
        }
    }
    result
}

async fn nni_read_bounded_output<R>(mut reader: R) -> std::io::Result<(String, bool)>
where
    R: AsyncRead + Unpin,
{
    let mut captured = Vec::with_capacity(NNI_WORK_RESULT_STREAM_BYTES);
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = NNI_WORK_RESULT_STREAM_BYTES.saturating_sub(captured.len());
        let keep = count.min(remaining);
        captured.extend_from_slice(&buffer[..keep]);
        if keep < count {
            truncated = true;
        }
    }
    let mut output = String::from_utf8_lossy(&captured).into_owned();
    let lossy_expanded = output.len() > NNI_WORK_RESULT_STREAM_BYTES;
    nni_truncate_utf8_bytes(&mut output, NNI_WORK_RESULT_STREAM_BYTES);
    Ok((output, truncated || lossy_expanded))
}

async fn nni_execute_work_command(
    payload: &NniWorkCommandPayload,
    allowed_programs: &BTreeSet<String>,
    timeout_max_seconds: u64,
) -> NniWorkResult {
    if !allowed_programs.contains(&payload.program) {
        return nni_failed_work_result("program_not_allowed", "program_not_allowed");
    }
    let started = std::time::Instant::now();
    let mut command = Command::new(&payload.program);
    command
        .args(&payload.args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::piped())
        .stderr(StdProcessStdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return nni_failed_work_result("command_spawn_failed", error.to_string());
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_task = tokio::spawn(async move {
        match stdout {
            Some(stream) => nni_read_bounded_output(stream).await,
            None => Ok((String::new(), false)),
        }
    });
    let stderr_task = tokio::spawn(async move {
        match stderr {
            Some(stream) => nni_read_bounded_output(stream).await,
            None => Ok((String::new(), false)),
        }
    });
    let timeout_seconds = payload.timeout_seconds.min(timeout_max_seconds.max(1));
    let (exit_status, timed_out) = match tokio::time::timeout(
        Duration::from_secs(timeout_seconds),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => (Some(status), false),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let (stdout, stdout_truncated) = stdout_task
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            let (stderr, stderr_truncated) = stderr_task
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            return NniWorkResult {
                status: "failed".to_string(),
                exit_code: None,
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                stdout,
                stderr: if stderr.is_empty() { error.to_string() } else { stderr },
                stdout_truncated,
                stderr_truncated,
                error_code: Some("command_wait_failed".to_string()),
            };
        }
        Err(_) => {
            let _ = child.kill().await;
            (child.wait().await.ok(), true)
        }
    };
    let (stdout, stdout_truncated) = stdout_task
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let (stderr, stderr_truncated) = stderr_task
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let exit_code = exit_status.as_ref().and_then(std::process::ExitStatus::code);
    let success = !timed_out && exit_status.as_ref().is_some_and(std::process::ExitStatus::success);
    NniWorkResult {
        status: if success { "success" } else { "failed" }.to_string(),
        exit_code,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        error_code: if success {
            None
        } else if timed_out {
            Some("command_timeout".to_string())
        } else {
            Some("command_exit_nonzero".to_string())
        },
    }
}

fn nni_local_admin_identity(state: &AppState) -> anyhow::Result<AuthIdentity> {
    let user_key = {
        let db = state
            .core
            .db
            .get()
            .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
        db.query_row(
            "SELECT user_key FROM auth_keys
             WHERE role = 'admin' AND enabled = 1 AND principal_id IS NOT NULL
             ORDER BY created_at ASC, rowid ASC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    }
    .ok_or_else(|| anyhow::anyhow!("local_admin_unavailable"))?;
    resolve_auth_identity_by_key(state, &user_key)?
        .filter(|identity| identity.role.eq_ignore_ascii_case("admin"))
        .ok_or_else(|| anyhow::anyhow!("local_admin_unavailable"))
}

fn nni_capability_task_payload(
    identity: &AuthIdentity,
    payload: &NniWorkCapabilityPayload,
) -> anyhow::Result<Value> {
    let mut direct = json!({
        "entrypoint": "run_capability",
        "capability": payload.capability,
        "args": payload.args,
    });
    crate::worker::run_capability::parse_direct_capability_request(&direct)?;
    crate::task_execution_policy::stamp_authenticated_submission_policy(
        &mut direct,
        Some(identity),
        Some("nni-control-plane"),
        None,
    )
    .map_err(|error| anyhow::anyhow!(error.as_token()))?;
    Ok(direct)
}

fn nni_enqueue_capability_task(
    state: &AppState,
    assignment: &NniWorkAssignment,
    payload: &NniWorkCapabilityPayload,
) -> anyhow::Result<String> {
    let identity = nni_local_admin_identity(state)?;
    let direct = nni_capability_task_payload(&identity, payload)?;
    let task_id = uuid::Uuid::new_v4();
    let idempotency_key = format!(
        "nni-capability:{}:{}",
        assignment.task_id, assignment.attempt
    );
    let (persisted_task_id, _) = crate::repo::insert_submitted_task(
        state,
        &task_id,
        identity.user_id,
        identity.chat_id,
        Some(&identity.user_key),
        claw_core::types::ChannelKind::Ui,
        None,
        None,
        None,
        Some(&idempotency_key),
        "ask",
        &direct.to_string(),
    )?;
    Ok(persisted_task_id.to_string())
}

fn nni_capability_result_text(result: Option<&Value>) -> String {
    let Some(result) = result else {
        return String::new();
    };
    let text = [
        result.pointer("/text"),
        result.pointer("/final_result_json/text"),
        result.pointer("/task_journal/summary/final_answer"),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| value.as_str().map(str::trim).filter(|value| !value.is_empty()));
    let raw = text
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(result).unwrap_or_default());
    crate::visible_text::sanitize_user_visible_text(&raw)
}

fn nni_elapsed_work_ms(started_at_unix: Option<i64>) -> u64 {
    let started = started_at_unix.unwrap_or_else(current_unix_ts);
    u64::try_from(current_unix_ts().saturating_sub(started))
        .unwrap_or_default()
        .saturating_mul(1_000)
}

async fn nni_wait_for_capability_task(
    state: &AppState,
    local_task_id: &str,
    started_at_unix: Option<i64>,
) -> NniWorkResult {
    let task_id = match uuid::Uuid::parse_str(local_task_id) {
        Ok(value) => value,
        Err(_) => {
            return nni_failed_work_result("local_task_id_invalid", "local_task_id_invalid")
        }
    };
    loop {
        let task = match crate::repo::get_task_query_record(state, task_id) {
            Ok(Some((task, _, _))) => task,
            Ok(None) => {
                return nni_failed_work_result("local_task_not_found", "local_task_not_found")
            }
            Err(error) => {
                return nni_failed_work_result("local_task_read_failed", error.to_string())
            }
        };
        match task.status {
            claw_core::types::TaskStatus::Succeeded => {
                return NniWorkResult {
                    status: "success".to_string(),
                    exit_code: None,
                    duration_ms: nni_elapsed_work_ms(started_at_unix),
                    stdout: nni_capability_result_text(task.result_json.as_ref()),
                    stderr: String::new(),
                    stdout_truncated: false,
                    stderr_truncated: false,
                    error_code: None,
                };
            }
            claw_core::types::TaskStatus::Failed
            | claw_core::types::TaskStatus::Timeout
            | claw_core::types::TaskStatus::Canceled => {
                let error_code = match task.status {
                    claw_core::types::TaskStatus::Timeout => "local_task_timeout",
                    claw_core::types::TaskStatus::Canceled => "local_task_cancelled",
                    _ => "capability_execution_failed",
                };
                let detail = task
                    .error_text
                    .as_deref()
                    .map(crate::visible_text::sanitize_user_visible_text)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| nni_capability_result_text(task.result_json.as_ref()));
                let mut result = nni_failed_work_result(error_code, detail);
                result.duration_ms = nni_elapsed_work_ms(started_at_unix);
                return result;
            }
            claw_core::types::TaskStatus::Queued | claw_core::types::TaskStatus::Running => {
                if matches!(
                    task.execution_state,
                    Some(
                        claw_core::types::TaskExecutionState::NeedsConfirmation
                            | claw_core::types::TaskExecutionState::Blocked
                    )
                ) {
                    let _ = crate::repo::cancel_task_by_id(state, local_task_id);
                    let mut result = nni_failed_work_result(
                        "capability_confirmation_required",
                        "capability_confirmation_required",
                    );
                    result.duration_ms = nni_elapsed_work_ms(started_at_unix);
                    return result;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn nni_accept_work_assignment(state: &AppState, heartbeat_data: &Value) {
    let Some(raw_assignment) = heartbeat_data.get("work_assignment") else {
        return;
    };
    if raw_assignment.is_null() {
        return;
    }
    let envelope: NniWorkAssignmentEnvelope = match serde_json::from_value(raw_assignment.clone()) {
        Ok(value) => value,
        Err(error) => {
            append_nni_log_event_best_effort(
                state,
                "distributed_work_assignment_invalid",
                json!({"error_code": "assignment_decode_failed", "error": error.to_string()}),
            );
            return;
        }
    };
    let heartbeat_task_id = heartbeat_data
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let keys = match nni_work_assignment_verifying_keys() {
        Ok(value) => value,
        Err(error_code) => {
            append_nni_log_event_best_effort(
                state,
                "distributed_work_assignment_invalid",
                json!({"error_code": error_code}),
            );
            return;
        }
    };
    let assignment = match nni_verify_work_assignment_envelope(
        &envelope,
        heartbeat_task_id,
        &keys,
    ) {
        Ok(value) => value,
        Err(error_code) => {
            append_nni_log_event_best_effort(
                state,
                "distributed_work_assignment_invalid",
                json!({"error_code": error_code}),
            );
            return;
        }
    };
    let local_device_pubkey = heartbeat_data
        .get("local_device_pubkey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if assignment.target_device_pubkey != local_device_pubkey {
        append_nni_log_event_best_effort(
            state,
            "distributed_work_assignment_invalid",
            json!({
                "task_id": assignment.task_id,
                "attempt": assignment.attempt,
                "error_code": "device_mismatch",
            }),
        );
        return;
    }
    let Some(node_url) = heartbeat_data.get("node_url").and_then(Value::as_str) else {
        return;
    };
    if read_nni_local_work_state(state).ok().flatten().is_some() {
        append_nni_log_event_best_effort(
            state,
            "distributed_work_assignment_busy",
            json!({"task_id": assignment.task_id, "attempt": assignment.attempt}),
        );
        return;
    }
    let local = NniLocalWorkState {
        schema_version: NNI_WORK_STATE_SCHEMA_VERSION,
        node_url: node_url.to_string(),
        status: "assigned".to_string(),
        assignment,
        local_task_id: None,
        started_at_unix: None,
        result: None,
    };
    if let Err(error) = write_nni_local_work_state(state, &local) {
        append_nni_log_event_best_effort(
            state,
            "distributed_work_state_write_failed",
            json!({"error": error.to_string()}),
        );
        return;
    }
    nni_spawn_work_worker(state.clone(), false);
}

fn nni_spawn_work_worker(state: AppState, resume: bool) {
    tokio::spawn(async move {
        let _guard = nni_work_execution_lock().lock().await;
        if let Err(error) = nni_process_work_state(&state, resume).await {
            append_nni_log_event_best_effort(
                &state,
                "distributed_work_worker_failed",
                json!({"error": error.to_string()}),
            );
        }
    });
}

async fn nni_process_work_state(state: &AppState, resume: bool) -> anyhow::Result<()> {
    let Some(mut local) = read_nni_local_work_state(state)? else {
        return Ok(());
    };
    if local.status == "running"
        && resume
        && local.assignment.task_type == "command_v1"
    {
        local.status = "completed_pending_report".to_string();
        local.result = Some(nni_failed_work_result("worker_restarted", "worker_restarted"));
        write_nni_local_work_state(state, &local)?;
    }
    if local.status == "assigned" {
        let validation = nni_validate_work_assignment(&local.assignment);
        local.status = "running".to_string();
        local.started_at_unix = Some(current_unix_ts());
        write_nni_local_work_state(state, &local)?;
        let result = match validation {
            Ok(NniWorkPayload::Command(payload)) => {
                nni_execute_work_command(
                    &payload,
                    &nni_work_allowed_programs(),
                    nni_work_command_timeout_max_seconds(),
                )
                .await
            }
            Ok(NniWorkPayload::Capability(payload)) => {
                match nni_enqueue_capability_task(state, &local.assignment, &payload) {
                    Ok(local_task_id) => {
                        local.local_task_id = Some(local_task_id.clone());
                        write_nni_local_work_state(state, &local)?;
                        nni_wait_for_capability_task(
                            state,
                            &local_task_id,
                            local.started_at_unix,
                        )
                        .await
                    }
                    Err(error) => nni_failed_work_result(
                        "capability_task_enqueue_failed",
                        error.to_string(),
                    ),
                }
            }
            Err(error_code) => nni_failed_work_result(error_code, error_code),
        };
        local.status = "completed_pending_report".to_string();
        local.result = Some(nni_bound_work_result_for_heartbeat(result));
        write_nni_local_work_state(state, &local)?;
    } else if local.status == "running" && local.assignment.task_type == "capability_v1" {
        let result = match local.local_task_id.as_deref() {
            Some(local_task_id) => {
                nni_wait_for_capability_task(state, local_task_id, local.started_at_unix).await
            }
            None => nni_failed_work_result("local_task_id_missing", "local_task_id_missing"),
        };
        local.status = "completed_pending_report".to_string();
        local.result = Some(nni_bound_work_result_for_heartbeat(result));
        write_nni_local_work_state(state, &local)?;
    }
    Ok(())
}

fn nni_work_report_error_is_terminal(error_code: &str) -> bool {
    matches!(
        error_code,
        "nni_work_task_not_found"
            | "nni_work_lease_not_active"
            | "nni_work_lease_expired"
            | "nni_work_lease_token_invalid"
            | "nni_work_lease_token_mismatch"
            | "nni_work_report_invalid"
            | "nni_work_report_field_invalid"
            | "nni_work_result_invalid"
            | "nni_work_result_field_invalid"
    ) || error_code.starts_with("nni_work_result_")
}

fn nni_remove_local_work_state(state: &AppState) -> anyhow::Result<()> {
    match fs::remove_file(nni_work_state_path(state)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn nni_handle_work_heartbeat_response(state: &AppState, heartbeat_data: &Value) {
    let node_url = heartbeat_data
        .get("node_url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let local = match read_nni_local_work_state(state) {
        Ok(value) => value,
        Err(error) => {
            append_nni_log_event_best_effort(
                state,
                "distributed_work_state_read_failed",
                json!({"error": error.to_string()}),
            );
            return;
        }
    };
    let expected_heartbeat_task_id = heartbeat_data
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let expected_device_pubkey = heartbeat_data
        .get("local_device_pubkey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(raw_control) = heartbeat_data.get("work_control") else {
        if local.is_some() || nni_work_pull_enabled() {
            append_nni_log_event_best_effort(
                state,
                "distributed_work_control_invalid",
                json!({"error_code": "work_control_missing"}),
            );
        }
        return;
    };
    let envelope: NniWorkAssignmentEnvelope = match serde_json::from_value(raw_control.clone()) {
        Ok(value) => value,
        Err(error) => {
            append_nni_log_event_best_effort(
                state,
                "distributed_work_control_invalid",
                json!({"error_code": "work_control_decode_failed", "error": error.to_string()}),
            );
            return;
        }
    };
    let keys = match nni_work_assignment_verifying_keys() {
        Ok(value) => value,
        Err(error_code) => {
            append_nni_log_event_best_effort(
                state,
                "distributed_work_control_invalid",
                json!({"error_code": error_code}),
            );
            return;
        }
    };
    let control = match nni_verify_work_control_envelope(
        &envelope,
        expected_heartbeat_task_id,
        expected_device_pubkey,
        &keys,
    ) {
        Ok(value) => value,
        Err(error_code) => {
            append_nni_log_event_best_effort(
                state,
                "distributed_work_control_invalid",
                json!({"error_code": error_code}),
            );
            return;
        }
    };
    if let Some(local) = local {
        if local.status == "completed_pending_report"
            && local.node_url.trim_end_matches('/') == node_url.trim_end_matches('/')
        {
            let report_status = control.work_report_status.as_str();
            let report_error_code = control.work_report_error_code.as_deref();
            let accepted = matches!(report_status, "accepted" | "already_reported");
            let terminal_rejection = report_status == "error"
                && report_error_code.is_some_and(nni_work_report_error_is_terminal);
            if accepted || terminal_rejection {
                if let Err(error) = append_nni_work_audit(state, &local)
                    .and_then(|()| nni_remove_local_work_state(state))
                {
                    append_nni_log_event_best_effort(
                        state,
                        "distributed_work_report_finalize_failed",
                        json!({"error": error.to_string()}),
                    );
                    return;
                }
                append_nni_log_event_best_effort(
                    state,
                    if accepted {
                        "distributed_work_reported"
                    } else {
                        "distributed_work_report_rejected"
                    },
                    json!({
                        "task_id": local.assignment.task_id,
                        "attempt": local.assignment.attempt,
                        "result_status": local.result.as_ref().map(|value| value.status.as_str()),
                        "report_error_code": report_error_code,
                    }),
                );
            } else if report_status == "error" {
                append_nni_log_event_best_effort(
                    state,
                    "distributed_work_report_deferred",
                    json!({
                        "task_id": local.assignment.task_id,
                        "attempt": local.assignment.attempt,
                        "error_code": report_error_code,
                    }),
                );
            }
        }
    }
    let trusted = json!({
        "task_id": control.heartbeat_task_id,
        "node_url": node_url,
        "local_device_pubkey": control.device_pubkey,
        "work_assignment": control.work_assignment,
    });
    nni_accept_work_assignment(state, &trusted);
}

fn nni_resume_work_worker(state: AppState) {
    match read_nni_local_work_state(&state) {
        Ok(Some(local)) if matches!(local.status.as_str(), "assigned" | "running") => {
            nni_spawn_work_worker(state, true);
        }
        Ok(_) => {}
        Err(error) => append_nni_log_event_best_effort(
            &state,
            "distributed_work_state_read_failed",
            json!({"error": error.to_string()}),
        ),
    }
}

#[cfg(test)]
mod nni_distributed_work_unit_tests {
    use super::*;
    use k256::ecdsa::{signature::Signer, Signature, SigningKey};
    use k256::elliptic_curve::rand_core::OsRng;

    fn assignment(task_type: &str, payload: Value) -> NniWorkAssignment {
        let payload_canonical = serde_json::to_string(&payload).unwrap();
        let now = u64::try_from(current_unix_ts()).unwrap();
        NniWorkAssignment {
            schema_version: NNI_WORK_ASSIGNMENT_SCHEMA_VERSION,
            heartbeat_task_id: "nni-heartbeat-0123456789abcdef0123456789abcdef".to_string(),
            task_id: "nni-work-0123456789abcdef0123456789abcdef".to_string(),
            task_type: task_type.to_string(),
            target_device_pubkey: "a".repeat(128),
            target_platform: nni_work_platform().unwrap().to_string(),
            target_arch: nni_work_arch().unwrap().to_string(),
            attempt: 1,
            lease_token: "b".repeat(64),
            lease_expires_at_unix: now + 60,
            payload_sha256: nni_work_sha256(&payload_canonical),
            payload_canonical,
            issued_at_unix: now,
        }
    }

    #[test]
    fn program_validation_rejects_shells_and_paths() {
        assert!(nni_work_program_name_valid("uname"));
        assert!(nni_work_program_name_valid("printf"));
        assert!(!nni_work_program_name_valid("sh"));
        assert!(!nni_work_program_name_valid("/usr/bin/uname"));
        assert!(!nni_work_program_name_valid("uname;id"));
    }

    #[test]
    fn heartbeat_work_challenge_binds_the_exact_outbound_metadata() {
        use sha2::{Digest, Sha256};

        let capabilities = NniWorkerCapabilities {
            protocol_version: 1,
            platform: "linux".to_string(),
            arch: "x86_64".to_string(),
            supported_task_types: vec!["capability_v1".to_string()],
            worker_instance_id: "11111111-2222-4333-8444-555555555555".to_string(),
            active_lease: None,
        };
        let report = NniHeartbeatWorkReport {
            task_id: "nni-work-0123456789abcdef0123456789abcdef".to_string(),
            attempt: 1,
            lease_token: "b".repeat(64),
            result: NniWorkResult {
                status: "success".to_string(),
                exit_code: Some(0),
                duration_ms: 42,
                stdout: "ok".to_string(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                error_code: None,
            },
        };
        let nonce = "a".repeat(64);
        let capabilities_json = serde_json::to_string(&capabilities).unwrap();
        let report_json = serde_json::to_string(&report).unwrap();
        let challenge = hex::encode(Sha256::digest(
            format!(
                "{NNI_WORK_HEARTBEAT_CHALLENGE_DOMAIN}{nonce}\0{}\0{}",
                hex::encode(Sha256::digest(capabilities_json.as_bytes())),
                hex::encode(Sha256::digest(report_json.as_bytes())),
            )
            .as_bytes(),
        ));
        assert_eq!(
            challenge,
            "885c3bea19854a75640ebb51781bd81ff67d9f9ea874129d37227f7bd75698ba"
        );

        assert_eq!(
            nni_verify_heartbeat_work_challenge(
                &challenge,
                Some(&nonce),
                Some(&capabilities),
                Some(&report),
            ),
            Ok(())
        );
        let mut changed = report.clone();
        changed.result.stdout = "forged".to_string();
        assert_eq!(
            nni_verify_heartbeat_work_challenge(
                &challenge,
                Some(&nonce),
                Some(&capabilities),
                Some(&changed),
            ),
            Err("heartbeat_work_challenge_binding_invalid")
        );
        assert_eq!(
            nni_verify_heartbeat_work_challenge(
                &challenge,
                None,
                Some(&capabilities),
                Some(&report),
            ),
            Err("heartbeat_work_challenge_nonce_missing")
        );
    }

    #[test]
    fn signed_assignment_rejects_tampering_unknown_keys_and_heartbeat_replay() {
        use sha2::{Digest, Sha256};

        let signing_key = SigningKey::random(&mut OsRng);
        let public_key = signing_key.verifying_key().to_encoded_point(true);
        let public_key_hex = hex::encode(public_key.as_bytes());
        let keys = nni_parse_work_assignment_verifying_keys(&public_key_hex)
            .expect("parse verifying key");
        let key_id = hex::encode(Sha256::digest(public_key.as_bytes()));
        let expected_heartbeat = "nni-heartbeat-0123456789abcdef0123456789abcdef";
        let work = assignment("command_v1", json!({
            "program": "uname",
            "args": ["-a"],
            "timeout_seconds": 30,
        }));
        let payload_canonical = serde_json::to_string(&work).expect("serialize assignment");
        let signed = format!(
            "{}{}",
            NNI_WORK_ASSIGNMENT_SIGNATURE_DOMAIN, payload_canonical
        );
        let signature: Signature = signing_key.sign(signed.as_bytes());
        let envelope = NniWorkAssignmentEnvelope {
            schema_version: NNI_WORK_ASSIGNMENT_SCHEMA_VERSION,
            algorithm: NNI_WORK_ASSIGNMENT_SIGNATURE_ALGORITHM.to_string(),
            signing_key_id: key_id,
            payload_canonical,
            signature: hex::encode(signature.to_bytes()),
        };

        let verified = nni_verify_work_assignment_envelope(
            &envelope,
            expected_heartbeat,
            &keys,
        )
        .expect("verify signed assignment");
        assert_eq!(verified.task_id, work.task_id);

        let mut tampered = envelope.clone();
        tampered.payload_canonical = tampered.payload_canonical.replace("uname", "false");
        assert_eq!(
            nni_verify_work_assignment_envelope(&tampered, expected_heartbeat, &keys)
                .expect_err("tampering must fail"),
            "assignment_signature_invalid"
        );
        assert_eq!(
            nni_verify_work_assignment_envelope(
                &envelope,
                "nni-heartbeat-ffffffffffffffffffffffffffffffff",
                &keys,
            )
            .expect_err("heartbeat replay must fail"),
            "assignment_binding_invalid"
        );

        let other_key = SigningKey::random(&mut OsRng);
        let other_hex = hex::encode(other_key.verifying_key().to_encoded_point(true).as_bytes());
        let other_keys = nni_parse_work_assignment_verifying_keys(&other_hex)
            .expect("parse other verifying key");
        assert_eq!(
            nni_verify_work_assignment_envelope(&envelope, expected_heartbeat, &other_keys)
                .expect_err("unknown key must fail"),
            "assignment_signing_key_unknown"
        );

        let control_payload = serde_json::to_string(&NniWorkControl {
            schema_version: NNI_WORK_ASSIGNMENT_SCHEMA_VERSION,
            heartbeat_task_id: expected_heartbeat.to_string(),
            device_pubkey: "a".repeat(128),
            work_report_status: "accepted".to_string(),
            work_report_error_code: None,
            work_poll_status: "leased".to_string(),
            work_poll_error_code: None,
            work_assignment: Some(envelope.clone()),
            issued_at_unix: u64::try_from(current_unix_ts()).unwrap(),
        })
        .expect("serialize work control");
        let control_signature: Signature = signing_key.sign(
            format!("{}{}", NNI_WORK_CONTROL_SIGNATURE_DOMAIN, control_payload).as_bytes(),
        );
        let control_envelope = NniWorkAssignmentEnvelope {
            schema_version: NNI_WORK_ASSIGNMENT_SCHEMA_VERSION,
            algorithm: NNI_WORK_ASSIGNMENT_SIGNATURE_ALGORITHM.to_string(),
            signing_key_id: envelope.signing_key_id.clone(),
            payload_canonical: control_payload,
            signature: hex::encode(control_signature.to_bytes()),
        };
        let control = nni_verify_work_control_envelope(
            &control_envelope,
            expected_heartbeat,
            &"a".repeat(128),
            &keys,
        )
        .expect("verify signed work control");
        assert_eq!(control.work_report_status, "accepted");
        assert!(nni_verify_work_assignment_envelope(
            &control_envelope,
            expected_heartbeat,
            &keys,
        )
        .is_err());
        assert_eq!(
            nni_verify_work_control_envelope(
                &control_envelope,
                "nni-heartbeat-ffffffffffffffffffffffffffffffff",
                &"a".repeat(128),
                &keys,
            )
            .expect_err("work-control replay must fail"),
            "work_control_binding_invalid"
        );
    }

    #[test]
    fn verifies_node_crypto_assignment_signature_fixture() {
        let keys = nni_parse_work_assignment_verifying_keys(
            "0216451a0615c57cdb9caddc0b90a2acf727340d6ddced7e305331627c0d10cc46",
        )
        .expect("parse Node.js fixture key");
        let envelope = NniWorkAssignmentEnvelope {
            schema_version: 2,
            algorithm: "secp256k1-sha256".to_string(),
            signing_key_id:
                "fa7f8c2e1ed9da5d4b2ce637e54d87a5e6a78f60c1c6912875c272a8d0eac341"
                    .to_string(),
            payload_canonical: r#"{"schema_version":2,"heartbeat_task_id":"nni-heartbeat-11111111111111111111111111111111","task_id":"nni-work-22222222222222222222222222222222","task_type":"command_v1","target_device_pubkey":"33333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333","target_platform":"linux","target_arch":"aarch64","attempt":1,"lease_token":"4444444444444444444444444444444444444444444444444444444444444444","lease_expires_at_unix":4102444800,"payload_canonical":"{\"program\":\"uname\",\"args\":[\"-a\"],\"timeout_seconds\":30}","payload_sha256":"5555555555555555555555555555555555555555555555555555555555555555","issued_at_unix":1}"#.to_string(),
            signature:
                "aafacd24156612e1e738b8510667261d4b65a31d2cd2f68b5872d4a5852386115ebe590153788c744818c37895e744f5678b454df53fa963c864e9157f793538"
                    .to_string(),
        };
        let verified = nni_verify_work_assignment_envelope(
            &envelope,
            "nni-heartbeat-11111111111111111111111111111111",
            &keys,
        )
        .expect("verify Node.js signature in Rust");
        assert_eq!(verified.task_id, "nni-work-22222222222222222222222222222222");
    }

    #[test]
    fn capability_assignment_uses_the_existing_direct_capability_contract() {
        let parsed = nni_validate_work_assignment(&assignment(
            "capability_v1",
            json!({
                "capability": "web.search",
                "args": {"query": "distributed inference"},
            }),
        ))
        .expect("valid capability assignment");
        match parsed {
            NniWorkPayload::Capability(payload) => {
                assert_eq!(payload.capability, "web.search");
                assert_eq!(payload.args["query"], "distributed inference");
            }
            NniWorkPayload::Command(_) => panic!("expected capability payload"),
        }
        assert!(nni_validate_work_assignment(&assignment(
            "capability_v1",
            json!({"capability": "Browser Web", "args": {}}),
        ))
        .is_err());
    }

    #[test]
    fn capability_assignment_enters_the_authenticated_local_task_queue() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        state.seed_test_auth_identity("rk-nni-capability-admin", "admin");
        let assignment = assignment(
            "capability_v1",
            json!({
                "capability": "web.search",
                "args": {"query": "distributed inference"},
            }),
        );
        let payload = match nni_validate_work_assignment(&assignment)
            .expect("valid capability assignment")
        {
            NniWorkPayload::Capability(payload) => payload,
            NniWorkPayload::Command(_) => panic!("expected capability payload"),
        };

        let local_task_id = nni_enqueue_capability_task(&state, &assignment, &payload)
            .expect("enqueue local capability task");
        let (payload_json, status, user_key, principal_id): (String, String, String, String) = state
            .core
            .db
            .get()
            .expect("get test database")
            .query_row(
                "SELECT payload_json, status, user_key, principal_id FROM tasks WHERE task_id = ?1",
                [&local_task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("read queued local capability task");
        let queued: Value = serde_json::from_str(&payload_json).expect("parse queued payload");

        assert_eq!(status, "queued");
        assert_eq!(user_key, "rk-nni-capability-admin");
        assert!(!principal_id.is_empty());
        assert_eq!(queued["entrypoint"], "run_capability");
        assert_eq!(queued["capability"], "web.search");
        assert_eq!(queued["args"]["query"], "distributed inference");
        assert_eq!(queued["_agent_execution_policy"]["mode"], "yolo");
        assert_eq!(
            queued["_agent_execution_policy"]["authority"],
            "authenticated_admin"
        );
    }

    #[test]
    fn capability_result_prefers_readable_text_and_redacts_secrets() {
        let text = nni_capability_result_text(Some(&json!({
            "text": "done api_key=secret-value",
            "extra": {"internal": true},
        })));
        assert!(text.starts_with("done"));
        assert!(!text.contains("secret-value"));
    }

    #[tokio::test]
    async fn command_executor_captures_success_without_a_shell() {
        let result = nni_execute_work_command(
            &NniWorkCommandPayload {
                program: "printf".to_string(),
                args: vec!["heartbeat-work-ok".to_string()],
                timeout_seconds: 5,
            },
            &BTreeSet::from(["printf".to_string()]),
            5,
        )
        .await;
        assert_eq!(result.status, "success");
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "heartbeat-work-ok");
        assert!(result.error_code.is_none());
    }

    #[tokio::test]
    async fn command_executor_timeout_is_a_work_failure() {
        let result = nni_execute_work_command(
            &NniWorkCommandPayload {
                program: "sleep".to_string(),
                args: vec!["2".to_string()],
                timeout_seconds: 1,
            },
            &BTreeSet::from(["sleep".to_string()]),
            1,
        )
        .await;
        assert_eq!(result.status, "failed");
        assert_eq!(result.error_code.as_deref(), Some("command_timeout"));
    }

    #[test]
    fn report_json_is_bounded_even_when_every_byte_requires_escaping() {
        let result = nni_bound_work_result_for_heartbeat(NniWorkResult {
            status: "success".to_string(),
            exit_code: Some(0),
            duration_ms: 1,
            stdout: "\\\"".repeat(NNI_WORK_RESULT_STREAM_BYTES / 2),
            stderr: "\\\"".repeat(NNI_WORK_RESULT_STREAM_BYTES / 2),
            stdout_truncated: false,
            stderr_truncated: false,
            error_code: None,
        });
        assert!(serde_json::to_vec(&result).unwrap().len() <= NNI_WORK_REPORT_JSON_BYTES);
        assert!(result.stdout_truncated || result.stderr_truncated);
    }
}
