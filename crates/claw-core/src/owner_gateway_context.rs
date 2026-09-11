use crate::channel_event_admission::{
    sha256_hex, sign_admission_request, verify_admission_request_signature,
};

pub const HEADER: &str = "x-agent-owner-session";
pub const PREFIX: &str = "/v1/nni/assets/owner/";

fn message(session: &str, method: &str, path: &str, body: &[u8]) -> Vec<u8> {
    format!(
        "owner_gateway_session_v1\n{session}\n{method}\n{path}\n{}",
        sha256_hex(body)
    )
    .into_bytes()
}

// The key is already held by both authenticated gateway processes. This is a
// domain-separated internal assertion, never a new client credential.
pub fn sign(
    key: &str,
    session: &str,
    method: &str,
    path: &str,
    body: &[u8],
    now: u64,
) -> Option<String> {
    if uuid::Uuid::parse_str(session).is_err() || !path.starts_with(PREFIX) {
        return None;
    }
    let signature = sign_admission_request(key, now, &message(session, method, path, body)).ok()?;
    Some(format!("{session}:{now}:{signature}"))
}

pub fn binding(
    key: &str,
    assertion: Option<&str>,
    method: &str,
    path: &str,
    body: &[u8],
    now: u64,
) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    let Some(assertion) = assertion else {
        // Explicit key callers have a key lifecycle, not a browser session.
        return Some(sha256_hex(
            format!("owner_gateway_key_v1\0{key}").as_bytes(),
        ));
    };
    if assertion.len() > 160 {
        return None;
    }
    let mut fields = assertion.splitn(3, ':');
    let session = fields.next()?;
    let timestamp: u64 = fields.next()?.parse().ok()?;
    let signature = fields.next()?;
    if uuid::Uuid::parse_str(session).is_err()
        || now.abs_diff(timestamp) > 15
        || !verify_admission_request_signature(
            key,
            timestamp,
            &message(session, method, path, body),
            signature,
        )
    {
        return None;
    }
    Some(sha256_hex(
        format!("owner_gateway_session_v1\0{key}\0{session}").as_bytes(),
    ))
}

#[cfg(test)]
#[path = "owner_gateway_context_tests.rs"]
mod tests;
