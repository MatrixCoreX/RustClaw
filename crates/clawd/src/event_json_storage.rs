//! Lossless host storage encoding. Public event JSON and its hash chain stay unchanged.
use anyhow::{ensure, Context};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub(crate) const MAX_PACKED_JSON_BYTES: usize = 16 * 1024 * 1024;
const HEADER_BYTES: usize = 45; // magic(4), projection flag(1), length(8), SHA-256(32)
pub(crate) struct StoredJson {
    pub json: String,
    pub version: i64,
    pub blob: Option<Vec<u8>>,
}

fn projection(raw: &str, event_indexes: bool) -> anyhow::Result<Value> {
    if !event_indexes {
        return Ok(json!({}));
    }
    let event: Value = serde_json::from_str(raw)?;
    let mut projected = json!({"payload": {}});
    if let Some(kind) = event.get("event_kind") {
        projected["event_kind"] = kind.clone();
    }
    for field in ["skill", "requested_action_ref"] {
        if let Some(value) = event.get("payload").and_then(|p| p.get(field)) {
            projected["payload"][field] = value.clone();
        }
    }
    Ok(projected)
}

pub(crate) fn encode(raw: &str, event_indexes: bool) -> anyhow::Result<StoredJson> {
    let plain = || StoredJson {
        json: raw.into(),
        version: 1,
        blob: None,
    };
    if !(512..=MAX_PACKED_JSON_BYTES).contains(&raw.len()) {
        return Ok(plain());
    }
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::fast());
    compressor.write_all(raw.as_bytes())?;
    let compressed = compressor.finish()?;
    let projected = serde_json::to_string(&projection(raw, event_indexes)?)?;
    if projected
        .len()
        .saturating_add(compressed.len())
        .saturating_add(HEADER_BYTES + 16)
        >= raw.len().saturating_mul(7) / 8
    {
        return Ok(plain());
    }
    let mut blob = Vec::with_capacity(HEADER_BYTES + compressed.len());
    blob.extend_from_slice(b"EVJ1");
    blob.push(u8::from(event_indexes));
    blob.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    blob.extend_from_slice(&Sha256::digest(raw.as_bytes()));
    blob.extend_from_slice(&compressed);
    Ok(StoredJson {
        json: projected,
        version: 2,
        blob: Some(blob),
    })
}

pub(crate) fn decode(raw: &str, version: i64, blob: Option<&[u8]>) -> anyhow::Result<String> {
    if matches!(version, 0 | 1) {
        return Ok(raw.into());
    }
    ensure!(version == 2, "event_storage_version_unsupported");
    let blob = blob.context("event_storage_blob_missing")?;
    ensure!(
        blob.len() >= HEADER_BYTES && blob.len() <= MAX_PACKED_JSON_BYTES + HEADER_BYTES,
        "event_storage_blob_size_invalid"
    );
    ensure!(
        &blob[..4] == b"EVJ1" && blob[4] <= 1,
        "event_storage_codec_unsupported"
    );
    let length = u64::from_le_bytes(blob[5..13].try_into()?);
    ensure!(
        length <= MAX_PACKED_JSON_BYTES as u64,
        "event_storage_payload_too_large"
    );
    ensure!(
        raw.len() <= MAX_PACKED_JSON_BYTES,
        "event_storage_projection_too_large"
    );
    let compressed = &blob[HEADER_BYTES..];
    let mut decoder = ZlibDecoder::new(compressed);
    let mut decoded = Vec::with_capacity(length as usize);
    decoder
        .by_ref()
        .take(length + 1)
        .read_to_end(&mut decoded)?;
    ensure!(
        decoded.len() as u64 == length,
        "event_storage_length_mismatch"
    );
    ensure!(
        decoder.total_in() == compressed.len() as u64,
        "event_storage_trailing_data"
    );
    ensure!(
        Sha256::digest(&decoded).as_slice() == &blob[13..45],
        "event_storage_digest_mismatch"
    );
    let decoded = String::from_utf8(decoded).context("event_storage_utf8_invalid")?;
    ensure!(
        serde_json::from_str::<Value>(raw)? == projection(&decoded, blob[4] == 1)?,
        "event_storage_projection_mismatch"
    );
    Ok(decoded)
}

pub(crate) fn read(raw: &str, version: i64, blob: Option<&[u8]>) -> anyhow::Result<Value> {
    Ok(serde_json::from_str(&decode(raw, version, blob)?)?)
}

#[cfg(test)]
#[path = "event_json_storage_tests.rs"]
mod tests;
