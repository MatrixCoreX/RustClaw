#!/usr/bin/env python3
"""Create and verify canonical, external release artifact manifests."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


SCHEMA_VERSION = 1
SAFE_NAME = re.compile(r"^[A-Za-z0-9._-]+$")
SAFE_TARGET = re.compile(r"^[A-Za-z0-9._-]+$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_bytes(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode()


def safe_token(value: str, label: str, maximum: int = 160) -> str:
    if not value or len(value) > maximum or not SAFE_TARGET.fullmatch(value):
        raise SystemExit(f"release_manifest_{label}_invalid")
    return value


def create_manifest(args: argparse.Namespace) -> None:
    artifact = Path(args.artifact).resolve(strict=True)
    sbom = Path(args.sbom).resolve(strict=True)
    if not artifact.is_file() or artifact.is_symlink() or not SAFE_NAME.fullmatch(artifact.name):
        raise SystemExit("release_manifest_artifact_invalid")
    if not sbom.is_file() or sbom.is_symlink() or not SAFE_NAME.fullmatch(sbom.name):
        raise SystemExit("release_manifest_sbom_invalid")
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "artifact": {
            "name": artifact.name,
            "size": artifact.stat().st_size,
            "sha256": sha256_file(artifact),
        },
        "evidence": {
            "sbom": {
                "name": sbom.name,
                "size": sbom.stat().st_size,
                "sha256": sha256_file(sbom),
            }
        },
        "release": {
            "version": safe_token(args.version, "version"),
            "commit": safe_token(args.commit, "commit"),
            "target": safe_token(args.target, "target"),
            "package_root": safe_token(args.package_root, "package_root"),
        },
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(canonical_bytes(manifest))


def verify_manifest(args: argparse.Namespace) -> None:
    artifact = Path(args.artifact).resolve(strict=True)
    sbom = Path(args.sbom).resolve(strict=True)
    manifest_path = Path(args.manifest).resolve(strict=True)
    try:
        raw = manifest_path.read_bytes()
        manifest = json.loads(raw)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SystemExit("release_manifest_json_invalid") from error
    if raw != canonical_bytes(manifest):
        raise SystemExit("release_manifest_not_canonical")
    if set(manifest) != {"schema_version", "artifact", "evidence", "release"}:
        raise SystemExit("release_manifest_fields_invalid")
    if manifest["schema_version"] != SCHEMA_VERSION:
        raise SystemExit("release_manifest_schema_unsupported")
    artifact_record = manifest.get("artifact")
    evidence = manifest.get("evidence")
    release = manifest.get("release")
    if not isinstance(artifact_record, dict) or set(artifact_record) != {"name", "size", "sha256"}:
        raise SystemExit("release_manifest_artifact_fields_invalid")
    if not isinstance(release, dict) or set(release) != {"version", "commit", "target", "package_root"}:
        raise SystemExit("release_manifest_release_fields_invalid")
    if not isinstance(evidence, dict) or set(evidence) != {"sbom"}:
        raise SystemExit("release_manifest_evidence_fields_invalid")
    sbom_record = evidence.get("sbom")
    if not isinstance(sbom_record, dict) or set(sbom_record) != {"name", "size", "sha256"}:
        raise SystemExit("release_manifest_sbom_fields_invalid")
    if artifact_record["name"] != artifact.name or not SAFE_NAME.fullmatch(str(artifact_record["name"])):
        raise SystemExit("release_manifest_artifact_name_mismatch")
    if not isinstance(artifact_record["size"], int) or artifact_record["size"] != artifact.stat().st_size:
        raise SystemExit("release_manifest_artifact_size_mismatch")
    expected_hash = str(artifact_record["sha256"])
    if not SHA256.fullmatch(expected_hash) or expected_hash != sha256_file(artifact):
        raise SystemExit("release_manifest_artifact_hash_mismatch")
    if sbom_record["name"] != sbom.name or not SAFE_NAME.fullmatch(str(sbom_record["name"])):
        raise SystemExit("release_manifest_sbom_name_mismatch")
    if not isinstance(sbom_record["size"], int) or sbom_record["size"] != sbom.stat().st_size:
        raise SystemExit("release_manifest_sbom_size_mismatch")
    expected_sbom_hash = str(sbom_record["sha256"])
    if not SHA256.fullmatch(expected_sbom_hash) or expected_sbom_hash != sha256_file(sbom):
        raise SystemExit("release_manifest_sbom_hash_mismatch")
    if release["target"] != args.expected_target:
        raise SystemExit("release_manifest_target_mismatch")
    if release["package_root"] != args.expected_package_root:
        raise SystemExit("release_manifest_package_root_mismatch")
    if args.expected_version and release["version"] != args.expected_version:
        raise SystemExit("release_manifest_version_mismatch")
    for field in ("version", "commit", "target", "package_root"):
        safe_token(str(release[field]), field)
    print(json.dumps(manifest, sort_keys=True, separators=(",", ":")))


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    subcommands = root.add_subparsers(dest="command", required=True)
    create = subcommands.add_parser("create")
    create.add_argument("--artifact", required=True)
    create.add_argument("--sbom", required=True)
    create.add_argument("--output", required=True)
    create.add_argument("--version", required=True)
    create.add_argument("--commit", required=True)
    create.add_argument("--target", required=True)
    create.add_argument("--package-root", required=True)
    create.set_defaults(handler=create_manifest)
    verify = subcommands.add_parser("verify")
    verify.add_argument("--artifact", required=True)
    verify.add_argument("--sbom", required=True)
    verify.add_argument("--manifest", required=True)
    verify.add_argument("--expected-target", required=True)
    verify.add_argument("--expected-package-root", required=True)
    verify.add_argument("--expected-version")
    verify.set_defaults(handler=verify_manifest)
    return root


def main() -> None:
    args = parser().parse_args()
    args.handler(args)


if __name__ == "__main__":
    main()
