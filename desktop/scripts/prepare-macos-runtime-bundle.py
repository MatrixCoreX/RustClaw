#!/usr/bin/env python3
"""Stage one signed macOS Runtime release for embedding in the desktop app."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tomllib


SAFE_TOKEN = re.compile(r"^[A-Za-z0-9._-]+$")
BOOTSTRAP_FILES = (
    "deploy-github-release.sh",
    "scripts/product_identity.sh",
    "scripts/shell_compat.sh",
    "scripts/verify_release_binary.py",
    "scripts/security/release_manifest.py",
    "configs/product_identity.toml",
    "configs/release_allowed_signers",
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_real_file(path: Path, label: str) -> Path:
    try:
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise ValueError(f"{label}_missing_or_unsafe") from error
    if not resolved.is_file() or path.is_symlink():
        raise ValueError(f"{label}_missing_or_unsafe")
    return resolved


def require_real_directory(path: Path, label: str) -> Path:
    try:
        resolved = path.resolve(strict=True)
    except OSError as error:
        raise ValueError(f"{label}_missing_or_unsafe") from error
    if not resolved.is_dir():
        raise ValueError(f"{label}_missing_or_unsafe")
    return resolved


def platform_for_target(target: str) -> str:
    mapping = {
        "aarch64-apple-darwin": "macos-aarch64",
        "x86_64-apple-darwin": "macos-x86_64",
    }
    try:
        return mapping[target]
    except KeyError as error:
        raise ValueError("embedded_runtime_target_must_be_native_macos") from error


def stage(
    source_root: Path,
    release_dir: Path,
    output: Path,
    target: str,
    release_tag: str,
    python_runtime_root: Path,
) -> dict[str, object]:
    source_root = source_root.resolve(strict=True)
    release_dir = release_dir.resolve(strict=True)
    if not release_dir.is_dir() or release_dir.is_symlink():
        raise ValueError("embedded_runtime_release_directory_unsafe")
    identity = tomllib.loads(
        require_real_file(
            source_root / "configs/product_identity.toml", "product_identity"
        ).read_text(encoding="utf-8")
    )
    artifact_id = str(identity.get("release_artifact_id") or "").strip()
    if not SAFE_TOKEN.fullmatch(artifact_id):
        raise ValueError("embedded_runtime_artifact_id_invalid")
    platform = platform_for_target(target)
    if not SAFE_TOKEN.fullmatch(release_tag) or not release_tag.startswith(f"{platform}-"):
        raise ValueError("embedded_runtime_release_tag_invalid")
    archives = sorted(release_dir.glob(f"{artifact_id}-{platform}-*.tar.gz"))
    archives = [path for path in archives if path.is_file() and not path.is_symlink()]
    if len(archives) != 1:
        raise ValueError("embedded_runtime_archive_count_invalid")
    archive = archives[0]
    if archive.name != f"{artifact_id}-{release_tag}.tar.gz":
        raise ValueError("embedded_runtime_archive_tag_mismatch")
    companions = (
        archive,
        Path(f"{archive}.sha256"),
        Path(f"{archive}.spdx.json"),
        Path(f"{archive}.manifest.json"),
        Path(f"{archive}.manifest.json.sig"),
    )
    companions = tuple(require_real_file(path, "release_companion") for path in companions)
    if any(path.parent != release_dir for path in companions):
        raise ValueError("embedded_runtime_release_companion_escape")
    manifest = json.loads(companions[3].read_text(encoding="utf-8"))
    release = manifest.get("release") if isinstance(manifest, dict) else None
    if not isinstance(release, dict) or release.get("target") != target:
        raise ValueError("embedded_runtime_manifest_target_mismatch")
    if release.get("package_root") != artifact_id:
        raise ValueError("embedded_runtime_manifest_package_root_mismatch")

    if output.exists():
        shutil.rmtree(output)
    release_output = output / "release"
    bootstrap_output = output / "bootstrap"
    release_output.mkdir(parents=True)
    for path in companions:
        shutil.copy2(path, release_output / path.name)
    for relative in BOOTSTRAP_FILES:
        source = require_real_file(source_root / relative, "bootstrap_file")
        destination = bootstrap_output / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
    python_source = require_real_directory(
        python_runtime_root, "bootstrap_python_runtime"
    )
    python_output = bootstrap_output / "toolchains/python"
    shutil.copytree(python_source, python_output, symlinks=False)
    python_bin = python_output / "bin/python3"
    try:
        resolved_python = python_bin.resolve(strict=True)
    except OSError as error:
        raise ValueError("bootstrap_python_runtime_invalid") from error
    if not resolved_python.is_file() or not resolved_python.is_relative_to(
        python_output.resolve()
    ):
        raise ValueError("bootstrap_python_runtime_invalid")
    try:
        subprocess.run(
            [str(python_bin), "-I", "-c", "import json, tomllib"],
            check=True,
            timeout=30,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ValueError("bootstrap_python_runtime_invalid") from error

    descriptor = {
        "schema_version": 1,
        "target": target,
        "platform": platform,
        "release_tag": release_tag,
        "archive": archive.name,
        "manifest": companions[3].name,
        "manifest_sha256": sha256(companions[3]),
    }
    output.mkdir(parents=True, exist_ok=True)
    (output / "bundle.json").write_text(
        json.dumps(descriptor, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return descriptor


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--release-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--release-tag", required=True)
    parser.add_argument("--python-runtime-root", type=Path, required=True)
    args = parser.parse_args()
    descriptor = stage(
        args.source_root,
        args.release_dir,
        args.output,
        args.target,
        args.release_tag,
        args.python_runtime_root,
    )
    print(json.dumps(descriptor, sort_keys=True))


if __name__ == "__main__":
    main()
