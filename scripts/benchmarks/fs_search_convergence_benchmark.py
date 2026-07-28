#!/usr/bin/env python3
"""Disposable end-to-end benchmark for the fs_search convergence contract."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import resource
import shutil
import statistics
import subprocess
import tempfile
import time
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
PACKAGE_ROOT = ROOT / "data" / "skill-packages" / "fs_search"


def percentile(samples: list[float], quantile: float) -> float:
    ordered = sorted(samples)
    index = max(0, min(len(ordered) - 1, int(round((len(ordered) - 1) * quantile))))
    return round(ordered[index], 3)


def current_binary() -> Path:
    current = json.loads((PACKAGE_ROOT / "current.json").read_text(encoding="utf-8"))
    binary = (
        PACKAGE_ROOT
        / "versions"
        / current["install_dir"]
        / "runtime"
        / "bin"
        / "fs-search-skill"
    )
    if not binary.is_file():
        raise RuntimeError(
            "fs_search is not installed; run scripts/skill_calls/call_fs_search.sh "
            "--auto-build once"
        )
    return binary


def create_fixture(root: Path, visible_files: int, ignored_files: int) -> None:
    (root / ".git").mkdir()
    (root / "ignored" / "vendor").mkdir(parents=True)
    (root / ".hidden").mkdir()
    (root / ".gitignore").write_text("ignored/\n", encoding="utf-8")
    for index in range(visible_files):
        bucket = root / "src" / f"bucket-{index % 16:02d}"
        bucket.mkdir(parents=True, exist_ok=True)
        marker = "needle" if index % 7 == 0 else "ordinary"
        (bucket / f"repeated-{index:05d}.txt").write_text(
            f"line one\n{marker} value {index}\nline three\n", encoding="utf-8"
        )
    for index in range(ignored_files):
        (root / "ignored" / "vendor" / f"ignored-{index:05d}.txt").write_text(
            "needle ignored\n", encoding="utf-8"
        )
    deep = root
    for depth in range(12):
        deep /= f"level-{depth:02d}"
    deep.mkdir(parents=True)
    (deep / "deep-needle.txt").write_text("needle deep\n", encoding="utf-8")
    (root / "src" / "latin1.txt").write_bytes(b"caf\xe9 needle\n")
    (root / "src" / "binary.bin").write_bytes(b"\x00needle\x00")


def storage_context(database_path: Path) -> dict[str, Any]:
    return {
        "skill_storage": {
            "storage_kind": "sqlite",
            "skill_name": "fs_search",
            "schema_version": 1,
            "database_path": str(database_path),
        }
    }


def invoke(
    binary: Path,
    args: dict[str, Any],
    context: dict[str, Any],
    path_value: str,
    sequence: int,
) -> tuple[dict[str, Any], float, int]:
    request = {
        "request_id": f"fs-search-benchmark-{sequence}",
        "args": args,
        "context": context,
        "user_id": 0,
        "chat_id": 0,
    }
    environment = os.environ.copy()
    environment["WORKSPACE_ROOT"] = str(ROOT)
    environment["SKILL_TIMEOUT_SECONDS"] = "30"
    environment["PATH"] = path_value
    started = time.perf_counter_ns()
    completed = subprocess.run(
        [str(binary)],
        input=json.dumps(request, separators=(",", ":")) + "\n",
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=environment,
        timeout=35,
        check=False,
    )
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    if completed.returncode != 0:
        raise RuntimeError(
            f"skill exited {completed.returncode}: {completed.stderr[:400]}"
        )
    lines = completed.stdout.splitlines()
    if len(lines) != 1:
        raise RuntimeError(f"expected one JSON line, received {len(lines)}")
    response = json.loads(lines[0])
    if response.get("status") != "ok":
        raise RuntimeError(f"skill error: {response.get('error_text')}")
    return response, elapsed_ms, len(lines[0].encode("utf-8"))


def summarize(name: str, records: list[tuple[dict[str, Any], float, int]]) -> dict[str, Any]:
    elapsed = [record[1] for record in records]
    output_bytes = [record[2] for record in records]
    last = records[-1][0].get("extra", {})
    scan = last.get("scan", {})
    return {
        "name": name,
        "runs": len(records),
        "p50_ms": percentile(elapsed, 0.50),
        "p95_ms": percentile(elapsed, 0.95),
        "output_bytes_p50": int(statistics.median(output_bytes)),
        "backend": scan.get("backend"),
        "backend_version": scan.get("backend_version"),
        "fallback_reason": scan.get("backend_fallback_reason"),
        "cache_reused": last.get("cache_reused"),
        "completeness": last.get("completeness"),
        "known_match_count": last.get("known_match_count"),
        "visited_entries": scan.get("visited_entries"),
        "observation_bytes": last.get("observation_bytes"),
    }


def raw_rg_reference(fixture: Path, repetitions: int) -> list[dict[str, Any]]:
    rg = shutil.which("rg")
    if not rg:
        return [{"name": "host_rg", "available": False}]
    scenarios = [
        ("host_rg_files", [rg, "--files", "--null", "."]),
        (
            "host_rg_content",
            [rg, "--json", "--fixed-strings", "--", "needle", "."],
        ),
    ]
    reports = []
    for name, command in scenarios:
        samples: list[float] = []
        output_sizes: list[int] = []
        for _ in range(repetitions):
            started = time.perf_counter_ns()
            completed = subprocess.run(
                command,
                cwd=fixture,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=30,
                check=False,
            )
            samples.append((time.perf_counter_ns() - started) / 1_000_000)
            output_sizes.append(len(completed.stdout))
            if completed.returncode not in (0, 1):
                raise RuntimeError(f"host rg exited {completed.returncode}")
        reports.append(
            {
                "name": name,
                "available": True,
                "runs": repetitions,
                "p50_ms": percentile(samples, 0.50),
                "p95_ms": percentile(samples, 0.95),
                "output_bytes_p50": int(statistics.median(output_sizes)),
            }
        )
    return reports


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repetitions", type=int, default=7)
    parser.add_argument("--visible-files", type=int, default=320)
    parser.add_argument("--ignored-files", type=int, default=900)
    parser.add_argument("--output", type=Path)
    options = parser.parse_args()
    repetitions = max(3, options.repetitions)
    binary = current_binary()
    temp_parent = ROOT / "data" / "tmp"
    temp_parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="fs-search-benchmark-", dir=temp_parent) as raw:
        benchmark_root = Path(raw)
        fixture = benchmark_root / "repository"
        fixture.mkdir()
        create_fixture(fixture, options.visible_files, options.ignored_files)
        relative_root = fixture.relative_to(ROOT).as_posix()
        context = storage_context(benchmark_root / "cache.sqlite3")
        normal_path = os.environ.get("PATH", "")
        no_rg_path = str(fixture / "missing-path")
        find_args = {
            "action": "find_ext",
            "root": relative_root,
            "ext": "txt",
            "max_results": 32,
        }
        grep_args = {
            "action": "grep_text",
            "root": relative_root,
            "query": "needle",
            "pattern_kind": "literal",
            "output_mode": "content",
            "globs": ["**/*.txt"],
            "max_results": 32,
        }
        scenarios: dict[str, list[tuple[dict[str, Any], float, int]]] = {
            "find_ripgrep_cold": [],
            "find_ripgrep_cached_page": [],
            "find_rust_fallback": [],
            "grep_ripgrep": [],
            "grep_rust_fallback": [],
        }
        sequence = 0
        for _ in range(repetitions):
            sequence += 1
            cold = invoke(binary, find_args, context, normal_path, sequence)
            scenarios["find_ripgrep_cold"].append(cold)
            cold_extra = cold[0].get("extra", {})
            cursor = cold_extra.get("page", {}).get("next_cursor") or cold_extra.get(
                "continuation", {}
            ).get("cursor")
            if not cursor:
                raise RuntimeError("find benchmark did not produce a continuation cursor")
            sequence += 1
            cached_args = dict(find_args)
            cached_args["cursor"] = cursor
            scenarios["find_ripgrep_cached_page"].append(
                invoke(binary, cached_args, context, normal_path, sequence)
            )
            sequence += 1
            scenarios["find_rust_fallback"].append(
                invoke(binary, find_args, context, no_rg_path, sequence)
            )
            sequence += 1
            scenarios["grep_ripgrep"].append(
                invoke(binary, grep_args, context, normal_path, sequence)
            )
            sequence += 1
            scenarios["grep_rust_fallback"].append(
                invoke(binary, grep_args, context, no_rg_path, sequence)
            )
        maximum_rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        report = {
            "schema_version": 1,
            "fixture": {
                "visible_files": options.visible_files + 3,
                "ignored_files": options.ignored_files,
                "deep_levels": 12,
                "disposable": True,
            },
            "skill_binary": str(binary.relative_to(ROOT)),
            "repetitions": repetitions,
            "scenarios": [summarize(name, rows) for name, rows in scenarios.items()],
            "host_reference": raw_rg_reference(fixture, repetitions),
            "child_max_rss": maximum_rss,
            "child_max_rss_unit": "KiB on Linux; bytes on macOS",
        }
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if options.output:
        options.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
