#!/usr/bin/env python3
"""Publisher-only wheel preparation; destination devices never build dependencies."""

from __future__ import annotations

import argparse
from email.parser import BytesParser
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import zipfile

from skill_store_packages import arch_for_target, platform_for_target, runner_specs, supports_platform

ROOT = Path(__file__).resolve().parents[1]


def wheel_identity(path: Path, *, require_pure: bool = False) -> tuple[str, str]:
    with zipfile.ZipFile(path) as wheel:
        metadata_paths = [name for name in wheel.namelist()
                          if name.count("/") == 1 and name.endswith(".dist-info/METADATA")]
        wheel_paths = [name for name in wheel.namelist()
                       if name.count("/") == 1 and name.endswith(".dist-info/WHEEL")]
        if len(metadata_paths) != 1 or len(wheel_paths) != 1:
            raise ValueError(f"release_wheel_metadata_invalid: {path.name}")
        metadata = BytesParser().parsebytes(wheel.read(metadata_paths[0]))
        info = BytesParser().parsebytes(wheel.read(wheel_paths[0]))
        if require_pure and info.get("Root-Is-Purelib", "").lower() != "true":
            raise ValueError("release_dependency_requires_native_build")
        name, version = metadata.get("Name", ""), metadata.get("Version", "")
        if not name or not version:
            raise ValueError("release_wheel_identity_missing")
        return re.sub(r"[-_.]+", "-", name).lower(), version


def add_wheel_hashes(lockfile: Path, wheels: Path) -> None:
    hashes: dict[tuple[str, str], set[str]] = {}
    for wheel in sorted(wheels.glob("*.whl")):
        identity = wheel_identity(wheel)
        with wheel.open("rb") as stream:
            hashes.setdefault(identity, set()).add(hashlib.file_digest(stream, "sha256").hexdigest())
    result: list[str] = []
    block: list[str] = []
    for line in lockfile.read_text().splitlines():
        block.append(line)
        if line.rstrip().endswith("\\"):
            continue
        match = re.match(r"^([A-Za-z0-9_.-]+)==([^ ;\\]+)", block[0])
        if match:
            identity = re.sub(r"[-_.]+", "-", match[1]).lower(), match[2]
            for digest in sorted(hashes.get(identity, ())):
                if digest not in "\n".join(block):
                    block[-1] = block[-1].rstrip() + " \\"
                    block.append(f"    --hash=sha256:{digest}")
        result.extend(block)
        block = []
    if block:
        raise ValueError("release_dependency_lock_truncated")
    lockfile.write_text("\n".join(result) + "\n")


def verify_wheels(lockfile: Path, destination: Path) -> None:
    # Resolve the whole graph without an index or any source-build fallback.
    with tempfile.TemporaryDirectory(prefix="release-wheel-verify-") as directory:
        verification_lock = Path(directory) / "requirements.lock"
        shutil.copy2(lockfile, verification_lock)
        add_wheel_hashes(verification_lock, destination)
        subprocess.run([sys.executable, "-m", "pip", "install", "--dry-run",
                        "--target", str(Path(directory) / "install"),
                        "--ignore-installed", "--no-index", "--only-binary=:all:",
                        "--require-hashes", "--find-links", str(destination),
                        "-r", str(verification_lock)], check=True)


def prepare_wheels(lockfile: Path, target: str) -> None:
    os_name, arch = platform_for_target(target), arch_for_target(target)
    if os_name != platform_for_target("host") or arch != arch_for_target("host"):
        raise ValueError("release_wheels_require_native_publisher")
    python_version = f"{sys.version_info.major}.{sys.version_info.minor}"
    if python_version not in {"3.13", "3.14"}:
        raise ValueError("release_wheel_python_version_unsupported")
    original_lock_digest = hashlib.sha256(lockfile.read_bytes()).hexdigest()
    if os_name == "linux":
        platforms = [f"manylinux_2_{minor}_{arch}" for minor in range(28, 16, -1)]
        platforms += [f"manylinux2014_{arch}", f"linux_{arch}"]
    elif os_name == "macos":
        platforms = [f"macosx_13_0_{'arm64' if arch == 'aarch64' else arch}"]
    else:
        raise ValueError("release_wheel_platform_unsupported")
    destination = lockfile.parent / "release-wheels"
    destination.mkdir(exist_ok=True)
    prepared_names: set[str] = set()
    with tempfile.TemporaryDirectory(prefix="release-wheel-download-") as directory:
        downloads = Path(directory)
        command = [sys.executable, "-m", "pip", "download", "--require-hashes", "--no-deps",
                   "--prefer-binary", "--python-version", python_version, "--implementation", "cp",
                   "--abi", "cp" + python_version.replace(".", ""), "--dest", str(downloads), "-r", str(lockfile)]
        for value in platforms:
            command.extend(["--platform", value])
        subprocess.run(command, check=True)
        for dependency in sorted(downloads.iterdir()):
            if dependency.suffix == ".whl":
                wheel_identity(dependency)
                shutil.copy2(dependency, destination / dependency.name)
                prepared_names.add(dependency.name)
                continue
            # Native extension compilation is confined to this matching publisher
            # runner, never the destination device or a cross-architecture host.
            built = downloads / "built"
            built.mkdir(exist_ok=True)
            subprocess.run([sys.executable, "-m", "pip", "wheel", "--no-deps",
                            "--wheel-dir", str(built), str(dependency)], check=True)
            for wheel in built.glob("*.whl"):
                wheel_identity(wheel)
                shutil.copy2(wheel, destination / wheel.name)
                prepared_names.add(wheel.name)
                wheel.unlink()
    verify_wheels(lockfile, destination)
    records = []
    for name in sorted(prepared_names):
        wheel = destination / name
        with wheel.open("rb") as stream:
            records.append({"name": wheel.name, "sha256": hashlib.file_digest(stream, "sha256").hexdigest()})
    (lockfile.parent / "release-wheels.json").write_text(json.dumps({
        "schema_version": 1, "target": target, "python_version": python_version,
        "source_lock_sha256": original_lock_digest, "wheels": records,
    }, sort_keys=True) + "\n")
    print(f"RELEASE_WHEELS_READY target={target} files={len(list(destination.glob('*.whl')))}")


def import_wheels(lockfile: Path, prepared_root: Path, relative: Path, target: str) -> None:
    expected_lock = hashlib.sha256(lockfile.read_bytes()).hexdigest()
    versions = set()
    destination = lockfile.parent / "release-wheels"
    destination.mkdir(exist_ok=True)
    for artifact in sorted(prepared_root.iterdir()):
        package = artifact / relative
        record_path = package / "release-wheels.json"
        if not record_path.is_file():
            continue
        record = json.loads(record_path.read_text())
        if record.get("schema_version") != 1 or record.get("target") != target or record.get("source_lock_sha256") != expected_lock:
            raise ValueError("release_wheel_bundle_mismatch")
        if not record.get("wheels"):
            raise ValueError("release_wheel_bundle_empty")
        versions.add(record["python_version"])
        for item in record["wheels"]:
            name = item["name"]
            if Path(name).name != name or not name.endswith(".whl"):
                raise ValueError("release_wheel_path_invalid")
            source = package / "release-wheels" / name
            if source.is_symlink() or not source.resolve().is_relative_to(prepared_root.resolve()):
                raise ValueError("release_wheel_path_escape")
            with source.open("rb") as stream:
                actual = hashlib.file_digest(stream, "sha256").hexdigest()
            if actual != item["sha256"]:
                raise ValueError("release_wheel_digest_mismatch")
            wheel_identity(source)
            # Pure wheels can have different ZIP timestamps on different runners.
            # Select one complete file and bind its actual hash in the staged lock.
            if not (destination / name).exists():
                shutil.copy2(source, destination / name)
    if versions != {"3.13", "3.14"}:
        raise ValueError("release_wheel_python_coverage_missing")
    add_wheel_hashes(lockfile, destination)
    print(f"RELEASE_WHEELS_IMPORTED target={target} files={len(list(destination.glob('*.whl')))}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--prepared-root", type=Path)
    args = parser.parse_args()
    for spec in runner_specs(ROOT / "configs/skills_registry.toml"):
        if spec.install_mode != "on_demand" or spec.adapter != "python":
            continue
        if not supports_platform(spec, platform_for_target(args.target), arch_for_target(args.target)):
            continue
        manifest = tomllib.loads(spec.manifest_path.read_text())
        relative = spec.manifest_path.parent.relative_to(ROOT)
        lockfile = args.destination / relative / manifest["build"]["lockfile"]
        if args.prepared_root:
            import_wheels(lockfile, args.prepared_root, relative, args.target)
        else:
            prepare_wheels(lockfile, args.target)


if __name__ == "__main__":
    main()
