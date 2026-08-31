#!/usr/bin/env python3
"""Scan runtime output surfaces for explicitly supplied secret markers."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tempfile
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterable


SCHEMA_VERSION = 1
CHUNK_BYTES = 1024 * 1024
MIN_MARKER_BYTES = 12


@dataclass(frozen=True)
class Finding:
    control_id: str
    surface: str
    path: str
    marker_sha256: str


def marker_digest(marker: bytes) -> str:
    return hashlib.sha256(marker).hexdigest()


def contains_marker(path: Path, marker: bytes) -> bool:
    overlap = max(0, len(marker) - 1)
    tail = b""
    with path.open("rb") as stream:
        while True:
            chunk = stream.read(CHUNK_BYTES)
            if not chunk:
                return False
            window = tail + chunk
            if marker in window:
                return True
            tail = window[-overlap:] if overlap else b""


def surface_files(root: Path) -> Iterable[Path]:
    if root.is_symlink():
        return
    if root.is_file():
        yield root
        return
    if not root.is_dir():
        return
    for directory, names, files in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        names[:] = [name for name in names if not (directory_path / name).is_symlink()]
        for name in files:
            path = directory_path / name
            if not path.is_symlink() and path.is_file():
                yield path


def parse_surface(raw: str) -> tuple[str, Path]:
    name, separator, path = raw.partition("=")
    name = name.strip()
    if not separator or not name or len(name) > 64 or not all(
        character.isalnum() or character in "_-" for character in name
    ):
        raise ValueError("secret_surface_name_invalid")
    resolved = Path(path).expanduser().resolve(strict=False)
    return name, resolved


def run_scan(surfaces: list[tuple[str, Path]], markers: list[bytes]) -> dict[str, object]:
    findings: list[Finding] = []
    scanned_files = 0
    for surface, root in surfaces:
        for path in surface_files(root):
            scanned_files += 1
            for marker in markers:
                if contains_marker(path, marker):
                    findings.append(
                        Finding(
                            control_id="SEC-SECRET-SURFACE-001",
                            surface=surface,
                            path=str(path),
                            marker_sha256=marker_digest(marker),
                        )
                    )
    findings.sort(key=lambda item: (item.surface, item.path, item.marker_sha256))
    return {
        "schema_version": SCHEMA_VERSION,
        "status": "pass" if not findings else "fail",
        "scanned_files": scanned_files,
        "surface_count": len(surfaces),
        "marker_count": len(markers),
        "findings": [asdict(item) for item in findings],
    }


def self_test() -> int:
    marker = b"agent-secret-marker-7f84d2b1"
    with tempfile.TemporaryDirectory(prefix="secret-surface-scan-") as raw_root:
        root = Path(raw_root)
        (root / "clean.log").write_text("normal runtime event\n", encoding="utf-8")
        (root / "nested").mkdir()
        (root / "nested" / "teaching.json").write_bytes(b'{"token":"' + marker + b'"}')
        (root / "runtime.sqlite3-wal").write_bytes(b"prefix" + marker + b"suffix")
        result = run_scan([("fixture", root)], [marker])
    encoded = json.dumps(result, sort_keys=True)
    passed = (
        result["status"] == "fail"
        and result["scanned_files"] == 3
        and len(result["findings"]) == 2
        and marker.decode("ascii") not in encoded
    )
    print(
        json.dumps(
            {
                "schema_version": SCHEMA_VERSION,
                "status": "pass" if passed else "fail",
                "test": "secret_surface_scan_self_test",
            },
            sort_keys=True,
        )
    )
    return 0 if passed else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--surface",
        action="append",
        default=[],
        metavar="NAME=PATH",
        help="runtime output surface to scan; repeat for logs, diagnostics, backups, and traces",
    )
    parser.add_argument(
        "--forbid-literal",
        action="append",
        default=[],
        help="exact secret marker that must not occur; repeatable and never emitted in output",
    )
    parser.add_argument("--output")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    try:
        surfaces = [parse_surface(value) for value in args.surface]
    except ValueError as error:
        parser.error(str(error))
    markers = [value.encode("utf-8") for value in args.forbid_literal]
    if not surfaces:
        parser.error("at least one --surface is required")
    if not markers or any(len(marker) < MIN_MARKER_BYTES for marker in markers):
        parser.error(f"each --forbid-literal must contain at least {MIN_MARKER_BYTES} UTF-8 bytes")
    result = run_scan(surfaces, markers)
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        output = Path(args.output)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(encoded, encoding="utf-8")
    else:
        print(encoded, end="")
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
