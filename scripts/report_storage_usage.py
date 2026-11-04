#!/usr/bin/env python3
"""Report metadata-only file usage and optional snapshot deltas; never clean data.

Use --root for a workspace, its data directory, or the enclosing mounted volume.
Nested symlinks/cross-filesystem mounts are excluded. Snapshots are observations,
not cleanup grants. Example: --root /path/to/data --output before.json, then
--root /path/to/data --baseline before.json --output after.json.
"""
from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import shutil
import stat
import sys
import tempfile
import time

KIND = "storage_usage_report"


class ReportError(ValueError):
    def __init__(self, code: str):
        self.code = code
        super().__init__(code)


def capacity(path: Path) -> dict:
    space = shutil.disk_usage(path)
    return {"total_bytes": space.total, "used_bytes": space.used, "available_bytes": space.free}


def inventory(root: Path) -> dict:
    root = root.resolve(strict=True)
    metadata = root.stat()
    if not root.is_dir():
        raise ReportError("storage_report_root_not_directory")
    started = time.time()
    global_seen: set[tuple[int, int]] = set()
    directories_seen = {(metadata.st_dev, metadata.st_ino)}
    groups = {}
    unique_bytes, unique_allocated = 0, 0
    allocation_known = True
    with os.scandir(root) as entries:
        names = sorted(entry.name for entry in entries)
    for name in names:
        row = {"apparent_bytes": 0, "allocated_bytes": 0, "regular_files": 0,
               "symlinks_skipped": 0, "mounts_skipped": 0, "directory_aliases_skipped": 0,
               "special_files_skipped": 0, "errors": 0}
        seen: set[tuple[int, int]] = set()
        pending = [root/name]
        while pending:
            path = pending.pop()
            try:
                item = path.lstat()
                if stat.S_ISLNK(item.st_mode):
                    row["symlinks_skipped"] += 1
                elif item.st_dev != metadata.st_dev:
                    row["mounts_skipped"] += 1
                elif stat.S_ISDIR(item.st_mode):
                    key = (item.st_dev, item.st_ino)
                    if key in directories_seen:
                        row["directory_aliases_skipped"] += 1
                        continue
                    directories_seen.add(key)
                    with os.scandir(path) as children:
                        pending.extend(path/child.name for child in children)
                elif stat.S_ISREG(item.st_mode):
                    key = (item.st_dev, item.st_ino)
                    allocated = item.st_blocks * 512 if hasattr(item, "st_blocks") else None
                    if key not in seen:
                        seen.add(key)
                        row["regular_files"] += 1
                        row["apparent_bytes"] += item.st_size
                        row["allocated_bytes"] = (row["allocated_bytes"] + allocated
                            if row["allocated_bytes"] is not None and allocated is not None else None)
                    if key not in global_seen:
                        global_seen.add(key)
                        unique_bytes += item.st_size
                        if allocated is None:
                            allocation_known = False
                        else:
                            unique_allocated += allocated
                else:
                    row["special_files_skipped"] += 1
            except OSError:
                row["errors"] += 1
        groups[name] = row
    return {"schema_version": 1, "kind": KIND, "root": str(root),
            "root_identity": {"device": metadata.st_dev, "inode": metadata.st_ino},
            "scan_started_at": started, "scan_finished_at": time.time(), "atomic_snapshot": False,
            "scope": "same_filesystem_regular_files_only", "group_sizes_additive": False,
            "complete": all(row["errors"] == 0 for row in groups.values()),
            "partial_counts_are_lower_bounds": any(row["errors"] for row in groups.values()),
            "filesystem": capacity(root), "system_filesystem": capacity(Path(root.anchor)),
            "totals": {"apparent_bytes": unique_bytes,
                       "allocated_bytes": unique_allocated if allocation_known else None,
                       "unique_regular_files": len(global_seen)}, "groups": groups}


def validate_report(value: object) -> dict:
    if not isinstance(value, dict) or value.get("kind") != KIND or value.get("schema_version") != 1:
        raise ReportError("storage_report_invalid_snapshot")
    if not isinstance(value.get("groups"), dict) or not isinstance(value.get("complete"), bool):
        raise ReportError("storage_report_invalid_snapshot")
    identity = value.get("root_identity")
    stamp = value.get("scan_finished_at")
    if (not isinstance(value.get("root"), str) or not isinstance(identity, dict)
        or any(type(identity.get(key)) is not int or identity[key] < 0 for key in ("device", "inode"))
        or type(stamp) not in (int, float) or not math.isfinite(stamp) or stamp < 0):
        raise ReportError("storage_report_invalid_snapshot")
    for row in value["groups"].values():
        if not isinstance(row, dict):
            raise ReportError("storage_report_invalid_snapshot")
        for key in ("apparent_bytes", "allocated_bytes", "errors"):
            number = row.get(key)
            if number is None and key == "allocated_bytes":
                continue
            if type(number) is not int or number < 0:
                raise ReportError("storage_report_invalid_snapshot")
    return value


def compare(current: dict, previous: dict) -> dict:
    validate_report(previous)
    if current["root"] != previous.get("root") or current["root_identity"] != previous.get("root_identity"):
        raise ReportError("storage_report_baseline_scope_mismatch")
    changes = {}
    for name in sorted(current["groups"].keys() | previous["groups"].keys()):
        now, before = current["groups"].get(name), previous["groups"].get(name)
        known = (now is not None or current["complete"]) and (before is not None or previous["complete"])
        known = known and not (now or {}).get("errors") and not (before or {}).get("errors")
        delta = {}
        for key in ("apparent_bytes", "allocated_bytes"):
            a = now[key] if now else 0
            b = before[key] if before else 0
            delta[key+"_delta"] = a-b if known and a is not None and b is not None else None
        changes[name] = delta
    return {"baseline_scan_finished_at": previous["scan_finished_at"], "groups": changes,
            "note": "Directory deltas are non-additive when hardlinks cross groups; filesystem use includes other paths and metadata."}


def save_report(path: Path, report: dict) -> None:
    if path.suffix != ".json" or path.is_symlink():
        raise ReportError("storage_report_output_must_be_json_report")
    if path.exists():
        old = validate_report(json.loads(path.read_text()))
        if old["root"] != report["root"] or old["root_identity"] != report["root_identity"]:
            raise ReportError("storage_report_output_scope_mismatch")
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent, delete=False) as output:
        temporary = Path(output.name)
        try:
            json.dump(report, output, ensure_ascii=False, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--baseline", type=Path, help="compare an earlier report for this exact root")
    parser.add_argument("--output", type=Path, help="save a private JSON report, never overwrite unrelated files")
    args = parser.parse_args(argv)
    try:
        if sys.platform not in ("linux", "darwin"):
            raise ReportError("storage_report_unsupported_platform")
        previous = validate_report(json.loads(args.baseline.read_text())) if args.baseline else None
        report = inventory(args.root)
        if previous is not None:
            report["comparison"] = compare(report, previous)
        if args.output:
            save_report(args.output, report)
        print(json.dumps(report, ensure_ascii=False))
        return 0 if report["complete"] else 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({"kind": KIND, "status": "error", "error_code": error.code if isinstance(error, ReportError) else "storage_report_failed",
                          "hint": "Check root access, baseline scope/schema, and the report output path; no data was cleaned."}))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
