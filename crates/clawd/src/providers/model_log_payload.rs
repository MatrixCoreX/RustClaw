//! Self-contained log encoding; never used by routing or execution.
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const ENCODING: &str = "gzip-base64-v1";

pub(crate) fn encode(payload: &Value) -> Value {
    encode_inner(payload).unwrap_or_else(|| payload.clone())
}

fn encode_inner(payload: &Value) -> Option<Value> {
    let raw = serde_json::to_vec(payload).ok()?;
    if !(4096..=MAX_BYTES).contains(&raw.len()) {
        return None;
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&raw).ok()?;
    let compressed = encoder.finish().ok()?;
    let encoded = json!({
        "model_log_encoding": ENCODING,
        "uncompressed_bytes": raw.len(),
        "sha256": format!("{:x}", Sha256::digest(&raw)),
        "data": STANDARD.encode(compressed),
    });
    (serde_json::to_vec(&encoded).ok()?.len() < raw.len()).then_some(encoded)
}

pub(crate) fn decode(payload: Value) -> anyhow::Result<Value> {
    let Some(encoding) = payload.get("model_log_encoding") else {
        return Ok(payload);
    };
    anyhow::ensure!(
        encoding == ENCODING,
        "model_log_payload_encoding_unsupported"
    );
    let bytes = payload
        .get("uncompressed_bytes")
        .and_then(Value::as_u64)
        .filter(|bytes| *bytes <= MAX_BYTES as u64)
        .ok_or_else(|| anyhow::anyhow!("model_log_payload_size_invalid"))?;
    let encoded = payload
        .get("data")
        .and_then(Value::as_str)
        .filter(|data| data.len() <= MAX_BYTES * 2)
        .ok_or_else(|| anyhow::anyhow!("model_log_payload_data_invalid"))?;
    let compressed = STANDARD.decode(encoded)?;
    let mut raw = Vec::new();
    GzDecoder::new(compressed.as_slice())
        .take(bytes + 1)
        .read_to_end(&mut raw)?;
    anyhow::ensure!(raw.len() as u64 == bytes, "model_log_payload_size_mismatch");
    let digest = format!("{:x}", Sha256::digest(&raw));
    anyhow::ensure!(
        payload.get("sha256").and_then(Value::as_str) == Some(digest.as_str()),
        "model_log_payload_integrity_mismatch"
    );
    Ok(serde_json::from_slice(&raw)?)
}

pub(crate) fn deserialize_optional<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Option::<Value>::deserialize(deserializer)?
        .map(decode)
        .transpose()
        .map_err(serde::de::Error::custom)
}

#[cfg(test)]
#[path = "model_log_payload_tests.rs"]
mod tests;
