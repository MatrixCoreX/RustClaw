"""Lossless request decoding for versioned model log records, with bounded output."""
import base64
import hashlib
import json
import zlib

MAX_BYTES = 16 * 1024 * 1024


def decode_model_log_payload(payload):
    if not isinstance(payload, dict) or "model_log_encoding" not in payload:
        return payload
    if payload["model_log_encoding"] != "gzip-base64-v1":
        raise ValueError("model_log_payload_encoding_unsupported")
    size = payload.get("uncompressed_bytes")
    data = payload.get("data")
    if (type(size) is not int or not 0 <= size <= MAX_BYTES
            or not isinstance(data, str) or len(data) > MAX_BYTES * 2):
        raise ValueError("model_log_payload_size_invalid")
    compressed = base64.b64decode(data, validate=True)
    decoder = zlib.decompressobj(16 + zlib.MAX_WBITS)
    try:
        raw = decoder.decompress(compressed, size + 1)
    except zlib.error as error:
        raise ValueError("model_log_payload_gzip_invalid") from error
    if len(raw) != size or not decoder.eof:
        raise ValueError("model_log_payload_size_mismatch")
    if hashlib.sha256(raw).hexdigest() != payload.get("sha256"):
        raise ValueError("model_log_payload_integrity_mismatch")
    return json.loads(raw)
