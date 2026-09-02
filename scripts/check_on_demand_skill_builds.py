#!/usr/bin/env python3
"""Guard normal builds and require explicit platform release precompilation."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

from skill_store_packages import on_demand_specs, runner_specs


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_SNIPPETS = {
    ".cargo/config.toml": (
        "[target.x86_64-unknown-linux-gnu]",
        "[target.aarch64-unknown-linux-gnu]",
        'linker = "scripts/rust-linker.sh"',
    ),
    ".dockerignore": (
        "target",
        "release-bin",
    ),
    "build-all.sh": (
        "--scope build-excludes --target \"$target\" --format packages",
        "CARGO_WORKSPACE_ARGS+=(--exclude \"$package\")",
        "configure_cargo_build_environment",
        '--package-root "$SCRIPT_DIR/data/skill-packages"',
    ),
    "install-agent-cmd.sh": (
        "--scope build-excludes --target \"$INSTALL_TARGET\" --format packages",
        "bash ./build-all.sh no-ui --target",
        "configure_cargo_build_environment",
    ),
    "package-release.sh": (
        "--scope build-excludes --target \"$APP_PACKAGE_TARGET\" --format packages",
        "pkg.get(\"name\") in excluded_packages",
        'APP_PACKAGE_TARGET="${APP_PACKAGE_TARGET:-$HOST_RUST_TARGET}"',
        "target/skill-packages/$APP_PACKAGE_TARGET",
        "target/prebuilt-skill-packages/$APP_PACKAGE_TARGET",
        "--scope platform-precompiled --target \"$APP_PACKAGE_TARGET\" --format skills",
        "prebuilt/skill-packages",
    ),
    "scripts/archive/cross-build/cross-build-upload.sh": (
        "bash ./build-all.sh no-ui --target",
        "--scope selected --target \"$TARGET\" --skill \"$1\" --format records",
        'if [[ "$ADAPTER" != "cargo" ]]',
        "configure_cargo_build_environment",
    ),
    "scripts/archive/cross-build/cross-build-upload-cloud.sh": (
        "bash ./build-all.sh no-ui --target",
        "--scope selected --target \"$TARGET\" --skill \"$1\" --format records",
        'if [[ "$ADAPTER" != "cargo" ]]',
        "configure_cargo_build_environment",
    ),
    "scripts/archive/cross-build/cross-build-pi.sh": (
        "bash \"${SCRIPT_DIR}/build-all.sh\"",
        "configure_cargo_build_environment",
    ),
    "scripts/archive/cross-build/local-cross-build-upload-pi.sh": (
        "--scope build-excludes --target \"${TARGET}\" --format packages",
        "build-all.sh\" no-ui --target",
        "configure_cargo_build_environment",
    ),
    "setup-config.sh": (
        "--scope proactive --target host --format specs",
        "cargo build -p \"$package\"",
        "configure_cargo_build_environment",
    ),
    "start-all.sh": (
        "--scope proactive --target host --format specs",
        "configure_cargo_build_environment",
        '[[ -f "$SCRIPT_DIR/Cargo.toml" && -f "$SCRIPT_DIR/Cargo.lock" ]]',
        "receipt-verify",
    ),
    "scripts/skill_calls/_run_skill.sh": (
        "--scope selected --target host --skill \"$SKILL_NAME\" --format records",
        "cargo build -p agent-skill-sdk --release",
        "install-local \"$manifest_path\" \"$ROOT_DIR\" \"$PACKAGE_ROOT\"",
        "configure_cargo_build_environment",
    ),
    "scripts/project_skill_receipts.py": (
        'choices=("proactive", "platform-precompiled")',
        'if spec.adapter == "cargo"',
        '"install-local"',
        'command.extend(["--target", args.target])',
    ),
    "scripts/precompile_skill_store.sh": (
        "--scope platform-precompiled --target \"$TARGET\" --format packages",
        "cargo \"${CARGO_ARGS[@]}\"",
        "--scope platform-precompiled",
        "target/prebuilt-skill-packages/$TARGET",
    ),
    ".github/workflows/ubuntu-x86_64-release.yml": (
        './scripts/precompile_skill_store.sh "${RUST_TARGET}"',
    ),
    ".github/workflows/pi-aarch64-release.yml": (
        "--precompile-skill-store",
        "--scope platform-precompiled --target \"${RUST_TARGET}\" --format skills",
    ),
    ".github/workflows/macos-polyglot-skill-ci.yml": (
        "./scripts/precompile_skill_store.sh host",
        "target/prebuilt-skill-packages/*",
    ),
    "docker/Dockerfile": (
        "--scope build-excludes --target linux --format cargo-excludes",
        "cargo build --release --workspace ${BUILD_EXCLUDES}",
        "/build/runtime-release",
    ),
    "scripts/rust-linker.sh": (
        "bin/gcc-ld/ld.lld",
        'exec clang "-fuse-ld=${bundled_lld}"',
        'exec cc "$@"',
    ),
    "scripts/shell_compat.sh": (
        "cargo_jobs_for_host_capacity",
        '"$available_kb" -ge 8388608',
        '"$cpu_count" -ge 4',
        "configure_cargo_build_environment",
        "CARGO_INCREMENTAL=1",
    ),
}

DIRECT_WORKSPACE_BUILD_FORBIDDEN = (
    "install-agent-cmd.sh",
    "scripts/archive/cross-build/cross-build-upload.sh",
    "scripts/archive/cross-build/cross-build-upload-cloud.sh",
    "scripts/archive/cross-build/local-cross-build-upload-pi.sh",
)

PROACTIVE_BUILD_ENTRYPOINTS = (
    "build-all.sh",
    "install-agent-cmd.sh",
    "package-release.sh",
    "setup-config.sh",
    "start-all.sh",
    "docker/Dockerfile",
)


def cargo_package_targets() -> dict[str, tuple[set[str], Path, set[str]]]:
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
        package["name"]: (
            {
                target["name"]
                for target in package.get("targets", [])
                if "bin" in target.get("kind", [])
            },
            Path(package["manifest_path"]).resolve(),
            {
                dependency["name"]
                for dependency in package.get("dependencies", [])
            },
        )
        for package in metadata.get("packages", [])
        if package.get("id") in workspace_members
    }


def main() -> int:
    errors: list[str] = []
    all_specs = runner_specs(ROOT / "configs/skills_registry.toml")
    specs = on_demand_specs(ROOT / "configs/skills_registry.toml")
    if not specs:
        errors.append("registry has no on-demand Skill Store packages")
    try:
        package_targets = cargo_package_targets()
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        errors.append(f"cargo metadata unavailable: {error}")
        package_targets = {}
    for spec in all_specs:
        if not spec.supported_os:
            errors.append(f"runner skill must declare supported_os: {spec.skill_name}")
        if spec.adapter != "cargo":
            continue
        package_data = package_targets.get(spec.package)
        if package_data is None:
            errors.append(
                f"manifest Cargo package is not a workspace package: {spec.package}"
            )
        else:
            targets, _manifest_path, _dependencies = package_data
            if spec.runner not in targets:
                errors.append(
                    f"registry runner is not a binary target of {spec.package}: {spec.runner}"
                )
    for spec in specs:
        package_data = package_targets.get(spec.package)
        expected_manifest = (ROOT / spec.source_dir / "Cargo.toml").resolve()
        legacy_source = ROOT / "crates" / "skills" / spec.skill_name
        if package_data is not None:
            _targets, manifest_path, _dependencies = package_data
            if manifest_path != expected_manifest:
                errors.append(
                    f"on-demand package source mismatch: {spec.package} "
                    f"expected={expected_manifest} actual={manifest_path}"
                )
        if legacy_source.exists():
            errors.append(
                f"on-demand skill must not remain under core source root: {legacy_source}"
            )
    on_demand_packages = {spec.package for spec in specs}
    for package, (_targets, _manifest_path, dependencies) in package_targets.items():
        if package in on_demand_packages:
            continue
        leaked_dependencies = sorted(dependencies & on_demand_packages)
        if leaked_dependencies:
            errors.append(
                f"proactive workspace package {package} depends on on-demand packages: "
                f"{','.join(leaked_dependencies)}"
            )
    for relative, snippets in REQUIRED_SNIPPETS.items():
        raw = (ROOT / relative).read_text(encoding="utf-8")
        for snippet in snippets:
            if snippet not in raw:
                errors.append(f"{relative}: missing contract snippet: {snippet}")
    for relative in DIRECT_WORKSPACE_BUILD_FORBIDDEN:
        raw = (ROOT / relative).read_text(encoding="utf-8")
        if "cargo build --workspace" in raw:
            errors.append(f"{relative}: direct workspace build bypasses registry exclusions")
    for relative in PROACTIVE_BUILD_ENTRYPOINTS:
        raw = (ROOT / relative).read_text(encoding="utf-8")
        for spec in specs:
            hardcoded_build = re.compile(
                rf"(?:^|\s)(?:-p|--package)(?:=|\s+)[\"']?{re.escape(spec.package)}(?:[\"'\s\\]|$)"
            )
            if hardcoded_build.search(raw):
                errors.append(
                    f"{relative}: proactively hardcodes on-demand package {spec.package}"
                )

    if errors:
        for error in errors:
            print(f"ON_DEMAND_SKILL_BUILD_CHECK error={error}")
        return 1
    print(
        "ON_DEMAND_SKILL_BUILD_CHECK "
        f"ok on_demand={len(specs)} runners={len(all_specs)} "
        "platform_metadata=required source_root=optional_skills"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
