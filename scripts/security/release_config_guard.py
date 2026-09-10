#!/usr/bin/env python3
"""Reject embedded release credentials without rewriting valid configuration."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from redact_configs import SENSITIVE_EXACT


SAFE_REFERENCE = re.compile(
    r"(?:REPLACE_ME(?:_[A-Z0-9_]+)?|REDACTED_[A-Z0-9_]+|\$\{[A-Z_][A-Z0-9_]*\})"
)
SECRET_SUFFIXES = ("_api_key", "_private_key", "_secret", "_password", "_passphrase")
PRIVATE_KEY_HEADER = re.compile(r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----")


def credential_fields(value: object, path: tuple[str, ...] = ()) -> list[str]:
    findings: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            location = (*path, key)
            normalized = key.lower().replace("-", "_")
            sensitive = (
                normalized in SENSITIVE_EXACT
                or normalized.endswith(SECRET_SUFFIXES)
                or normalized in {"authorization", "cookie", "x_api_key"}
            )
            if sensitive and child and (
                not isinstance(child, str) or not SAFE_REFERENCE.fullmatch(child)
            ):
                findings.append(".".join(location))
            else:
                findings.extend(credential_fields(child, location))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            findings.extend(credential_fields(child, (*path, str(index))))
    return findings


def inspect_config(root: Path, path: Path) -> list[str]:
    relative = path.relative_to(root)
    label = relative.as_posix()
    if path.is_symlink() or any(
        (root / parent).is_symlink() for parent in relative.parents if parent != Path(".")
    ):
        return [f"{label}: symlink_not_allowed"]
    try:
        raw = path.read_text(encoding="utf-8")
        data = tomllib.loads(raw)
    except (OSError, UnicodeError, tomllib.TOMLDecodeError):
        return [f"{label}: invalid_toml"]
    if PRIVATE_KEY_HEADER.search(raw):
        return [f"{label}: private_key_material"]
    # Translation keys describe messages, not credential configuration.
    if relative.parts[0] == "i18n":
        return []
    findings = [f"{label}: {key}: embedded_credential" for key in credential_fields(data)]
    if relative.parts[0] == "channels":
        for section, fields in data.items():
            if not isinstance(fields, dict):
                continue
            for key in ("admins", "allowlist", "bindings"):
                if fields.get(key):
                    findings.append(f"{label}: {section}.{key}: private_channel_binding")
    return findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config_dir", type=Path)
    args = parser.parse_args()
    root = args.config_dir.absolute()
    if not root.is_dir() or root.is_symlink():
        print("RELEASE_CONFIG_CHECK config_directory_invalid", file=sys.stderr)
        return 1
    paths = sorted(root.rglob("*.toml"))
    if not paths:
        print("RELEASE_CONFIG_CHECK configs_missing", file=sys.stderr)
        return 1
    findings = [finding for path in paths for finding in inspect_config(root, path)]
    if findings:
        print("RELEASE_CONFIG_CHECK rejected; use empty credentials or environment references", file=sys.stderr)
        for finding in findings:
            print(finding, file=sys.stderr)
        return 1
    print(f"RELEASE_CONFIG_CHECK ok files={len(paths)} modified=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
