use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use serde::Serialize;
use std::sync::OnceLock;

const CURSOR_TTL_SECONDS: i64 = 900;
type CursorMac = Hmac<Sha256>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    binding: String,
    snapshot: String,
    selection: String,
    offset: usize,
    page_size: usize,
    expires_at: i64,
}

fn signing_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| {
        // Process-local, independent of client-visible API keys. Restarting the
        // runtime invalidates cursors; clients restart discovery from page one.
        let mut key = [0; 32];
        key[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        key[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        key
    })
}

pub(super) fn issue(
    binding: &str,
    snapshot: &str,
    selection: &str,
    offset: usize,
    page_size: usize,
    now: i64,
) -> Result<String, &'static str> {
    let payload = serde_json::to_vec(&Cursor {
        version: 1,
        binding: binding.into(),
        snapshot: snapshot.into(),
        selection: selection.into(),
        offset,
        page_size,
        expires_at: now.saturating_add(CURSOR_TTL_SECONDS),
    })
    .map_err(|_| "context_cursor_invalid")?;
    let mut mac = CursorMac::new_from_slice(signing_key()).expect("fixed cursor key");
    mac.update(b"context-recall:v1\0");
    mac.update(&payload);
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&payload),
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

pub(super) fn offset(
    token: &str,
    binding: &str,
    snapshot: &str,
    selection: &str,
    page_size: usize,
    now: i64,
) -> Result<usize, &'static str> {
    if token.len() > 1024 {
        return Err("context_cursor_invalid");
    }
    let (payload, signature) = token.split_once('.').ok_or("context_cursor_invalid")?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| "context_cursor_invalid")?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| "context_cursor_invalid")?;
    let mut mac = CursorMac::new_from_slice(signing_key()).expect("fixed cursor key");
    mac.update(b"context-recall:v1\0");
    mac.update(&payload);
    mac.verify_slice(&signature)
        .map_err(|_| "context_cursor_invalid")?;
    let cursor: Cursor = serde_json::from_slice(&payload).map_err(|_| "context_cursor_invalid")?;
    if cursor.version != 1
        || cursor.binding != binding
        || cursor.selection != selection
        || cursor.page_size != page_size
        || cursor.offset == 0
    {
        return Err("context_cursor_invalid");
    }
    if cursor.expires_at <= now || cursor.expires_at > now.saturating_add(CURSOR_TTL_SECONDS) {
        return Err("context_cursor_expired");
    }
    if cursor.snapshot != snapshot {
        return Err("context_snapshot_changed");
    }
    Ok(cursor.offset)
}
