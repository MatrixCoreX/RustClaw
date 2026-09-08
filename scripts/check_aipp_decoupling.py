#!/usr/bin/env python3
"""Ratchet AiAPP package declarations away from host skill-specific coupling."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SKILL_ROOTS = ("crates/skills", "optional_skills", "external_skills")
HOST_FILES = (
    "crates/clawd/src/http/ui_routes/aipp.rs",
    "UI/src/components/AippPage.tsx",
)
SUPPORTED_HOST_PAIRS = {
    ("collection_feed_v1", "media_collection_v1"),
    ("task_activity_v1", "skill_task_activity_v1"),
}
SUPPORTED_SANDBOX_CONTRACTS = {
    "capability_bridge_v1",
}


def discover_manifests(root: Path) -> list[tuple[Path, dict]]:
    manifests: list[tuple[Path, dict]] = []
    for relative_root in SKILL_ROOTS:
        source_root = root / relative_root
        if not source_root.is_dir():
            continue
        for path in sorted(source_root.glob("*/skill.toml")):
            try:
                document = tomllib.loads(path.read_text(encoding="utf-8"))
            except (OSError, UnicodeError, tomllib.TOMLDecodeError) as error:
                raise RuntimeError(f"manifest_read_failed path={path}: {error}") from error
            if isinstance(document.get("aipp"), dict):
                manifests.append((path, document))
    return manifests


def safe_skill_name(value: object) -> str | None:
    if not isinstance(value, str) or not re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,127}", value):
        return None
    return value


def inspect(root: Path) -> tuple[list[str], int, int]:
    findings: list[str] = []
    manifests = discover_manifests(root)
    skill_names: set[str] = set()
    sandboxed = 0
    for path, document in manifests:
        package = document.get("package", {})
        aipp = document["aipp"]
        skill_name = safe_skill_name(package.get("name"))
        if skill_name is None:
            findings.append(f"aipp_package_name_invalid path={path.relative_to(root)}")
            continue
        skill_names.add(skill_name)
        renderer = aipp.get("renderer")
        data_contract = aipp.get("data_contract")
        if renderer == "sandbox_bundle_v1":
            sandboxed += 1
            if data_contract not in SUPPORTED_SANDBOX_CONTRACTS:
                findings.append(
                    f"aipp_sandbox_contract_unreviewed skill={skill_name} contract={data_contract!r}"
                )
            asset_root = aipp.get("asset_root")
            entrypoint = aipp.get("entrypoint")
            if asset_root != "aipp" or not isinstance(entrypoint, str):
                findings.append(f"aipp_sandbox_paths_invalid skill={skill_name}")
            else:
                package_root = path.parent.resolve()
                assets = (package_root / asset_root).resolve()
                entry = (package_root / entrypoint).resolve()
                if (
                    not assets.is_dir()
                    or not entry.is_file()
                    or assets not in entry.parents
                    or package_root not in assets.parents
                ):
                    findings.append(f"aipp_sandbox_assets_missing skill={skill_name}")
        elif (renderer, data_contract) not in SUPPORTED_HOST_PAIRS:
            findings.append(
                f"aipp_host_contract_unreviewed skill={skill_name} renderer={renderer!r} contract={data_contract!r}"
            )
        if data_contract == "skill_task_activity_v1" and aipp.get("task_channel_scope") not in {
            "all",
            "communication",
        }:
            findings.append(f"aipp_task_channel_scope_missing skill={skill_name}")

    for relative in HOST_FILES:
        path = root / relative
        try:
            source = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            findings.append(f"aipp_host_source_unreadable path={relative} error={error}")
            continue
        for skill_name in sorted(skill_names):
            if re.search(rf"(?<![a-z0-9_-]){re.escape(skill_name)}(?![a-z0-9_-])", source):
                findings.append(f"aipp_host_skill_branch path={relative} skill={skill_name}")
        if relative.startswith("UI/") and re.search(
            r"(?:crates/skills|optional_skills|external_skills)/", source
        ):
            findings.append(f"aipp_main_ui_imports_skill_source path={relative}")
    return findings, len(manifests), sandboxed


def self_test() -> int:
    assert safe_skill_name("sample_app") == "sample_app"
    assert safe_skill_name("../escape") is None
    source = 'if (selectedSkill === "sample_app") return customView;'
    assert re.search(r"(?<![a-z0-9_-])sample_app(?![a-z0-9_-])", source)
    assert ("task_activity_v1", "skill_task_activity_v1") in SUPPORTED_HOST_PAIRS
    assert "capability_bridge_v1" in SUPPORTED_SANDBOX_CONTRACTS
    print("AIPP_DECOUPLING_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    try:
        findings, apps, sandboxed = inspect(ROOT)
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 1
    for finding in findings:
        print(finding)
    print(f"AIPP_DECOUPLING_CHECK apps={apps} sandboxed={sandboxed} findings={len(findings)}")
    return 1 if findings else 0


if __name__ == "__main__":
    raise SystemExit(main())
