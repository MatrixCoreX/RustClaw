#!/usr/bin/env python3
"""Generate a deterministic SPDX 2.3 inventory from repository lockfiles."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import tomllib
import urllib.parse
from pathlib import Path


IGNORED_PARTS = {".git", "data", "dist", "node_modules", "target"}
SAFE_TOKEN = re.compile(r"^[A-Za-z0-9._-]+$")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def canonical_bytes(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":")) + "\n").encode()


def discover_lockfiles(root: Path) -> list[Path]:
    found: list[Path] = []
    for path in root.rglob("*"):
        if not path.is_file() or any(part in IGNORED_PARTS for part in path.relative_to(root).parts):
            continue
        if path.name in {"Cargo.lock", "package-lock.json", "requirements.txt"}:
            found.append(path)
    return sorted(found, key=lambda item: item.relative_to(root).as_posix())


def package_record(ecosystem: str, name: str, version: str, checksum: str | None, source: str) -> dict:
    identity = f"{ecosystem}\0{name}\0{version}\0{source}"
    spdx_id = f"SPDXRef-Package-{sha256_bytes(identity.encode())[:24]}"
    quoted_name = urllib.parse.quote(name, safe="._-")
    quoted_version = urllib.parse.quote(version, safe="._-+")
    record = {
        "SPDXID": spdx_id,
        "name": name,
        "versionInfo": version,
        "downloadLocation": source if source.startswith(("http://", "https://", "git+")) else "NOASSERTION",
        "filesAnalyzed": False,
        "licenseConcluded": "NOASSERTION",
        "licenseDeclared": "NOASSERTION",
        "copyrightText": "NOASSERTION",
        "externalRefs": [
            {
                "referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": f"pkg:{ecosystem}/{quoted_name}@{quoted_version}",
            }
        ],
    }
    if checksum and re.fullmatch(r"[0-9a-fA-F]{64}", checksum):
        record["checksums"] = [{"algorithm": "SHA256", "checksumValue": checksum.lower()}]
    return record


def cargo_packages(path: Path) -> list[dict]:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    records = []
    for package in data.get("package", []):
        name = str(package.get("name") or "").strip()
        version = str(package.get("version") or "").strip()
        if not name or not version:
            continue
        source = str(package.get("source") or "NOASSERTION")
        records.append(package_record("cargo", name, version, package.get("checksum"), source))
    return records


def npm_packages(path: Path) -> list[dict]:
    data = json.loads(path.read_text(encoding="utf-8"))
    records = []
    for location, package in sorted((data.get("packages") or {}).items()):
        if not location or not isinstance(package, dict):
            continue
        name = str(package.get("name") or Path(location).name).strip()
        version = str(package.get("version") or "").strip()
        if not name or not version:
            continue
        source = str(package.get("resolved") or "NOASSERTION")
        records.append(package_record("npm", name, version, None, source))
    return records


def requirement_packages(path: Path) -> list[dict]:
    records = []
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or line.startswith(("-", "http:", "https:", "git+")):
            continue
        match = re.fullmatch(r"([A-Za-z0-9_.-]+)==([A-Za-z0-9_.+!-]+)(?:\s+--hash=sha256:([0-9a-fA-F]{64}))?", line)
        if match:
            records.append(package_record("pypi", match.group(1), match.group(2), match.group(3), "NOASSERTION"))
    return records


def build_sbom(args: argparse.Namespace) -> dict:
    root = Path(args.root).resolve(strict=True)
    for token, label in ((args.name, "name"), (args.version, "version"), (args.commit, "commit")):
        if not SAFE_TOKEN.fullmatch(token):
            raise SystemExit(f"release_sbom_{label}_invalid")
    packages: dict[str, dict] = {}
    lockfiles = discover_lockfiles(root)
    for path in lockfiles:
        if path.name == "Cargo.lock":
            records = cargo_packages(path)
        elif path.name == "package-lock.json":
            records = npm_packages(path)
        else:
            records = requirement_packages(path)
        for record in records:
            packages[record["SPDXID"]] = record
    package_list = [packages[key] for key in sorted(packages)]
    inventory_digest = sha256_bytes(canonical_bytes(package_list))
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0") or "0")
    created = dt.datetime.fromtimestamp(epoch, tz=dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    document_id = f"SPDXRef-DOCUMENT-{inventory_digest[:24]}"
    return {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": args.name,
        "documentNamespace": f"https://spdx.invalid/agent-runtime/{args.version}/{args.commit}/{inventory_digest}",
        "creationInfo": {"created": created, "creators": ["Tool: generate_release_sbom.py"]},
        "documentDescribes": [document_id],
        "packages": [
            {
                "SPDXID": document_id,
                "name": args.name,
                "versionInfo": args.version,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": False,
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": "NOASSERTION",
                "copyrightText": "NOASSERTION",
                "externalRefs": [
                    {
                        "referenceCategory": "OTHER",
                        "referenceType": "commit",
                        "referenceLocator": args.commit,
                    }
                ],
            },
            *package_list,
        ],
        "relationships": [
            {"spdxElementId": document_id, "relationshipType": "DEPENDS_ON", "relatedSpdxElement": item["SPDXID"]}
            for item in package_list
        ],
        "annotations": [
            {
                "annotationDate": created,
                "annotationType": "OTHER",
                "annotator": "Tool: generate_release_sbom.py",
                "comment": "Lockfile inventory: " + ", ".join(path.relative_to(root).as_posix() for path in lockfiles),
            }
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(canonical_bytes(build_sbom(args)))


if __name__ == "__main__":
    main()
