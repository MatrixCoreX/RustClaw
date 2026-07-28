#!/usr/bin/env python3
"""Generate Cargo skill manifests and registry projections from one inventory."""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOTS = (ROOT / "crates/skills", ROOT / "optional_skills")
REGISTRIES = (ROOT / "configs/skills_registry.toml", ROOT / "docker/config/skills_registry.toml")
MARKER = "# AUTO-GENERATED: sync_skill_manifests.py"
ENV_LITERAL = re.compile(r'(?:std::)?env::var(?:_os)?\(\s*"([A-Z_][A-Z0-9_]*)"')
BASE_RUNTIME_ENV = {
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TMPDIR",
    "TMP",
    "TEMP",
    "TZ",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "WORKSPACE_ROOT",
    "SKILL_TIMEOUT_SECONDS",
}


@dataclass(frozen=True)
class CargoSkill:
    name: str
    source_dir: Path
    package: str
    binary: str

    @property
    def relative_manifest(self) -> str:
        return self.source_dir.relative_to(ROOT).joinpath("skill.toml").as_posix()


def cargo_skills() -> dict[str, CargoSkill]:
    skills: dict[str, CargoSkill] = {}
    for source_root in SOURCE_ROOTS:
        if not source_root.is_dir():
            continue
        for manifest in sorted(source_root.glob("*/Cargo.toml")):
            payload = tomllib.loads(manifest.read_text(encoding="utf-8"))
            package = payload.get("package", {}).get("name")
            if not isinstance(package, str) or not package:
                raise ValueError(f"missing package.name: {manifest}")
            bins = [
                item.get("name")
                for item in payload.get("bin", [])
                if isinstance(item, dict) and isinstance(item.get("name"), str)
            ]
            binary = bins[0] if bins else package
            name = manifest.parent.name
            skills[name] = CargoSkill(name, manifest.parent, package, binary)
    return skills


def registry_entries(path: Path) -> list[dict[str, object]]:
    return tomllib.loads(path.read_text(encoding="utf-8")).get("skills", [])


def runtime_environment_allowlist(skill: CargoSkill, entry: dict[str, object]) -> list[str]:
    names = set(BASE_RUNTIME_ENV)
    for source in skill.source_dir.rglob("*"):
        if not source.is_file() or source.suffix not in {".rs", ".py", ".js", ".ts"}:
            continue
        try:
            raw = source.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        names.update(ENV_LITERAL.findall(raw))
    capabilities = {str(value) for value in entry.get("capabilities", [])}
    for capability in capabilities:
        if capability.startswith("secrets."):
            names.add(capability.removeprefix("secrets.").upper().replace(".", "_"))
    if "llm" in capabilities:
        names.update(
            {
                "CLAWD_BASE_URL",
                "RUSTCLAW_INTERNAL_LLM_URL",
                "RUSTCLAW_INTERNAL_LLM_TOKEN",
                "RUSTCLAW_SELECTED_LLM_VENDOR",
                "RUSTCLAW_SELECTED_LLM_PROVIDER_TYPE",
                "OPENAI_BASE_URL",
                "OPENAI_MODEL",
                "OPENAI_API_KEY",
                "GOOGLE_API_KEY",
                "ANTHROPIC_API_KEY",
                "GROK_API_KEY",
                "DEEPSEEK_API_KEY",
                "QWEN_API_KEY",
                "MINIMAX_API_KEY",
                "MIMO_API_KEY",
            }
        )
    return sorted(names)


def sandbox_profile(entry: dict[str, object]) -> str:
    capabilities = {str(value) for value in entry.get("capabilities", [])}
    mappings = [
        value for value in entry.get("planner_capabilities", []) if isinstance(value, dict)
    ]
    writes = "fs.write" in capabilities or any(
        mapping.get("filesystem_write") is True or mapping.get("effect") == "mutate"
        for mapping in mappings
    )
    return "workspace_write" if writes else "required"


def render_manifest(skill: CargoSkill, entry: dict[str, object], version: str) -> str:
    supported_os = entry.get("supported_os") or ["linux", "macos"]
    description = str(entry.get("description") or f"RustClaw {skill.name} skill")
    timeout = int(entry.get("timeout_seconds") or 30)
    capabilities = {str(value) for value in entry.get("capabilities", [])}
    runtime_network = bool({"net", "llm"} & capabilities)
    storage = entry.get("storage") if isinstance(entry.get("storage"), dict) else None
    storage_kind = str(storage.get("kind")) if storage else "none"
    storage_version = int(storage.get("schema_version") or 1) if storage else 1
    migration_owner = str(storage.get("migration_owner")) if storage else skill.name
    config_files = entry.get("config_files") or []
    environment_allowlist = runtime_environment_allowlist(skill, entry)
    sandbox = sandbox_profile(entry)
    return f"""{MARKER}
schema_version = 1

[package]
name = {json.dumps(skill.name)}
version = {json.dumps(version)}
description = {json.dumps(description)}
protocol = "rustclaw-jsonl-v1"
supported_os = {json.dumps(supported_os)}
supported_arch = ["x86_64", "aarch64"]
license = "MIT"
source = {json.dumps(skill.source_dir.relative_to(ROOT).as_posix())}

[registry]
name = {json.dumps(skill.name)}
capability_policy_source = "registry"

[build]
adapter = "cargo"
source_root = "."
package = {json.dumps(skill.package)}
binary = {json.dumps(skill.binary)}
lockfile = "Cargo.lock"
network = "deny"
lifecycle_scripts = false

[run]
launcher = "native"
entrypoint = {json.dumps('runtime/bin/' + skill.binary)}
working_directory = "."
timeout_seconds = {timeout}
environment_allowlist = {json.dumps(environment_allowlist)}
smoke_args = {{}}

[security]
capability_policy_source = "registry"
sandbox = {json.dumps(sandbox)}
runtime_network = {str(runtime_network).lower()}
inherit_credentials = false

[storage]
kind = {json.dumps(storage_kind)}
schema_version = {storage_version}
migration_owner = {json.dumps(migration_owner)}

[lifecycle]
config_files = {json.dumps(config_files)}
preserve_data_on_uninstall = true
update_strategy = "atomic_replace"
"""


def sync_manifests(skills: dict[str, CargoSkill], entries: list[dict[str, object]], check: bool) -> int:
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    changed = 0
    for entry in entries:
        if entry.get("kind") != "runner":
            continue
        name = str(entry.get("name") or "")
        skill = skills.get(name)
        if skill is None:
            raise ValueError(f"runner skill has no Cargo source: {name}")
        path = skill.source_dir / "skill.toml"
        expected = render_manifest(skill, entry, str(version))
        current = path.read_text(encoding="utf-8") if path.is_file() else None
        if current is not None and MARKER not in current:
            raise ValueError(f"refusing to overwrite unmanaged manifest: {path}")
        if current != expected:
            changed += 1
            if not check:
                path.write_text(expected, encoding="utf-8")
    return changed


def update_registry(path: Path, skills: dict[str, CargoSkill], check: bool) -> int:
    raw = path.read_text(encoding="utf-8")
    pieces = re.split(r"(?=^\[\[skills\]\]\s*$)", raw, flags=re.MULTILINE)
    changed = 0
    rendered: list[str] = []
    for piece in pieces:
        if not piece.startswith("[[skills]]"):
            rendered.append(piece)
            continue
        parsed = tomllib.loads(piece).get("skills", [{}])[0]
        name = str(parsed.get("name") or "")
        if parsed.get("kind") != "runner" or name not in skills:
            rendered.append(piece)
            continue
        lines = [
            line
            for line in piece.splitlines(keepends=True)
            if not line.startswith("package_manifest = ")
        ]
        insert_after = max(
            (
                index
                for index, line in enumerate(lines)
                if line.startswith(("install_mode = ", "group = ", "kind = "))
            ),
            default=0,
        )
        projection = [
            f'package_manifest = "{skills[name].relative_manifest}"\n',
        ]
        lines[insert_after + 1 : insert_after + 1] = projection
        updated = "".join(lines)
        changed += int(updated != piece)
        rendered.append(updated)
    output = "".join(rendered)
    if output != raw and not check:
        path.write_text(output, encoding="utf-8")
    return changed


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        skills = cargo_skills()
        main_entries = registry_entries(REGISTRIES[0])
        manifest_changes = sync_manifests(skills, main_entries, args.check)
        registry_changes = sum(update_registry(path, skills, args.check) for path in REGISTRIES)
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        print(f"SKILL_MANIFEST_SYNC failed: {error}", file=sys.stderr)
        return 1
    if args.check and (manifest_changes or registry_changes):
        print(
            f"SKILL_MANIFEST_SYNC drift manifests={manifest_changes} registries={registry_changes}",
            file=sys.stderr,
        )
        return 1
    print(
        f"SKILL_MANIFEST_SYNC ok skills={len(skills)} manifest_changes={manifest_changes} registry_changes={registry_changes}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
