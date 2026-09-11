"""Derive the exact unsigned NSIS payload produced by the pinned Tauri bundler."""
import hashlib
import json
from pathlib import Path

# tauri-cli-v2.11.4 crates/tauri-bundler/src/bundle.rs patches the first
# placeholder and restores the unpatched file after each package is built.
PLACEHOLDER = b'__TAURI_BUNDLE_TYPE_VAR_UNK'
NSIS_MARKER = b'__TAURI_BUNDLE_TYPE_VAR_NSS'


def nsis_payload(raw: bytes) -> tuple[bytes, int]:
    offset = raw.find(PLACEHOLDER)
    if offset < 0:
        raise ValueError('tauri_bundle_placeholder_missing')
    return raw[:offset] + NSIS_MARKER + raw[offset + len(PLACEHOLDER):], offset


def expected_payload(binary: Path, evidence: Path) -> Path:
    raw = binary.read_bytes()
    expected, offset = nsis_payload(raw)
    output = evidence / 'expected-nsis-payload.exe'
    output.write_bytes(expected)
    report = {
        'bundler_contract': 'tauri-cli-v2.11.4',
        'patch_offset': offset,
        'patch_bytes': len(PLACEHOLDER),
        'source_sha256': hashlib.sha256(raw).hexdigest(),
        'expected_installed_sha256': hashlib.sha256(expected).hexdigest(),
        'bytes': len(expected),
    }
    (evidence / 'expected-payload.json').write_text(json.dumps(report, indent=2) + '\n')
    return output
