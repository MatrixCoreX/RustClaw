#!/usr/bin/env python3
"""Preview verified Cargo intermediates; --apply removes only those caches.

Runtime data, media, logs, installed dependencies, models, receipts, executables
and deployment backups are never cleanup targets. Builds run in CI, not here.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def default_targets() -> list[Path]:
    target = ROOT / "target"
    if target.is_symlink() or not target.is_dir():
        return []
    roots = [target]
    roots.extend(p for p in target.iterdir() if p.is_dir() and not p.is_symlink()
                 and re.fullmatch(r"(?:aarch64|x86_64|armv7|i686|riscv64)[a-zA-Z0-9_.-]*", p.name))
    targets = []
    for root in roots:
        for profile in ("debug", "release"):
            base = root / profile
            if base.is_symlink() or not base.is_dir():
                continue
            targets.extend(base / name for name in (".fingerprint", "incremental") if (base / name).exists())
            deps = base / "deps"
            if deps.is_dir() and not deps.is_symlink():
                targets.extend(p for p in deps.iterdir() if p.suffix in {".rlib", ".rmeta", ".o", ".d"})
    return sorted(targets)


def verified_files(path: Path) -> list[Path]:
    """Reject links, special files and mounts before allowing any deletion."""
    if path.resolve(strict=True) != path.absolute() or not path.is_relative_to(ROOT / "target"):
        raise RuntimeError("cleanup_unsafe_path")
    expected_device = ROOT.stat().st_dev
    files = []
    for item in [path, *path.rglob("*")] if path.is_dir() else [path]:
        metadata = item.lstat()
        if metadata.st_dev != expected_device or not (stat.S_ISREG(metadata.st_mode) or stat.S_ISDIR(metadata.st_mode)):
            raise RuntimeError("cleanup_unsafe_file_type")
        if stat.S_ISREG(metadata.st_mode):
            files.append(item)
    return files


def require_idle(targets: list[Path]) -> None:
    if sys.platform not in {"linux", "darwin"}:
        raise RuntimeError("cleanup_unsupported_platform")
    processes = subprocess.run(["ps", "-axo", "comm="], text=True, capture_output=True, check=True)
    if any(Path(p.strip()).name in {"cargo", "rustc", "rustdoc", "cmake", "ninja", "make"}
           for p in processes.stdout.splitlines()):
        raise RuntimeError("cleanup_build_active")
    if not targets:
        return
    if not shutil.which("lsof"):
        raise RuntimeError("cleanup_reference_audit_unavailable")
    for target in targets:
        args = ["lsof", "-nP", "-Fpn"]
        args += ["+D", str(target)] if target.is_dir() else ["--", str(target)]
        result = subprocess.run(args, text=True, capture_output=True)
        # Only unrelated desktop FUSE mount warnings are allowed. Never log
        # raw process arguments. Failed or incomplete target audits block apply.
        if any(line and not (line.startswith("lsof: WARNING: can't stat() fuse.") or
                             line.strip() == "Output information may be incomplete.")
               for line in result.stderr.splitlines()):
            raise RuntimeError("cleanup_reference_audit_failed")
        if result.returncode != 1 or result.stdout.strip():
            raise RuntimeError("cleanup_cache_in_use")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="remove verified inactive compiler intermediates")
    parser.add_argument("--json", action="store_true", help="print the preview/result and filesystem space as JSON")
    args = parser.parse_args(argv)
    report: dict = {"schema_version": 1, "mode": "apply" if args.apply else "preview",
                    "targets": [], "removed": [], "error_code": None}
    try:
        targets = default_targets()
        for path in targets:
            files = verified_files(path)
            report["targets"].append({"path": str(path.relative_to(ROOT)),
                                      "bytes": sum(p.stat().st_size for p in files)})
        if args.apply:
            require_idle(targets)
            for path in targets:
                if path not in default_targets():
                    raise RuntimeError("cleanup_target_changed")
                verified_files(path)
                require_idle([path])
                if path.is_dir():
                    shutil.rmtree(path)
                else:
                    path.unlink()
                report["removed"].append(str(path.relative_to(ROOT)))
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        report["error_code"] = str(error) if isinstance(error, RuntimeError) else "cleanup_audit_failed"
    space = shutil.disk_usage(ROOT)
    report["filesystem"] = {"total_bytes": space.total, "used_bytes": space.used, "free_bytes": space.free}
    report["candidate_bytes"] = sum(row["bytes"] for row in report["targets"])
    if args.json:
        print(json.dumps(report))
    else:
        for row in report["targets"]:
            print(f"{row['bytes']:>12}  {row['path']}")
        print(f"DEV_ARTIFACT_CLEAN {report['mode']} targets={len(report['targets'])} bytes={report['candidate_bytes']}")
        print(f"FILESYSTEM free_bytes={space.free} total_bytes={space.total}")
        if report["error_code"]:
            print(f"ERROR {report['error_code']}")
    return 1 if report["error_code"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
