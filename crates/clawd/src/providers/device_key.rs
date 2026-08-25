use std::{path::PathBuf, sync::OnceLock, time::Duration};

use serde_json::{json, Value};
use tokio::{process::Command, sync::Mutex, time::timeout};

use crate::LlmProviderRuntime;

static ENROLLMENT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const HOSTED_RELAY_SECRET_VENDOR: &str = "hosted_relay";

pub(super) async fn resolve_provider_api_key(
    provider: &LlmProviderRuntime,
) -> Result<String, &'static str> {
    if !provider.config.params.device_key_enrollment {
        return Ok(provider.api_key().trim().to_string());
    }
    if let Some(configured) = stored_device_key() {
        return Ok(configured);
    }
    let lock = ENROLLMENT_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().await;
    if let Some(configured) = stored_device_key() {
        return Ok(configured);
    }
    enroll_device_key(provider).await
}

pub(super) async fn refresh_provider_api_key(
    provider: &LlmProviderRuntime,
) -> Result<String, &'static str> {
    if !provider.config.params.device_key_enrollment {
        return Ok(provider.api_key().trim().to_string());
    }
    let lock = ENROLLMENT_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().await;
    enroll_device_key(provider).await
}

fn stored_device_key() -> Option<String> {
    let secret_name = claw_core::secrets::text_secret_name_for_vendor(HOSTED_RELAY_SECRET_VENDOR);
    claw_core::secrets::global_or_default()
        .lookup(&secret_name)
        .ok()
        .flatten()
        .map(|secret| secret.expose().trim().to_string())
        .filter(|value| value.starts_with("lrk_") && value.len() >= 80)
}

async fn enroll_device_key(provider: &LlmProviderRuntime) -> Result<String, &'static str> {
    let public_key_payload = run_signature_helper(&["pubkey"]).await?;
    let device_pubkey = public_key_payload
        .get("pubkey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| value.len() == 128 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("device_public_key_invalid")?
        .to_ascii_lowercase();
    let request_url = format!(
        "{}/device-key/request",
        provider.config.base_url.trim_end_matches('/')
    );
    let challenge_response = provider
        .client
        .post(request_url)
        .json(&json!({"device_pubkey": device_pubkey}))
        .send()
        .await
        .map_err(|_| "device_key_request_failed")?;
    if !challenge_response.status().is_success() {
        return Err("device_not_allowlisted");
    }
    let challenge_payload: Value = challenge_response
        .json()
        .await
        .map_err(|_| "device_key_challenge_invalid")?;
    let challenge = challenge_payload
        .get("data")
        .ok_or("device_key_challenge_invalid")?;
    let challenge_id = challenge
        .get("challenge_id")
        .and_then(Value::as_str)
        .ok_or("device_key_challenge_invalid")?;
    let challenge_text = challenge
        .get("challenge")
        .and_then(Value::as_str)
        .ok_or("device_key_challenge_invalid")?;
    if challenge.get("device_pubkey").and_then(Value::as_str) != Some(device_pubkey.as_str()) {
        return Err("device_key_challenge_mismatch");
    }
    let signature_payload = run_signature_helper(&["sign_challenge", challenge_text]).await?;
    let signature = signature_payload
        .get("signature")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| value.len() == 128 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("device_signature_invalid")?;
    let verify_url = format!(
        "{}/device-key/verify",
        provider.config.base_url.trim_end_matches('/')
    );
    let verify_response = provider
        .client
        .post(verify_url)
        .json(&json!({
            "device_pubkey": device_pubkey,
            "challenge_id": challenge_id,
            "signature": signature,
        }))
        .send()
        .await
        .map_err(|_| "device_key_verify_failed")?;
    if !verify_response.status().is_success() {
        return Err("device_key_verify_rejected");
    }
    let issued_payload: Value = verify_response
        .json()
        .await
        .map_err(|_| "device_key_response_invalid")?;
    let issued = issued_payload
        .get("data")
        .ok_or("device_key_response_invalid")?;
    if issued.get("device_pubkey").and_then(Value::as_str) != Some(device_pubkey.as_str()) {
        return Err("device_key_response_mismatch");
    }
    let token = issued
        .get("token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| value.starts_with("lrk_") && value.len() >= 80)
        .ok_or("device_key_response_invalid")?
        .to_string();
    let workspace_root = std::env::current_dir().map_err(|_| "workspace_root_unavailable")?;
    let credential_path = claw_core::git_remote_config::git_credential_store_path(&workspace_root);
    let secret_name = claw_core::secrets::text_secret_name_for_vendor(HOSTED_RELAY_SECRET_VENDOR);
    claw_core::secrets::set_file_secret(&credential_path, &secret_name, &token)
        .map_err(|_| "device_key_private_store_write_failed")?;
    Ok(token)
}

async fn run_signature_helper(arguments: &[&str]) -> Result<Value, &'static str> {
    let helper = signature_helper_path()?;
    if !helper.is_file() {
        return Err("device_signature_helper_unavailable");
    }
    let python = std::env::var("CRYPTOAUTHLIB_PYTHON")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "python3".to_string());
    let mut command = Command::new(python);
    command.arg(helper).args(arguments);
    command.kill_on_drop(true);
    let output = timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| "device_signature_helper_timeout")?
        .map_err(|_| "device_signature_helper_failed")?;
    if !output.status.success() {
        return Err("device_signature_helper_failed");
    }
    let payload: Value =
        serde_json::from_slice(&output.stdout).map_err(|_| "device_signature_helper_invalid")?;
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("device_signature_helper_rejected");
    }
    Ok(payload)
}

fn signature_helper_path() -> Result<PathBuf, &'static str> {
    if let Some(path) = std::env::var("APP_DEVICE_SIGNATURE_HELPER_PATH")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("device_signature_helper_path_invalid");
        }
        return Ok(path);
    }
    Ok(std::env::current_dir()
        .map_err(|_| "workspace_root_unavailable")?
        .join("pi_app")
        .join("signature.py"))
}
