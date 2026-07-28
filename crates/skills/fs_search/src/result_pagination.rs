use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const CURSOR_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub(super) enum CursorInput {
    Start,
    LegacyOffset(usize),
    Opaque(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct CursorPayload {
    version: u8,
    query_sha256: String,
    snapshot_sha256: String,
    offset: usize,
}

pub(super) struct ResultPage<T> {
    pub(super) items: Vec<T>,
    pub(super) metadata: Value,
    pub(super) returned_count: usize,
    pub(super) total_count: usize,
    pub(super) has_more: bool,
    pub(super) snapshot_sha256: String,
    pub(super) stale_snapshot: bool,
}

pub(super) fn cursor_from_args(
    obj: &serde_json::Map<String, Value>,
) -> Result<CursorInput, String> {
    let Some(value) = obj.get("cursor").or_else(|| obj.get("offset")) else {
        return Ok(CursorInput::Start);
    };
    if let Some(offset) = value.as_u64() {
        return Ok(CursorInput::LegacyOffset(
            offset.min(usize::MAX as u64) as usize
        ));
    }
    if value.is_null() {
        return Ok(CursorInput::Start);
    }
    let Some(token) = value.as_str().map(str::trim) else {
        return Err("invalid_cursor".to_string());
    };
    if token.is_empty() {
        return Ok(CursorInput::Start);
    }
    if let Ok(offset) = token.parse::<usize>() {
        return Ok(CursorInput::LegacyOffset(offset));
    }
    Ok(CursorInput::Opaque(token.to_string()))
}

pub(super) fn query_sha256(obj: &serde_json::Map<String, Value>) -> String {
    let mut normalized = obj.clone();
    normalized.remove("cursor");
    normalized.remove("offset");
    normalized.remove("max_results");
    // Runtime recall is advisory context injected by clawd, not part of the
    // filesystem query. It can legitimately change between continuation
    // requests and must not invalidate an otherwise identical opaque cursor.
    normalized.remove("_memory");
    let encoded = serde_json::to_vec(&normalized).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}

pub(super) fn cursor_snapshot_identity(
    cursor: &CursorInput,
) -> Result<Option<(String, String)>, String> {
    let CursorInput::Opaque(token) = cursor else {
        return Ok(None);
    };
    let payload = decode_cursor(token)?;
    if payload.version != CURSOR_VERSION {
        return Err("invalid_cursor".to_string());
    }
    Ok(Some((payload.query_sha256, payload.snapshot_sha256)))
}

pub(super) fn paginate<T>(
    values: &[T],
    cursor: &CursorInput,
    limit: usize,
    scan_truncated: bool,
    query_sha256: &str,
) -> Result<ResultPage<T>, String>
where
    T: Clone + Serialize,
{
    let total_count = values.len();
    let snapshot_sha256 = snapshot_sha256(values);
    let (page_start, stale_snapshot) = resolve_cursor(cursor, query_sha256, &snapshot_sha256)?;
    if stale_snapshot {
        let metadata = json!({
            "cursor": 0,
            "cursor_token": Value::Null,
            "limit": limit,
            "returned_count": 0,
            "known_match_count": total_count,
            "total_count": total_count,
            "has_more": false,
            "next_cursor": Value::Null,
            "previous_cursor": Value::Null,
            "legacy_next_offset": Value::Null,
            "scan_truncated": true,
            "stale_snapshot": true,
            "query_sha256": query_sha256,
            "snapshot_sha256": snapshot_sha256,
        });
        return Ok(ResultPage {
            items: Vec::new(),
            metadata,
            returned_count: 0,
            total_count,
            has_more: false,
            snapshot_sha256,
            stale_snapshot: true,
        });
    }
    if page_start > total_count {
        return Err("cursor_out_of_range".to_string());
    }
    let page_end = page_start.saturating_add(limit).min(total_count);
    let items = values[page_start..page_end].to_vec();
    let returned_count = items.len();
    let has_more = page_end < total_count || scan_truncated;
    let next_cursor =
        (page_end < total_count).then(|| encode_cursor(query_sha256, &snapshot_sha256, page_end));
    let previous_offset = (page_start > 0).then_some(page_start.saturating_sub(limit));
    let previous_cursor =
        previous_offset.map(|offset| encode_cursor(query_sha256, &snapshot_sha256, offset));
    let cursor_token =
        (page_start > 0).then(|| encode_cursor(query_sha256, &snapshot_sha256, page_start));
    let metadata = json!({
        "cursor": page_start,
        "cursor_token": cursor_token,
        "limit": limit,
        "returned_count": returned_count,
        "known_match_count": total_count,
        "total_count": total_count,
        "has_more": has_more,
        "next_cursor": next_cursor,
        "previous_cursor": previous_cursor,
        "legacy_next_offset": (page_end < total_count).then_some(page_end),
        "scan_truncated": scan_truncated,
        "stale_snapshot": false,
        "query_sha256": query_sha256,
        "snapshot_sha256": snapshot_sha256,
    });
    Ok(ResultPage {
        items,
        metadata,
        returned_count,
        total_count,
        has_more,
        snapshot_sha256,
        stale_snapshot: false,
    })
}

fn resolve_cursor(
    cursor: &CursorInput,
    query_sha256: &str,
    snapshot_sha256: &str,
) -> Result<(usize, bool), String> {
    match cursor {
        CursorInput::Start => Ok((0, false)),
        CursorInput::LegacyOffset(offset) => Ok((*offset, false)),
        CursorInput::Opaque(token) => {
            let payload = decode_cursor(token)?;
            if payload.version != CURSOR_VERSION {
                return Err("invalid_cursor".to_string());
            }
            if payload.query_sha256 != query_sha256 {
                return Err("cursor_query_mismatch".to_string());
            }
            Ok((payload.offset, payload.snapshot_sha256 != snapshot_sha256))
        }
    }
}

fn encode_cursor(query_sha256: &str, snapshot_sha256: &str, offset: usize) -> String {
    let payload = CursorPayload {
        version: CURSOR_VERSION,
        query_sha256: query_sha256.to_string(),
        snapshot_sha256: snapshot_sha256.to_string(),
        offset,
    };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap_or_default())
}

fn decode_cursor(token: &str) -> Result<CursorPayload, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| "invalid_cursor".to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "invalid_cursor".to_string())
}

fn snapshot_sha256<T: Serialize>(values: &[T]) -> String {
    let encoded = serde_json::to_vec(values).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}
