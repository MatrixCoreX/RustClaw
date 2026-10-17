#!/usr/bin/env python3
"""Split oversized release assets and safely reassemble them."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile


SCHEMA_VERSION = 1
SAFE_NAME = re.compile(r"[A-Za-z0-9._-]+")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def require_safe_file(root: Path, name: str) -> Path:
    if not SAFE_NAME.fullmatch(name) or name in {".", ".."}:
        raise ValueError(f"unsafe release asset name: {name!r}")
    path = root / name
    if path.parent.resolve() != root or path.is_symlink():
        raise ValueError(f"unsafe release asset path: {name!r}")
    return path


def write_json_atomic(path: Path, payload: dict[str, object]) -> None:
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, ensure_ascii=True, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def split_file(path: Path, max_bytes: int) -> Path:
    root = path.parent.resolve()
    original_size = path.stat().st_size
    original_digest = sha256_file(path)
    parts: list[dict[str, object]] = []
    with path.open("rb") as source:
        index = 0
        while True:
            payload = source.read(max_bytes)
            if not payload:
                break
            part_name = f"{path.name}.part{index:03d}"
            part_path = require_safe_file(root, part_name)
            if part_path.exists():
                raise FileExistsError(f"release asset part already exists: {part_name}")
            part_path.write_bytes(payload)
            parts.append(
                {
                    "name": part_name,
                    "size": len(payload),
                    "sha256": hashlib.sha256(payload).hexdigest(),
                }
            )
            index += 1
    if len(parts) < 2:
        raise ValueError(f"asset does not require splitting: {path.name}")
    manifest_path = require_safe_file(root, f"{path.name}.parts.json")
    write_json_atomic(
        manifest_path,
        {
            "schema_version": SCHEMA_VERSION,
            "file_name": path.name,
            "file_size": original_size,
            "sha256": original_digest,
            "part_size_limit": max_bytes,
            "parts": parts,
        },
    )
    path.unlink()
    return manifest_path


def split_directory(root: Path, max_bytes: int) -> list[Path]:
    root = root.resolve(strict=True)
    if not root.is_dir() or root.is_symlink():
        raise ValueError("release asset root must be a real directory")
    manifests: list[Path] = []
    for path in sorted(root.iterdir()):
        if not path.is_file() or path.is_symlink() or path.stat().st_size <= max_bytes:
            continue
        if not SAFE_NAME.fullmatch(path.name):
            raise ValueError(f"unsafe release asset name: {path.name!r}")
        manifests.append(split_file(path, max_bytes))
    return manifests


def load_manifest(manifest_path: Path, root: Path) -> dict[str, object]:
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ValueError("release parts manifest must be a regular file")
    payload = json.loads(manifest_path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict) or payload.get("schema_version") != SCHEMA_VERSION:
        raise ValueError("unsupported release parts manifest")
    file_name = payload.get("file_name")
    digest = payload.get("sha256")
    size = payload.get("file_size")
    parts = payload.get("parts")
    if not isinstance(file_name, str) or not SAFE_NAME.fullmatch(file_name):
        raise ValueError("invalid release parts output name")
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise ValueError("invalid release parts digest")
    if not isinstance(size, int) or size < 0 or not isinstance(parts, list) or len(parts) < 2:
        raise ValueError("invalid release parts size or part list")
    seen: set[str] = set()
    for index, entry in enumerate(parts):
        if not isinstance(entry, dict):
            raise ValueError("invalid release part entry")
        name = entry.get("name")
        part_size = entry.get("size")
        part_digest = entry.get("sha256")
        if name != f"{file_name}.part{index:03d}" or name in seen:
            raise ValueError("release parts must be unique and contiguous")
        if not isinstance(part_size, int) or part_size <= 0:
            raise ValueError("invalid release part size")
        if not isinstance(part_digest, str) or not re.fullmatch(r"[0-9a-f]{64}", part_digest):
            raise ValueError("invalid release part digest")
        require_safe_file(root, name)
        seen.add(name)
    return payload


def assemble(manifest_path: Path, root: Path, output: Path | None) -> Path:
    root = root.resolve(strict=True)
    if not root.is_dir() or root.is_symlink():
        raise ValueError("release part root must be a real directory")
    manifest_path = manifest_path.resolve(strict=True)
    if manifest_path.parent != root:
        raise ValueError("release parts manifest must be inside the part root")
    payload = load_manifest(manifest_path, root)
    file_name = str(payload["file_name"])
    destination = output.resolve() if output else require_safe_file(root, file_name)
    if destination.parent != root or destination.is_symlink():
        raise ValueError("release output path must stay inside the part root")

    descriptor, temporary = tempfile.mkstemp(prefix=f".{file_name}.", dir=root)
    digest = hashlib.sha256()
    total = 0
    try:
        with os.fdopen(descriptor, "wb") as target:
            for entry in payload["parts"]:
                part_path = require_safe_file(root, str(entry["name"]))
                if not part_path.is_file() or part_path.stat().st_size != entry["size"]:
                    raise ValueError(f"release part size mismatch: {part_path.name}")
                part_digest = hashlib.sha256()
                with part_path.open("rb") as source:
                    for block in iter(lambda: source.read(1024 * 1024), b""):
                        target.write(block)
                        digest.update(block)
                        part_digest.update(block)
                        total += len(block)
                if part_digest.hexdigest() != entry["sha256"]:
                    raise ValueError(f"release part digest mismatch: {part_path.name}")
            target.flush()
            os.fsync(target.fileno())
        if total != payload["file_size"] or digest.hexdigest() != payload["sha256"]:
            raise ValueError("reassembled release asset digest mismatch")
        os.replace(temporary, destination)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise
    return destination


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    split_parser = subparsers.add_parser("split")
    split_parser.add_argument("root", type=Path)
    split_parser.add_argument("--max-bytes", type=int, required=True)
    assemble_parser = subparsers.add_parser("assemble")
    assemble_parser.add_argument("manifest", type=Path)
    assemble_parser.add_argument("--root", type=Path, required=True)
    assemble_parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    if args.command == "split":
        if args.max_bytes <= 0:
            parser.error("--max-bytes must be positive")
        for manifest in split_directory(args.root, args.max_bytes):
            print(manifest)
        return 0
    destination = assemble(args.manifest, args.root, args.output)
    print(destination)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
