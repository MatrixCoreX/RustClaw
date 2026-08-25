use chrono::Utc;
use p256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use crate::{
    config::{ModelProvider, RelayConfig, StoreConfig},
    openai::ChatCompletionRequest,
    quota::QuotaLimits,
    rewrite_sse_line,
    store::{IssuedKey, RelayStore, StoreError},
};

const TEST_PEPPER: &str = "test-only-pepper-with-at-least-thirty-two-bytes";

fn enroll_device(
    store: &RelayStore,
    label: &str,
    daily_request_limit: u32,
    private_key_byte: u8,
) -> IssuedKey {
    let private_key = [private_key_byte; 32];
    let signing_key = SigningKey::from_slice(&private_key).expect("signing key");
    let encoded_point = signing_key.verifying_key().to_encoded_point(false);
    let device_pubkey = hex::encode(&encoded_point.as_bytes()[1..]);
    store
        .allow_device(label, &device_pubkey, daily_request_limit)
        .expect("allow device");
    let challenge = store
        .create_enrollment_challenge(&device_pubkey)
        .expect("create challenge");
    let digest = Sha256::digest(challenge.challenge.as_bytes());
    let signature: Signature = signing_key.sign_prehash(&digest).expect("sign challenge");
    assert!(crate::device_proof::verify_enrollment_signature(
        &device_pubkey,
        &challenge.challenge,
        &hex::encode(signature.to_bytes()),
    ));
    store
        .complete_enrollment(&challenge.challenge_id, &device_pubkey)
        .expect("complete enrollment")
}

#[test]
fn issued_key_authenticates_without_exposing_secret_in_key_list() {
    let directory = tempdir().expect("temp directory");
    let store = RelayStore::open(&directory.path().join("relay.db"), TEST_PEPPER).expect("store");
    let issued = enroll_device(&store, "device-a", 100, 1);
    let authenticated = store.authenticate(&issued.token).expect("authenticate");
    assert_eq!(authenticated.key_id, issued.key_id);
    assert_eq!(authenticated.label, "device-a");
    assert_eq!(authenticated.device_pubkey, issued.device_pubkey);

    let serialized = serde_json::to_string(&store.list_keys().expect("list keys")).expect("json");
    assert!(!serialized.contains(&issued.token));
    assert!(!serialized.contains(issued.token.rsplit('_').next().expect("secret")));
}

#[test]
fn revoked_key_is_rejected() {
    let directory = tempdir().expect("temp directory");
    let store = RelayStore::open(&directory.path().join("relay.db"), TEST_PEPPER).expect("store");
    let issued = enroll_device(&store, "device-b", 100, 2);
    assert!(store
        .revoke_device(issued.device_pubkey.as_deref().expect("device public key"))
        .expect("revoke"));
    assert!(matches!(
        store.authenticate(&issued.token),
        Err(StoreError::KeyDisabled)
    ));
}

#[test]
fn daily_request_limit_is_atomic_and_persistent() {
    let directory = tempdir().expect("temp directory");
    let path = directory.path().join("relay.db");
    let store = RelayStore::open(&path, TEST_PEPPER).expect("store");
    let issued = enroll_device(&store, "device-c", 2, 3);
    let key = store.authenticate(&issued.token).expect("authenticate");
    store
        .reserve_attempt(&key, "request-1", 10, 100, 2)
        .expect("first reservation");
    store
        .settle_attempt("request-1", true, 7)
        .expect("first settlement");
    store
        .reserve_attempt(&key, "request-2", 10, 100, 2)
        .expect("second reservation");
    assert!(matches!(
        store.reserve_attempt(&key, "request-3", 10, 100, 2),
        Err(StoreError::DailyRequestLimit)
    ));
    drop(store);

    let reopened = RelayStore::open_and_recover(&path, TEST_PEPPER).expect("reopen store");
    let key = reopened
        .authenticate(&issued.token)
        .expect("authenticate again");
    let snapshot = reopened.quota_snapshot(&key).expect("quota snapshot");
    assert_eq!(snapshot.request_count, 2);
    assert_eq!(snapshot.remaining_requests, 0);
    assert_eq!(snapshot.total_tokens, 7);
}

#[test]
fn restart_marks_inflight_attempt_failed_without_refunding_request() {
    let directory = tempdir().expect("temp directory");
    let path = directory.path().join("relay.db");
    let store = RelayStore::open(&path, TEST_PEPPER).expect("store");
    let issued = enroll_device(&store, "device-restart", 3, 4);
    let key = store.authenticate(&issued.token).expect("authenticate");
    store
        .reserve_attempt(&key, "interrupted-request", 50, 1_000, 1)
        .expect("reserve attempt");
    drop(store);

    let administrative_store = RelayStore::open(&path, TEST_PEPPER).expect("admin store");
    let administrative_key = administrative_store
        .authenticate(&issued.token)
        .expect("admin authenticate");
    assert_eq!(
        administrative_store
            .quota_snapshot(&administrative_key)
            .expect("admin quota snapshot")
            .failed_requests,
        0,
        "an administrative database open must not settle live attempts"
    );
    drop(administrative_store);

    let reopened = RelayStore::open_and_recover(&path, TEST_PEPPER).expect("reopen store");
    let key = reopened
        .authenticate(&issued.token)
        .expect("authenticate again");
    let snapshot = reopened.quota_snapshot(&key).expect("quota snapshot");
    assert_eq!(snapshot.request_count, 1);
    assert_eq!(snapshot.failed_requests, 1);
    assert_eq!(snapshot.remaining_requests, 2);
    reopened
        .reserve_attempt(&key, "request-after-restart", 1_000, 1_000, 1)
        .expect("released token reservation");
}

#[test]
fn per_key_inflight_limit_is_atomic() {
    let directory = tempdir().expect("temp directory");
    let store = RelayStore::open(&directory.path().join("relay.db"), TEST_PEPPER).expect("store");
    let issued = enroll_device(&store, "device-inflight", 3, 5);
    let key = store.authenticate(&issued.token).expect("authenticate");
    store
        .reserve_attempt(&key, "request-active", 10, 100, 1)
        .expect("first reservation");
    assert!(matches!(
        store.reserve_attempt(&key, "request-blocked", 10, 100, 1),
        Err(StoreError::KeyInflightLimit)
    ));
    store
        .settle_attempt("request-active", true, 7)
        .expect("settle active request");
    store
        .reserve_attempt(&key, "request-after-settle", 10, 100, 1)
        .expect("reservation after settlement");
}

#[test]
fn admin_key_is_isolated_from_client_usage() {
    let directory = tempdir().expect("temp directory");
    let store = RelayStore::open(&directory.path().join("relay.db"), TEST_PEPPER).expect("store");
    let client = enroll_device(&store, "device-client", 100, 6);
    let admin = store
        .issue_admin_key("website-admin")
        .expect("issue admin key");
    let admin_key = store
        .authenticate(&admin.token)
        .expect("authenticate admin");

    assert!(admin_key.require_scope("usage.admin.read").is_ok());
    assert!(admin_key.require_scope("usage.admin.write").is_ok());
    assert!(matches!(
        admin_key.require_scope("chat.completions"),
        Err(StoreError::ScopeDenied)
    ));
    assert_eq!(store.active_key_count().expect("active client count"), 1);

    let page = store
        .admin_usage_page(Utc::now().date_naive(), 1, 50, "all")
        .expect("admin usage page");
    assert_eq!(page.schema_version, 1);
    assert_eq!(page.total, 1);
    assert_eq!(page.devices.len(), 1);
    assert_eq!(page.devices[0].key_id, client.key_id);
    assert_eq!(page.devices[0].device_pubkey, client.device_pubkey.unwrap());
    assert_ne!(page.devices[0].key_id, admin.key_id);
}

#[test]
fn admin_usage_and_daily_limit_update_are_consistent() {
    let directory = tempdir().expect("temp directory");
    let store = RelayStore::open(&directory.path().join("relay.db"), TEST_PEPPER).expect("store");
    let client = enroll_device(&store, "device-usage", 3, 7);
    let other = enroll_device(&store, "device-other", 5, 8);
    let admin = store
        .issue_admin_key("website-admin")
        .expect("issue admin key");
    let key = store
        .authenticate(&client.token)
        .expect("authenticate client");

    store
        .reserve_attempt(&key, "request-success", 10, 100, 1)
        .expect("reserve successful request");
    store
        .settle_attempt("request-success", true, 17)
        .expect("settle successful request");
    store
        .reserve_attempt(&key, "request-failed", 10, 100, 1)
        .expect("reserve failed request");
    store
        .settle_attempt("request-failed", false, 0)
        .expect("settle failed request");

    let first_page = store
        .admin_usage_page(Utc::now().date_naive(), 1, 1, "enabled")
        .expect("first usage page");
    assert_eq!(first_page.total, 2);
    assert_eq!(first_page.total_pages, 2);
    let usage = &first_page.devices[0];
    assert_eq!(usage.key_id, client.key_id);
    assert_eq!(usage.device_pubkey, client.device_pubkey.clone().unwrap());
    assert_eq!(usage.request_count, 2);
    assert_eq!(usage.successful_requests, 1);
    assert_eq!(usage.failed_requests, 1);
    assert_eq!(usage.total_tokens, 17);
    assert_eq!(usage.remaining_requests, 1);

    let update = store
        .update_daily_request_limit(
            &admin.key_id,
            client.device_pubkey.as_deref().expect("device public key"),
            10,
        )
        .expect("update daily limit")
        .expect("client exists");
    assert_eq!(update.schema_version, 1);
    assert_eq!(update.device_pubkey, client.device_pubkey.clone().unwrap());
    assert_eq!(update.previous_daily_request_limit, 3);
    assert_eq!(update.daily_request_limit, 10);
    let updated_key = store
        .authenticate(&client.token)
        .expect("authenticate updated client");
    assert_eq!(updated_key.daily_request_limit, 10);
    assert_eq!(
        store
            .quota_snapshot(&updated_key)
            .expect("updated quota snapshot")
            .remaining_requests,
        8
    );
    assert!(store
        .update_daily_request_limit(&admin.key_id, &"ab".repeat(64), 10)
        .expect("missing target lookup")
        .is_none());
    assert_ne!(other.key_id, client.key_id);
}

#[test]
fn enrollment_requires_allowlist_and_cannot_be_replayed() {
    let directory = tempdir().expect("temp directory");
    let store = RelayStore::open(&directory.path().join("relay.db"), TEST_PEPPER).expect("store");
    let signing_key = SigningKey::from_slice(&[9_u8; 32]).expect("signing key");
    let encoded_point = signing_key.verifying_key().to_encoded_point(false);
    let device_pubkey = hex::encode(&encoded_point.as_bytes()[1..]);

    assert!(matches!(
        store.create_enrollment_challenge(&device_pubkey),
        Err(StoreError::DeviceNotAllowed)
    ));
    store
        .allow_device("device-replay", &device_pubkey, 100)
        .expect("allow device");
    let challenge = store
        .create_enrollment_challenge(&device_pubkey)
        .expect("create challenge");
    store
        .complete_enrollment(&challenge.challenge_id, &device_pubkey)
        .expect("complete enrollment");
    assert!(matches!(
        store.complete_enrollment(&challenge.challenge_id, &device_pubkey),
        Err(StoreError::EnrollmentReplay)
    ));
}

#[test]
fn enrollment_rejects_signature_from_another_device() {
    let signing_key = SigningKey::from_slice(&[10_u8; 32]).expect("signing key");
    let other_signing_key = SigningKey::from_slice(&[11_u8; 32]).expect("other signing key");
    let encoded_point = signing_key.verifying_key().to_encoded_point(false);
    let device_pubkey = hex::encode(&encoded_point.as_bytes()[1..]);
    let challenge = "relay-device-enrollment-v1\nchallenge\npubkey\n123";
    let digest = Sha256::digest(challenge.as_bytes());
    let signature: Signature = other_signing_key
        .sign_prehash(&digest)
        .expect("sign challenge");
    assert!(!crate::device_proof::verify_enrollment_signature(
        &device_pubkey,
        challenge,
        &hex::encode(signature.to_bytes()),
    ));
}

#[test]
fn request_preserves_tool_fields_and_replaces_only_model() {
    let request: ChatCompletionRequest = serde_json::from_value(json!({
        "model": "minimax",
        "messages": [{"role": "user", "content": "hello"}],
        "tools": [{"type": "function", "function": {"name": "clock", "parameters": {"type": "object"}}}],
        "tool_choice": "auto",
        "parallel_tool_calls": true,
        "stream": false
    }))
    .expect("request");
    let config = test_config();
    request.validate(&config).expect("valid request");
    let body = request.to_upstream_body(&config.provider);
    assert_eq!(body["model"], "MiniMax-M3");
    assert_eq!(body["tools"][0]["function"]["name"], "clock");
    assert_eq!(body["parallel_tool_calls"], true);
}

#[test]
fn request_rejects_caller_controlled_upstream_fields() {
    let request: ChatCompletionRequest = serde_json::from_value(json!({
        "model": "minimax",
        "messages": [{"role": "user", "content": "hello"}],
        "base_url": "https://attacker.invalid/v1"
    }))
    .expect("request");
    assert!(request.validate(&test_config()).is_err());
}

#[test]
fn sse_rewriter_masks_upstream_model_and_preserves_usage() {
    let (line, tokens) = rewrite_sse_line(
        b"data: {\"model\":\"MiniMax-M3\",\"usage\":{\"total_tokens\":9}}\n".to_vec(),
        "minimax",
    );
    let text = String::from_utf8(line).expect("utf8");
    assert!(text.contains("\"model\":\"minimax\""));
    assert!(!text.contains("MiniMax-M3"));
    assert_eq!(tokens, 9);
}

fn test_config() -> RelayConfig {
    RelayConfig {
        listen_addr: "127.0.0.1:8796".parse().expect("listen address"),
        store: StoreConfig {
            database_path: "unused.db".into(),
            key_pepper: TEST_PEPPER.to_owned(),
        },
        default_model: "minimax".to_owned(),
        provider: ModelProvider {
            alias: "minimax".to_owned(),
            base_url: "https://api.minimaxi.com/v1".to_owned(),
            api_key: "test-upstream-key".to_owned(),
            model: "MiniMax-M3".to_owned(),
            vendor: "minimax".to_owned(),
        },
        upstream_timeout: std::time::Duration::from_secs(30),
        max_request_body_bytes: 1024 * 1024,
        max_messages: 16,
        max_tools: 16,
        max_inflight: 4,
        max_inflight_per_key: 2,
        limits: QuotaLimits {
            requests_per_minute: 20,
            requests_per_day: 100,
            tokens_per_day: 100_000,
            max_tokens_per_request: 4096,
        },
    }
}
