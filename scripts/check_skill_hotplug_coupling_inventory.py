#!/usr/bin/env python3
"""Inventory and ratchet skill hot-plug coupling in production code."""

from __future__ import annotations

import argparse
import json
import re
import tempfile
import tomllib
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
MAIN_REGISTRY = Path("configs/skills_registry.toml")
DOCKER_REGISTRY = Path("docker/config/skills_registry.toml")
BASELINE = Path("scripts/baselines/skill_hotplug_coupling_inventory.json")
SCAN_ROOTS = (
    Path("crates/clawd/src"),
    Path("crates/claw-core/src/config"),
    Path("crates/skill-sdk/src"),
    Path("crates/skill-runner/src"),
    Path("crates/skills/extension_manager/src"),
)
MIGRATION_PATH_TOKENS = ("legacy", "historical", "compat")
CONTRACT_AUTHORITY_PATHS = {
    "crates/skill-sdk/src/admission.rs",
}
# Channel-delivery receipts are a separate machine contract, not skill package
# install receipts. Keep this explicit so the generic digest token cannot be
# mistaken for a skill hot-plug coupling surface.
SURFACE_EXCLUSIONS = {
    "receipt_resolution_or_pin": {
        "crates/clawd/src/repo/channel_delivery_receipt.rs",
    },
    # The NNI internal gateway authenticates a scoped core-domain token. This
    # is an authorization boundary, not planner-side skill selection.
    "semantic_special_case": {
        "crates/clawd/src/http/ui_routes/nni_skill_gateway.rs",
    },
}
DOMAIN_ADAPTER_PATHS = {
    "crates/clawd/src/http/ui_routes/nni_internal_llm.rs",
    "crates/clawd/src/http/ui_routes/nni_remote_join.rs",
    "crates/clawd/src/http/ui_routes/nni_skill_gateway.rs",
    "crates/clawd/src/repo/crypto_storage.rs",
    "crates/clawd/src/skill_storage/data_owners.rs",
    "crates/clawd/src/skill_storage/migration.rs",
    "crates/clawd/src/skill_storage/ownership.rs",
    "crates/clawd/src/skill_storage/schema.rs",
}

SURFACE_PATTERNS = {
    "registration_write_or_reload": re.compile(
        r"skills_registry\.toml|skill_switches|generated/skills|"
        r"reload_skill_views|register_external_skill"
    ),
    "receipt_resolution_or_pin": re.compile(
        r"current_pointer|current\.json|SkillRuntimeResolver::new|"
        r"receipt_digest|expected_receipt|expected_version"
    ),
    "activation_or_removal": re.compile(
        r"remove_skill_store_package|store\.activate\(|"
        r"skill_(?:generation|tombstone|lease|refcount)|"
        r"(?:tombstone|lease|refcount)_skill"
    ),
    "duplicate_skill_list": re.compile(
        r"default_(?:core|optional|full)_skills|"
        r"skill_store_optional_skill_names|is_builtin_skill_name"
    ),
    "skill_timeout_fallback": re.compile(
        r"unwrap_or_else\(\|\|\s*match\s+skill_name|"
        r"skill_timeout_seconds\.max\("
    ),
    "semantic_special_case": re.compile(
        r"(?:skill_name|skill|observed\.skill|name)\s*(?:==|!=)|"
        r"match\s+(?:skill_name|skill)|starts_with\(\"(?:image|audio|video|crypto|kb)"
    ),
}


def read_skills(root: Path, relative: Path) -> list[dict[str, Any]]:
    parsed = tomllib.loads((root / relative).read_text(encoding="utf-8"))
    skills = parsed.get("skills")
    if not isinstance(skills, list):
        raise ValueError(f"registry_skills_missing:{relative}")
    return skills


def registry_inventory(root: Path) -> tuple[dict[str, str], list[str]]:
    main = read_skills(root, MAIN_REGISTRY)
    docker = read_skills(root, DOCKER_REGISTRY)
    main_map = {str(row.get("name") or ""): str(row.get("kind") or "") for row in main}
    docker_map = {str(row.get("name") or ""): str(row.get("kind") or "") for row in docker}
    findings: list[str] = []
    if main_map != docker_map:
        for name in sorted(set(main_map) | set(docker_map)):
            if main_map.get(name) != docker_map.get(name):
                findings.append(
                    f"registry_parity_mismatch:{name}:"
                    f"main={main_map.get(name)}:docker={docker_map.get(name)}"
                )
    return main_map, findings


def rust_files(root: Path) -> list[Path]:
    paths: set[Path] = set()
    for relative in SCAN_ROOTS:
        scan_root = root / relative
        if scan_root.is_dir():
            paths.update(path for path in scan_root.rglob("*.rs") if path.is_file())
    return sorted(paths)


def is_test_path(relative: Path) -> bool:
    lowered = relative.as_posix().lower()
    return (
        relative.name.endswith("_tests.rs")
        or relative.name == "tests.rs"
        or any(
            part in {"tests", "fixtures"} or part.endswith("_tests")
            for part in relative.parts
        )
    )


def classify_skill_literal(relative: Path, kind: str) -> str:
    lowered = relative.as_posix().lower()
    if is_test_path(relative):
        return "test_fixture"
    if any(token in lowered for token in MIGRATION_PATH_TOKENS):
        return "migration_reader"
    if relative.as_posix() in DOMAIN_ADAPTER_PATHS:
        return "domain_adapter"
    if kind == "builtin" and (
        "/skills/builtin" in lowered or relative.as_posix().endswith("/skills.rs")
    ):
        return "host_tool_dispatch"
    return "business_skill_coupling"


def quoted_skill_pattern(names: set[str]) -> re.Pattern[str]:
    alternatives = "|".join(re.escape(name) for name in sorted(names, key=len, reverse=True))
    return re.compile(rf'(?P<quote>\")(?P<name>{alternatives})(?P=quote)')


def scan(root: Path) -> tuple[dict[str, Any], list[str]]:
    skills, findings = registry_inventory(root)
    names = {name for name in skills if name}
    pattern = quoted_skill_pattern(names)
    literals: list[dict[str, Any]] = []
    surfaces: list[dict[str, Any]] = []
    for path in rust_files(root):
        relative = path.relative_to(root)
        for line_number, raw_line in enumerate(
            path.read_text(encoding="utf-8", errors="replace").splitlines(), 1
        ):
            code = raw_line.split("//", 1)[0]
            for match in pattern.finditer(code):
                name = match.group("name")
                literals.append(
                    {
                        "category": classify_skill_literal(relative, skills[name]),
                        "path": relative.as_posix(),
                        "skill": name,
                        "line": line_number,
                    }
                )
            if is_test_path(relative):
                continue
            for surface, surface_pattern in SURFACE_PATTERNS.items():
                if relative.as_posix() in SURFACE_EXCLUSIONS.get(surface, set()):
                    continue
                if surface_pattern.search(code):
                    surfaces.append(
                        {
                            "surface": (
                                "contract_authority"
                                if relative.as_posix() in CONTRACT_AUTHORITY_PATHS
                                else surface
                            ),
                            "path": relative.as_posix(),
                            "line": line_number,
                        }
                    )
    return {
        "schema_version": 1,
        "registries": {
            "main": MAIN_REGISTRY.as_posix(),
            "docker": DOCKER_REGISTRY.as_posix(),
            "skills": len(skills),
        },
        "skill_name_literals": literals,
        "coupling_surfaces": surfaces,
    }, findings


def snapshot(inventory: dict[str, Any]) -> dict[str, Any]:
    literal_counts = Counter(
        f"{row['category']}|{row['path']}|{row['skill']}"
        for row in inventory["skill_name_literals"]
        if row["category"] != "test_fixture"
    )
    surface_counts = Counter(
        f"{row['surface']}|{row['path']}"
        for row in inventory["coupling_surfaces"]
        if row["surface"] != "contract_authority"
    )
    return {
        "schema_version": 1,
        "skill_name_literal_max": dict(sorted(literal_counts.items())),
        "coupling_surface_max": dict(sorted(surface_counts.items())),
    }


def compare_counts(label: str, current: Counter[str], expected: dict[str, int]) -> list[str]:
    findings: list[str] = []
    for key in sorted(set(current) | set(expected)):
        actual = current.get(key, 0)
        allowed = int(expected.get(key, 0))
        if key not in expected:
            findings.append(f"{label}_new:{key}:count={actual}")
        elif actual > allowed:
            findings.append(f"{label}_grew:{key}:{actual}>{allowed}")
        elif actual < allowed:
            findings.append(f"{label}_baseline_must_shrink:{key}:{actual}<{allowed}")
    return findings


def check_baseline(inventory: dict[str, Any], baseline: dict[str, Any]) -> list[str]:
    if baseline.get("schema_version") != 1:
        return ["baseline_schema_invalid"]
    current = snapshot(inventory)
    findings = compare_counts(
        "skill_literal",
        Counter(current["skill_name_literal_max"]),
        baseline.get("skill_name_literal_max") or {},
    )
    findings.extend(
        compare_counts(
            "coupling_surface",
            Counter(current["coupling_surface_max"]),
            baseline.get("coupling_surface_max") or {},
        )
    )
    return findings


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="skill-hotplug-inventory-") as raw:
        root = Path(raw)
        registry = '''[[skills]]\nname="host"\nkind="builtin"\n\n[[skills]]\nname="crypto"\nkind="runner"\n'''
        for relative in (MAIN_REGISTRY, DOCKER_REGISTRY):
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(registry, encoding="utf-8")
        source = root / "crates/clawd/src/skills.rs"
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_text(
            'fn f(name: &str) { if name == "crypto" { } let _ = "host"; }\n',
            encoding="utf-8",
        )
        inventory, findings = scan(root)
        assert not findings
        delivery_receipt = root / "crates/clawd/src/repo/channel_delivery_receipt.rs"
        delivery_receipt.parent.mkdir(parents=True, exist_ok=True)
        delivery_receipt.write_text("fn delivery(receipt_digest: &str) {}\n", encoding="utf-8")
        inventory, findings = scan(root)
        assert not findings
        assert not any(
            item["path"] == delivery_receipt.relative_to(root).as_posix()
            for item in inventory["coupling_surfaces"]
        )
        baseline = snapshot(inventory)
        assert not check_baseline(inventory, baseline)
        source.write_text(source.read_text() + 'const EXTRA: &str = "crypto";\n')
        changed, findings = scan(root)
        assert not findings
        assert any("skill_literal_grew" in item for item in check_baseline(changed, baseline))
        (root / DOCKER_REGISTRY).write_text(
            '[[skills]]\nname="host"\nkind="runner"\n', encoding="utf-8"
        )
        _, findings = scan(root)
        assert findings and findings[0].startswith("registry_parity_mismatch")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--print-baseline", action="store_true")
    parser.add_argument("--update-baseline", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        self_test()
        print("SKILL_HOTPLUG_COUPLING_INVENTORY_SELF_TEST ok")
        return 0
    inventory, findings = scan(ROOT)
    if args.print_baseline:
        print(json.dumps(snapshot(inventory), indent=2, sort_keys=True))
        return 0
    if args.update_baseline:
        if findings:
            for finding in findings:
                print(f"- {finding}")
            return 1
        baseline_path = ROOT / BASELINE
        baseline_path.write_text(
            json.dumps(snapshot(inventory), indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(f"SKILL_HOTPLUG_COUPLING_BASELINE_UPDATED path={BASELINE}")
        return 0
    baseline = json.loads((ROOT / BASELINE).read_text(encoding="utf-8"))
    findings.extend(check_baseline(inventory, baseline))
    if args.json:
        print(json.dumps({"inventory": inventory, "findings": findings}, indent=2))
    categories = Counter(row["category"] for row in inventory["skill_name_literals"])
    surfaces = Counter(row["surface"] for row in inventory["coupling_surfaces"])
    print(
        "SKILL_HOTPLUG_COUPLING_INVENTORY_CHECK "
        f"skills={inventory['registries']['skills']} "
        f"literals={sum(categories.values())} categories={dict(sorted(categories.items()))} "
        f"surfaces={dict(sorted(surfaces.items()))} findings={len(findings)}"
    )
    for finding in findings:
        print(f"- {finding}")
    return 1 if findings else 0


if __name__ == "__main__":
    raise SystemExit(main())
