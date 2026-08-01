#!/usr/bin/env python3
"""Compare one-shot and persistent skill-runner protocol dispatch."""

from __future__ import annotations

import argparse
import json
import os
import resource
import statistics
import subprocess
import threading
import time
from pathlib import Path
from typing import Any


REQUEST_ARGS: dict[str, dict[str, Any]] = {
    "fs_search": {
        "action": "find_name",
        "root": "crates",
        "pattern": "Cargo.toml",
        "max_results": 20,
    },
    "system_basic": {"action": "info"},
    "transform": {
        "action": "transform_data",
        "strict": True,
        "data": [{"value": 2}, {"value": 1}, {"value": 2}],
        "ops": [{"op": "dedup", "fields": ["value"]}],
    },
}


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, int(len(ordered) * fraction + 0.999999) - 1))
    return ordered[index]


def process_tree_rss_kib(root_pid: int) -> int:
    pending = [root_pid]
    seen: set[int] = set()
    total = 0
    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        try:
            status = Path(f"/proc/{pid}/status").read_text(encoding="utf-8")
            for line in status.splitlines():
                if line.startswith("VmRSS:"):
                    total += int(line.split()[1])
                    break
            children = Path(f"/proc/{pid}/task/{pid}/children").read_text(
                encoding="utf-8"
            )
            pending.extend(int(value) for value in children.split())
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    return total


class PeakRssSampler:
    def __init__(self, pid: int) -> None:
        self.pid = pid
        self.peak_kib = 0
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def _run(self) -> None:
        while not self._stop.wait(0.001):
            self.peak_kib = max(self.peak_kib, process_tree_rss_kib(self.pid))

    def __enter__(self) -> "PeakRssSampler":
        self._thread.start()
        return self

    def __exit__(self, *_args: object) -> None:
        self._stop.set()
        self._thread.join()
        self.peak_kib = max(self.peak_kib, process_tree_rss_kib(self.pid))


def runner_env(repo: Path) -> dict[str, str]:
    return {
        "PATH": os.environ.get("PATH", ""),
        "APP_SKILL_PACKAGES_ROOT": str(repo / "data/skill-packages"),
        "WORKSPACE_ROOT": str(repo),
        "SKILL_TIMEOUT_SECONDS": "60",
        "APP_ALLOW_PATH_OUTSIDE_WORKSPACE": "0",
        "APP_ALLOW_SUDO": "0",
        "APP_UNRESTRICTED_ADMIN": "0",
    }


def request_for(repo: Path, skill: str, sequence: int) -> dict[str, Any]:
    skill_root = repo / "data/skill-packages" / skill
    pointer = json.loads((skill_root / "current.json").read_text(encoding="utf-8"))
    receipt = json.loads(
        (skill_root / "versions" / pointer["install_dir"] / "install-receipt.json").read_text(
            encoding="utf-8"
        )
    )
    return {
        "request_id": f"bench-{skill}-{sequence}",
        "user_id": 1,
        "chat_id": 1,
        "user_key": None,
        "skill_name": skill,
        "expected_skill_version": pointer["version"],
        "expected_manifest_digest": receipt["manifest_digest"],
        "expected_receipt_digest": pointer["receipt_digest"],
        "expected_registry_generation": 0,
        "expected_registry_generation_digest": None,
        "expected_base_registry_digest": None,
        "expected_overlay_generation_digest": None,
        "expected_policy_digest": None,
        "expected_admission_receipt_digest": None,
        "args": REQUEST_ARGS[skill],
        "context": {"workspace_root": str(repo)},
    }


def read_final(process: subprocess.Popen[str]) -> dict[str, Any]:
    assert process.stdout is not None
    for line in process.stdout:
        record = json.loads(line)
        if record.get("record_type") != "skill_progress":
            return record
    stderr = process.stderr.read().strip() if process.stderr is not None else ""
    raise RuntimeError(f"runner exited before final response: {stderr}")


def assert_success(response: dict[str, Any], skill: str) -> None:
    if response.get("status") != "ok":
        raise RuntimeError(f"{skill} failed: {json.dumps(response, ensure_ascii=False)}")


def run_cold(repo: Path, runner: Path, skill: str, iterations: int) -> dict[str, Any]:
    durations: list[float] = []
    peak_rss = 0
    usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.perf_counter()
    for sequence in range(iterations):
        process = subprocess.Popen(
            [str(runner)],
            cwd=repo,
            env=runner_env(repo),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        call_started = time.perf_counter()
        with PeakRssSampler(process.pid) as sampler:
            stdout, stderr = process.communicate(
                json.dumps(request_for(repo, skill, sequence), separators=(",", ":")) + "\n"
            )
        duration = (time.perf_counter() - call_started) * 1000
        peak_rss = max(peak_rss, sampler.peak_kib)
        if process.returncode != 0:
            raise RuntimeError(f"runner exit={process.returncode}: {stderr.strip()}")
        finals = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        assert_success(finals[-1], skill)
        durations.append(duration)
    elapsed = time.perf_counter() - started
    usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
    return summarize(
        durations,
        elapsed,
        peak_rss,
        usage_after.ru_utime + usage_after.ru_stime - usage_before.ru_utime - usage_before.ru_stime,
    )


def run_warm(repo: Path, runner: Path, skill: str, iterations: int) -> dict[str, Any]:
    usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
    process = subprocess.Popen(
        [str(runner)],
        cwd=repo,
        env=runner_env(repo),
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    assert process.stdin is not None
    durations: list[float] = []
    started = time.perf_counter()
    with PeakRssSampler(process.pid) as sampler:
        for sequence in range(iterations):
            call_started = time.perf_counter()
            process.stdin.write(
                json.dumps(request_for(repo, skill, sequence), separators=(",", ":")) + "\n"
            )
            process.stdin.flush()
            assert_success(read_final(process), skill)
            durations.append((time.perf_counter() - call_started) * 1000)
        elapsed = time.perf_counter() - started
        process.stdin.close()
        process.wait(timeout=5)
    if process.returncode != 0:
        stderr = process.stderr.read().strip() if process.stderr is not None else ""
        raise RuntimeError(f"runner exit={process.returncode}: {stderr}")
    usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = (
        usage_after.ru_utime
        + usage_after.ru_stime
        - usage_before.ru_utime
        - usage_before.ru_stime
    )
    return summarize(durations, elapsed, sampler.peak_kib, cpu)


def summarize(durations: list[float], elapsed: float, rss_kib: int, cpu: float) -> dict[str, Any]:
    return {
        "iterations": len(durations),
        "latency_ms": {
            "p50": round(statistics.median(durations), 3),
            "p95": round(percentile(durations, 0.95), 3),
            "mean": round(statistics.mean(durations), 3),
        },
        "throughput_per_second": round(len(durations) / elapsed, 3),
        "cpu_seconds": round(cpu, 4),
        "peak_process_tree_rss_kib": rss_kib,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--runner", type=Path)
    parser.add_argument("--iterations", type=int, default=30)
    parser.add_argument("--mode", choices=("cold", "warm", "both"), default="both")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    repo = args.repo.resolve()
    runner = (args.runner or repo / "target/release/skill-runner").resolve()
    result: dict[str, Any] = {"schema_version": 1, "runner": str(runner), "skills": {}}
    for skill in REQUEST_ARGS:
        modes: dict[str, Any] = {}
        if args.mode in ("cold", "both"):
            modes["cold"] = run_cold(repo, runner, skill, args.iterations)
        if args.mode in ("warm", "both"):
            modes["warm"] = run_warm(repo, runner, skill, args.iterations)
        result["skills"][skill] = modes
    encoded = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
    print(encoded, end="")


if __name__ == "__main__":
    main()
