#!/usr/bin/env python3
"""Guard proactive build scripts against compiling Skill Store packages."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

from skill_store_packages import on_demand_pairs


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_SNIPPETS = {
    "build-all.sh": (
        "scripts/skill_store_packages.py\" --format pairs",
        "CARGO_WORKSPACE_ARGS+=(--exclude \"$package\")",
    ),
    "install-rustclaw-cmd.sh": (
        "scripts/skill_store_packages.py\" --format packages",
        "bash ./build-all.sh no-ui --target",
    ),
    "package-release.sh": (
        "scripts/skill_store_packages.py\" --format packages",
        "pkg.get(\"name\") in on_demand_packages",
    ),
    "cross-build-upload.sh": ("bash ./build-all.sh no-ui --target",),
    "cross-build-upload-cloud.sh": ("bash ./build-all.sh no-ui --target",),
    "cross-build-pi.sh": ("bash \"${SCRIPT_DIR}/build-all.sh\"",),
    "local-cross-build-upload-pi.sh": (
        "scripts/skill_store_packages.py\" --format packages",
        "build-all.sh\" no-ui --target",
    ),
    "setup-config.sh": ("scripts/skill_store_packages.py\" --format runners",),
    "start-all.sh": ("scripts/skill_store_packages.py\" --format runners",),
}

DIRECT_WORKSPACE_BUILD_FORBIDDEN = (
    "install-rustclaw-cmd.sh",
    "cross-build-upload.sh",
    "cross-build-upload-cloud.sh",
    "local-cross-build-upload-pi.sh",
)


def cargo_package_targets() -> dict[str, set[str]]:
    completed = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(completed.stdout)
    workspace_members = set(metadata.get("workspace_members", []))
    return {
        package["name"]: {
            target["name"]
            for target in package.get("targets", [])
            if "bin" in target.get("kind", [])
        }
        for package in metadata.get("packages", [])
        if package.get("id") in workspace_members
    }


def main() -> int:
    errors: list[str] = []
    pairs = on_demand_pairs(ROOT / "configs/skills_registry.toml")
    if not pairs:
        errors.append("registry has no on-demand Skill Store packages")
    try:
        package_targets = cargo_package_targets()
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        errors.append(f"cargo metadata unavailable: {error}")
        package_targets = {}
    for package, runner in pairs:
        if package not in package_targets:
            errors.append(f"registry install_package is not a workspace package: {package}")
        elif runner not in package_targets[package]:
            errors.append(f"registry runner is not a binary target of {package}: {runner}")
    for relative, snippets in REQUIRED_SNIPPETS.items():
        raw = (ROOT / relative).read_text(encoding="utf-8")
        for snippet in snippets:
            if snippet not in raw:
                errors.append(f"{relative}: missing contract snippet: {snippet}")
    for relative in DIRECT_WORKSPACE_BUILD_FORBIDDEN:
        raw = (ROOT / relative).read_text(encoding="utf-8")
        if "cargo build --workspace" in raw:
            errors.append(f"{relative}: direct workspace build bypasses registry exclusions")

    if errors:
        for error in errors:
            print(f"ON_DEMAND_SKILL_BUILD_CHECK error={error}")
        return 1
    print(f"ON_DEMAND_SKILL_BUILD_CHECK ok packages={len(pairs)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
