#!/usr/bin/env python3
"""Project selected skills into verified, platform-specific receipts.

Cargo skills adopt exactly the binary produced by the ordinary workspace
build, avoiding a second compilation. Other adapters run only their manifest-
selected installer. Every path then performs protocol smoke and atomic receipt
activation serially; network remains denied.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

from skill_store_packages import arch_for_target, platform_for_target, runner_specs, supports_platform


ROOT = Path(__file__).resolve().parents[1]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--registry", type=Path, default=ROOT / "configs/skills_registry.toml")
    parser.add_argument("--binary-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--package-root", type=Path, default=ROOT / "data/skill-packages")
    parser.add_argument("--sdk-cli", type=Path, default=ROOT / "target/release/rustclaw-skill")
    parser.add_argument("--skill", action="append", default=[])
    parser.add_argument("--target", default="host")
    parser.add_argument(
        "--scope",
        choices=("proactive", "platform-precompiled"),
        default="proactive",
    )
    return parser.parse_args()


def run_projection(args: argparse.Namespace, spec: object) -> dict[str, object]:
    if spec.adapter == "cargo":
        command = [
            str(args.sdk_cli),
            "adopt-built",
            str(spec.manifest_path),
            str(ROOT),
            str(args.package_root),
            str(args.binary_dir / spec.runner),
        ]
    else:
        command = [
            str(args.sdk_cli),
            "install-local",
            str(spec.manifest_path),
            str(ROOT),
            str(args.package_root),
        ]
    if args.target != "host":
        command.extend(["--target", args.target])
    completed = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=180,
    )
    try:
        payload = json.loads(completed.stdout.strip())
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"receipt projection returned invalid JSON for {spec.skill_name}: "
            f"exit={completed.returncode} stdout_bytes={len(completed.stdout)} "
            f"stderr_bytes={len(completed.stderr)}"
        ) from error
    if completed.returncode != 0 or payload.get("ok") is not True:
        error = payload.get("error") if isinstance(payload, dict) else None
        raise RuntimeError(f"receipt projection failed for {spec.skill_name}: {error}")
    return payload


def main() -> int:
    args = parse_args()
    if not args.sdk_cli.is_file():
        print(f"skill receipt CLI is missing: {args.sdk_cli}", file=sys.stderr)
        return 1
    selected = set(args.skill)
    os_name = platform_for_target(args.target)
    arch = arch_for_target(args.target)
    projected = 0
    skipped = 0
    failures: list[str] = []
    try:
        for spec in runner_specs(args.registry):
            in_scope = (
                spec.install_mode != "on_demand"
                if args.scope == "proactive"
                else spec.install_mode == "on_demand" and spec.adapter == "cargo"
            )
            if not in_scope or (selected and spec.skill_name not in selected):
                continue
            if not supports_platform(spec, os_name, arch):
                skipped += 1
                continue
            if spec.adapter == "cargo":
                binary = args.binary_dir / spec.runner
                if not binary.is_file():
                    raise RuntimeError(f"built skill binary is missing: {binary}")
            try:
                payload = run_projection(args, spec)
            except (OSError, RuntimeError, subprocess.SubprocessError) as error:
                failures.append(f"{spec.skill_name}: {error}")
                print(
                    json.dumps(
                        {"skill_name": spec.skill_name, "ok": False, "error": str(error)},
                        separators=(",", ":"),
                        sort_keys=True,
                    )
                )
                continue
            outcome = payload.get("install", {})
            print(
                json.dumps(
                    {
                        "skill_name": spec.skill_name,
                        "adapter": spec.adapter,
                        "receipt_digest": outcome.get("receipt_digest"),
                        "reused": outcome.get("reused", False),
                    },
                    separators=(",", ":"),
                    sort_keys=True,
                )
            )
            projected += 1
    except (OSError, RuntimeError, subprocess.SubprocessError, ValueError) as error:
        print(f"SKILL_RECEIPT_PROJECTION failed: {error}", file=sys.stderr)
        return 1
    if failures:
        print(
            f"SKILL_RECEIPT_PROJECTION failed count={len(failures)} "
            f"projected={projected} skipped_platform={skipped}",
            file=sys.stderr,
        )
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print(
        f"SKILL_RECEIPT_PROJECTION ok projected={projected} skipped_platform={skipped}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
