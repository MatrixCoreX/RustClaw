#!/usr/bin/env python3
"""Resolve registry-owned runner packages for precise platform builds.

The historical CLI (no ``--scope`` argument) still lists on-demand Skill
Store packages. Build/install entry points can additionally request the
proactive set for one target, the packages excluded from that target's normal
build, or one exact skill package.
"""

from __future__ import annotations

import argparse
import platform
import json
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path


OPTIONAL_SKILLS_ROOT = Path("optional_skills")


@dataclass(frozen=True)
class OnDemandSkillSpec:
    skill_name: str
    package: str
    runner: str
    source_dir: Path


@dataclass(frozen=True)
class SkillBuildSpec:
    skill_name: str
    adapter: str
    package: str
    runner: str
    manifest_path: Path
    install_mode: str
    supported_os: tuple[str, ...]
    supported_arch: tuple[str, ...]


SAFE_CARGO_NAME = re.compile(r"^[A-Za-z0-9_.-]+$")


def runner_binary_name(raw_name: str) -> str:
    runner = raw_name.strip().replace("_", "-")
    if runner and not runner.endswith("-skill"):
        runner += "-skill"
    return runner


def normalize_os_token(raw: str) -> str:
    token = raw.strip().lower().replace("_", "-").replace(" ", "-")
    aliases = {
        "*": "any",
        "all": "any",
        "unix": "any",
        "mac": "macos",
        "mac-os": "macos",
        "osx": "macos",
        "darwin": "macos",
        "gnu-linux": "linux",
        "debian": "linux",
        "ubuntu": "linux",
        "raspbian": "linux",
        "raspberry-pi-os": "linux",
        "raspi": "linux",
        "win32": "windows",
        "win64": "windows",
    }
    return aliases.get(token, token)


def platform_for_target(raw_target: str) -> str:
    target = raw_target.strip().lower()
    if target in {"", "host"}:
        return normalize_os_token(platform.system())
    direct = normalize_os_token(target)
    if direct in {"linux", "macos", "windows"}:
        return direct
    if "darwin" in target or "apple" in target:
        return "macos"
    if "windows" in target:
        return "windows"
    if "linux" in target:
        return "linux"
    raise ValueError(f"cannot determine platform from target: {raw_target}")


def arch_for_target(raw_target: str) -> str:
    target = raw_target.strip().lower()
    if target in {"", "host", "linux", "macos", "windows"}:
        target = platform.machine().strip().lower()
    aliases = {
        "amd64": "x86_64",
        "x64": "x86_64",
        "arm64": "aarch64",
        "armv7l": "armv7",
        "armhf": "armv7",
    }
    direct = aliases.get(target, target)
    if direct in {"x86_64", "aarch64", "armv7"}:
        return direct
    if target.startswith("x86_64-"):
        return "x86_64"
    if target.startswith("aarch64-"):
        return "aarch64"
    if target.startswith("armv7-"):
        return "armv7"
    raise ValueError(f"cannot determine architecture from target: {raw_target}")


def supports_platform(spec: SkillBuildSpec, platform_name: str, arch_name: str) -> bool:
    supported = {normalize_os_token(value) for value in spec.supported_os if value.strip()}
    supported_arch = {value.strip().lower() for value in spec.supported_arch if value.strip()}
    os_matches = not supported or "any" in supported or platform_name in supported
    arch_matches = not supported_arch or "any" in supported_arch or arch_name in supported_arch
    return os_matches and arch_matches


def resolve_manifest_path(registry_path: Path, raw: str) -> Path:
    relative = Path(raw)
    if relative.is_absolute() or ".." in relative.parts:
        raise ValueError(f"unsafe package_manifest path: {raw}")
    candidates = (Path.cwd() / relative, registry_path.parent / relative)
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    raise ValueError(f"package_manifest not found: {raw}")


def runner_specs(registry_path: Path) -> list[SkillBuildSpec]:
    registry = tomllib.loads(registry_path.read_text(encoding="utf-8"))
    specs: set[SkillBuildSpec] = set()
    for skill in registry.get("skills", []):
        install_mode = str(skill.get("install_mode") or "proactive").strip()
        if str(skill.get("kind") or "").strip() != "runner":
            continue
        skill_name = str(skill.get("name") or "").strip()
        manifest_ref = str(skill.get("package_manifest") or "").strip()
        if not skill_name or not manifest_ref:
            raise ValueError("runner skill must declare name and package_manifest")
        manifest_path = resolve_manifest_path(registry_path, manifest_ref)
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        package_meta = manifest.get("package", {})
        build = manifest.get("build", {})
        run = manifest.get("run", {})
        registry_ref = manifest.get("registry", {})
        if package_meta.get("name") != skill_name or registry_ref.get("name") != skill_name:
            raise ValueError(f"manifest identity mismatch for runner skill: {skill_name}")
        adapter = str(build.get("adapter") or "").strip()
        package = str(build.get("package") or "").strip()
        binary = str(build.get("binary") or "").strip()
        if adapter == "cargo" and (not package or not binary):
            raise ValueError(f"cargo manifest must declare package and binary: {skill_name}")
        runner = binary if binary else Path(str(run.get("entrypoint") or "")).name
        supported_os = tuple(
            sorted(
                {
                    str(value).strip()
                    for value in package_meta.get("supported_os", [])
                    if str(value).strip()
                }
            )
        )
        supported_arch = tuple(
            sorted(
                {
                    str(value).strip().lower()
                    for value in package_meta.get("supported_arch", [])
                    if str(value).strip()
                }
            )
        )
        if not adapter or not runner:
            raise ValueError(f"runner manifest is incomplete: {skill_name}")
        if (package and not SAFE_CARGO_NAME.fullmatch(package)) or not SAFE_CARGO_NAME.fullmatch(runner):
            raise ValueError(
                f"runner skill has unsafe package or runner: {skill_name}"
            )
        specs.add(
            SkillBuildSpec(
                skill_name=skill_name,
                adapter=adapter,
                package=package,
                runner=runner,
                manifest_path=manifest_path,
                install_mode=install_mode,
                supported_os=supported_os,
                supported_arch=supported_arch,
            )
        )
    return sorted(specs, key=lambda item: (item.skill_name, item.adapter, item.runner))


def on_demand_specs(registry_path: Path) -> list[OnDemandSkillSpec]:
    """Return only Cargo packages excluded from proactive workspace builds.

    Non-Cargo on-demand skills are still returned by ``runner_specs`` and the
    CLI's manifest/record formats.  They do not have a Cargo package or source
    ``Cargo.toml`` to police here; their selected adapter owns installation.
    """
    specs = {
        OnDemandSkillSpec(
            skill_name=spec.skill_name,
            package=spec.package,
            runner=spec.runner,
            source_dir=spec.manifest_path.parent,
        )
        for spec in runner_specs(registry_path)
        if spec.install_mode == "on_demand" and spec.adapter == "cargo"
    }
    return sorted(specs, key=lambda item: (item.package, item.runner, item.skill_name))


def on_demand_pairs(registry_path: Path) -> list[tuple[str, str]]:
    return [(spec.package, spec.runner) for spec in on_demand_specs(registry_path)]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--registry",
        type=Path,
        default=Path("configs/skills_registry.toml"),
    )
    parser.add_argument(
        "--format",
        choices=("packages", "runners", "pairs", "specs", "manifests", "records", "cargo-excludes"),
        default="packages",
    )
    parser.add_argument(
        "--scope",
        choices=(
            "on-demand",
            "proactive",
            "unsupported-proactive",
            "build-excludes",
            "all-runners",
            "selected",
        ),
        default="on-demand",
    )
    parser.add_argument(
        "--target",
        default="host",
        help="host, an OS token, or a Rust target triple",
    )
    parser.add_argument("--skill", default="")
    return parser.parse_args()


def select_specs(args: argparse.Namespace) -> list[SkillBuildSpec]:
    specs = runner_specs(args.registry)
    platform_name = platform_for_target(args.target)
    arch_name = arch_for_target(args.target)
    if args.scope == "on-demand":
        return [spec for spec in specs if spec.install_mode == "on_demand"]
    if args.scope == "proactive":
        return [
            spec
            for spec in specs
            if spec.install_mode != "on_demand" and supports_platform(spec, platform_name, arch_name)
        ]
    if args.scope == "unsupported-proactive":
        return [
            spec
            for spec in specs
            if spec.install_mode != "on_demand" and not supports_platform(spec, platform_name, arch_name)
        ]
    if args.scope == "build-excludes":
        return [
            spec
            for spec in specs
            if spec.adapter == "cargo"
            and (spec.install_mode == "on_demand" or not supports_platform(spec, platform_name, arch_name))
        ]
    if args.scope == "all-runners":
        return specs

    requested = args.skill.strip()
    if not requested:
        raise ValueError("--scope selected requires --skill")
    registry = tomllib.loads(args.registry.read_text(encoding="utf-8"))
    canonical = requested
    for skill in registry.get("skills", []):
        name = str(skill.get("name") or "").strip()
        aliases = {str(value).strip() for value in skill.get("aliases", [])}
        if requested == name or requested in aliases:
            canonical = name
            break
    selected = [spec for spec in specs if spec.skill_name == canonical]
    if not selected:
        raise ValueError(f"unknown runner skill: {requested}")
    if not supports_platform(selected[0], platform_name, arch_name):
        supported = f"os={','.join(selected[0].supported_os) or 'any'} arch={','.join(selected[0].supported_arch) or 'any'}"
        raise ValueError(
            f"skill {canonical} does not support platform {platform_name}/{arch_name}; supported={supported}"
        )
    return selected


def main() -> int:
    args = parse_args()
    try:
        specs = select_specs(args)
    except (OSError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"skill store package discovery failed: {error}", file=sys.stderr)
        return 1

    seen: set[str] = set()
    for spec in specs:
        if args.format in {"packages", "cargo-excludes", "pairs"} and not spec.package:
            continue
        dedup_key = spec.package if args.format in {"packages", "cargo-excludes"} else (
            spec.runner if args.format == "runners" else spec.skill_name
        )
        if dedup_key in seen:
            continue
        seen.add(dedup_key)
        if args.format == "packages":
            print(spec.package)
        elif args.format == "runners":
            print(spec.runner)
        elif args.format == "pairs":
            print(f"{spec.package}\t{spec.runner}")
        elif args.format == "specs":
            print(
                f"{spec.skill_name}\t{spec.package or '-'}\t{spec.runner}\t"
                f"{spec.install_mode}\t{','.join(spec.supported_os)}\t{spec.adapter}"
            )
        elif args.format == "manifests":
            print(spec.manifest_path)
        elif args.format == "records":
            print(json.dumps({
                "skill_name": spec.skill_name,
                "adapter": spec.adapter,
                "package": spec.package or None,
                "runner": spec.runner,
                "manifest_path": str(spec.manifest_path),
                "install_mode": spec.install_mode,
                "supported_os": spec.supported_os,
                "supported_arch": spec.supported_arch,
            }, separators=(",", ":"), sort_keys=True))
        elif args.format == "cargo-excludes":
            print(f"--exclude={spec.package}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
