#!/usr/bin/env python3
"""Aggregate deterministic, restart-continuity, and live-NL lifecycle evidence."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROLES = ("deterministic", "restart_continuity", "live_nl")


class AggregateError(RuntimeError):
    pass


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Combine the three long-running lifecycle evidence classes into one summary.",
    )
    parser.add_argument("--deterministic", type=Path, required=True)
    parser.add_argument("--restart-continuity", type=Path, required=True)
    parser.add_argument("--live-nl", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args(argv)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def command_stdout(command: list[str]) -> str | None:
    completed = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    output = completed.stdout.strip()
    return output if completed.returncode == 0 and output else None


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def path_ref(path: Path) -> str:
    return os.path.relpath(path.resolve(), ROOT)


def read_json(path: Path) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise AggregateError(f"invalid JSON evidence: {path_ref(path)}") from error
    if not isinstance(payload, dict):
        raise AggregateError(f"JSON evidence must be an object: {path_ref(path)}")
    return payload


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise AggregateError(f"cannot read JSONL evidence: {path_ref(path)}") from error
    for line_number, line in enumerate(lines, start=1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as error:
            raise AggregateError(
                f"invalid JSONL evidence: {path_ref(path)} line {line_number}"
            ) from error
        if not isinstance(row, dict):
            raise AggregateError(
                f"JSONL row must be an object: {path_ref(path)} line {line_number}"
            )
        rows.append(row)
    return rows


def object_counts(payload: dict[str, Any]) -> tuple[int, int, int]:
    counts = payload.get("case_counts")
    if isinstance(counts, dict):
        total = int(counts.get("total") or 0)
        passed = int(counts.get("passed") or 0)
        failed = int(counts.get("failed") or 0)
        if total != passed + failed:
            raise AggregateError("case_counts total does not equal passed + failed")
        return total, passed, failed
    cases = payload.get("cases")
    if isinstance(cases, dict):
        passed = sum(
            1
            for value in cases.values()
            if isinstance(value, dict) and value.get("status") == "pass"
        )
        return len(cases), passed, len(cases) - passed
    status = str(payload.get("status") or "")
    return 1, 1 if status == "pass" else 0, 0 if status == "pass" else 1


def nl_row_passed(row: dict[str, Any]) -> bool:
    assertion = str(row.get("assertion") or "").lower()
    result = str(row.get("result") or "").lower()
    return assertion == "pass" or result == "pass"


def source_summary(role: str, path: Path) -> dict[str, Any]:
    resolved = path.expanduser().resolve()
    if not resolved.is_file():
        raise AggregateError(f"{role} evidence does not exist: {path_ref(resolved)}")
    if role == "live_nl" or resolved.suffix == ".jsonl":
        rows = read_jsonl(resolved)
        total = len(rows)
        passed = sum(1 for row in rows if nl_row_passed(row))
        failed = total - passed
        status = "pass" if total > 0 and failed == 0 else "fail"
        source_commit = None
    else:
        payload = read_json(resolved)
        total, passed, failed = object_counts(payload)
        status = "pass" if payload.get("status") == "pass" and failed == 0 else "fail"
        source_commit = payload.get("source_commit")
    return {
        "role": role,
        "path": path_ref(resolved),
        "sha256": sha256_file(resolved),
        "status": status,
        "case_counts": {"total": total, "passed": passed, "failed": failed},
        "source_commit": source_commit,
    }


def aggregate(paths: dict[str, Path]) -> dict[str, Any]:
    sources = [source_summary(role, paths[role]) for role in SOURCE_ROLES]
    total = sum(source["case_counts"]["total"] for source in sources)
    passed = sum(source["case_counts"]["passed"] for source in sources)
    failed = sum(source["case_counts"]["failed"] for source in sources)
    source_commit = command_stdout(["git", "rev-parse", "HEAD"])
    return {
        "schema_version": 1,
        "suite": "long_running_lifecycle_linux",
        "status": "pass" if failed == 0 and all(source["status"] == "pass" for source in sources) else "fail",
        "source_commit": source_commit,
        "source_commit_pushed": (
            source_commit is not None
            and source_commit == command_stdout(["git", "rev-parse", "origin/main"])
        ),
        "platform": platform.system().lower(),
        "arch": platform.machine().lower(),
        "generated_at": utc_now(),
        "case_counts": {"total": total, "passed": passed, "failed": failed},
        "sources": sources,
        "redaction": {
            "status": "pass",
            "raw_prompts_embedded": False,
            "credentials_embedded": False,
            "source_paths_are_relative": True,
        },
    }


def validate_output_secrets(summary: dict[str, Any]) -> None:
    sys.path.insert(0, str(ROOT / "scripts" / "nl_tests"))
    from secret_scan import secret_scan_findings

    findings = secret_scan_findings(summary)
    if findings:
        raise AggregateError(f"aggregate secret scan failed with {len(findings)} finding(s)")


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        summary = aggregate(
            {
                "deterministic": args.deterministic,
                "restart_continuity": args.restart_continuity,
                "live_nl": args.live_nl,
            }
        )
        validate_output_secrets(summary)
        output = args.output.expanduser().resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(
            json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        print(json.dumps(summary, ensure_ascii=False, sort_keys=True))
        return 0 if summary["status"] == "pass" else 1
    except AggregateError as error:
        print(f"LONG_RUNNING_LIFECYCLE_AGGREGATE_FAIL {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
