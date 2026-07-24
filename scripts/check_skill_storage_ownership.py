#!/usr/bin/env python3
"""Enforce skill-owned storage boundaries for persisted skills."""
from __future__ import annotations

import argparse
import tempfile
import tomllib
from pathlib import Path


PERSISTED_SKILLS = {
    "crypto": {"kind": "sqlite", "schema_version": 1, "migration_owner": "crypto"},
    "kb": {"kind": "sqlite", "schema_version": 1, "migration_owner": "kb"},
}
REGISTRY_PATHS = (
    Path("configs/skills_registry.toml"),
    Path("docker/config/skills_registry.toml"),
)
CONFIG_PATHS = (
    Path("configs/config.toml"),
    Path("docker/config/config.toml"),
)
ALLOWED_EXCHANGE_TABLE_OWNERS = {
    Path("crates/clawd/src/repo/crypto_storage.rs"),
    Path("crates/clawd/src/skill_storage/mod.rs"),
    Path("crates/clawd/src/skill_storage/migration.rs"),
    Path("crates/clawd/src/skill_storage/schema.rs"),
    Path("scripts/import-crypto-credentials.sh"),
}
SKILL_SOURCE_ROOTS = (
    Path("crates/skills"),
    Path("optional_skills"),
    Path("external_skills"),
)


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def is_test_path(path: Path) -> bool:
    return "tests" in path.parts or path.name.endswith("_tests.rs")


def read_text(root: Path, relative: Path, findings: list[str]) -> str:
    path = root / relative
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        findings.append(f"missing_required_file:{relative.as_posix()}")
        return ""


def load_toml(root: Path, relative: Path, findings: list[str]) -> dict:
    text = read_text(root, relative, findings)
    if not text:
        return {}
    try:
        return tomllib.loads(text)
    except tomllib.TOMLDecodeError as exc:
        findings.append(f"invalid_toml:{relative.as_posix()}:{exc}")
        return {}


def check_configs(root: Path, findings: list[str]) -> None:
    for relative in CONFIG_PATHS:
        config = load_toml(root, relative, findings)
        storage_root = str(config.get("database", {}).get("skill_data_root", "")).strip()
        if not storage_root:
            findings.append(f"missing_skill_data_root:{relative.as_posix()}")


def registry_entries(raw: dict) -> dict[str, dict]:
    return {
        str(entry.get("name", "")).strip(): entry
        for entry in raw.get("skills", [])
        if str(entry.get("name", "")).strip()
    }


def check_registries(root: Path, findings: list[str]) -> None:
    parsed: list[tuple[Path, dict[str, dict]]] = []
    for relative in REGISTRY_PATHS:
        entries = registry_entries(load_toml(root, relative, findings))
        parsed.append((relative, entries))
        for skill_name, expected in PERSISTED_SKILLS.items():
            entry = entries.get(skill_name)
            if entry is None:
                findings.append(f"missing_persisted_skill:{relative.as_posix()}:{skill_name}")
                continue
            actual = entry.get("storage")
            if actual != expected:
                findings.append(
                    f"invalid_storage_declaration:{relative.as_posix()}:{skill_name}"
                )
    if len(parsed) == 2:
        for skill_name in PERSISTED_SKILLS:
            left = parsed[0][1].get(skill_name, {}).get("storage")
            right = parsed[1][1].get(skill_name, {}).get("storage")
            if left != right:
                findings.append(f"registry_storage_drift:{skill_name}")


def production_rust_files(root: Path) -> list[Path]:
    return sorted(
        path
        for path in root.rglob("*.rs")
        if "target" not in path.parts and not is_test_path(path.relative_to(root))
    )


def check_runtime_boundaries(root: Path, findings: list[str]) -> None:
    forbidden_main_table_owners = (
        Path("migrations/004_key_auth.sql"),
        Path("crates/clawd/src/repo/auth.rs"),
        Path("crates/clawd/src/repo/auth/schema.rs"),
    )
    for relative in forbidden_main_table_owners:
        if "exchange_api_credentials" in read_text(root, relative, findings):
            findings.append(f"crypto_table_in_main_storage:{relative.as_posix()}")

    for path in production_rust_files(root):
        relative = path.relative_to(root)
        text = path.read_text(encoding="utf-8")
        if "database_sqlite_path" in text:
            findings.append(f"generic_database_context_field:{relative.as_posix()}")
        if (
            "exchange_api_credentials" in text
            and relative not in ALLOWED_EXCHANGE_TABLE_OWNERS
        ):
            findings.append(f"crypto_table_outside_owner:{relative.as_posix()}")

    for source_root in SKILL_SOURCE_ROOTS:
        absolute_root = root / source_root
        if not absolute_root.exists():
            continue
        for path in sorted(absolute_root.rglob("*.rs")):
            relative = path.relative_to(root)
            if is_test_path(relative) or relative.parts[:3] == (
                "crates",
                "skills",
                "db_basic",
            ):
                continue
            text = path.read_text(encoding="utf-8")
            for marker in ("database.sqlite_path", "data/rustclaw.db", "claw_core::AppConfig"):
                if marker in text:
                    findings.append(
                        f"skill_reads_runtime_main_storage:{relative.as_posix()}:{marker}"
                    )

    runner = read_text(root, Path("crates/clawd/src/skills/runner.rs"), findings)
    for required in ("skill_storage", "registry.storage", "descriptor"):
        if required not in runner:
            findings.append(f"runner_missing_storage_contract:{required}")
    kb_main = read_text(root, Path("crates/skills/kb/src/main.rs"), findings)
    if "skill_storage" not in kb_main:
        findings.append("kb_missing_skill_storage_context")
    resolver = read_text(
        root, Path("crates/clawd/src/skill_storage/resolver.rs"), findings
    )
    for required in ("data/skills", "validate_skill_name", "state.db"):
        if required not in resolver:
            findings.append(f"resolver_missing_boundary:{required}")


def evaluate(root: Path) -> list[str]:
    findings: list[str] = []
    check_configs(root, findings)
    check_registries(root, findings)
    check_runtime_boundaries(root, findings)
    return findings


def write_fixture(root: Path) -> None:
    files = {
        "configs/config.toml": '[database]\nskill_data_root = "data/skills"\n',
        "docker/config/config.toml": '[database]\nskill_data_root = "data/skills"\n',
        "configs/skills_registry.toml": (
            '[[skills]]\nname = "crypto"\nstorage = { kind = "sqlite", '
            'schema_version = 1, migration_owner = "crypto" }\n'
            '[[skills]]\nname = "kb"\nstorage = { kind = "sqlite", '
            'schema_version = 1, migration_owner = "kb" }\n'
        ),
        "docker/config/skills_registry.toml": (
            '[[skills]]\nname = "crypto"\nstorage = { kind = "sqlite", '
            'schema_version = 1, migration_owner = "crypto" }\n'
            '[[skills]]\nname = "kb"\nstorage = { kind = "sqlite", '
            'schema_version = 1, migration_owner = "kb" }\n'
        ),
        "migrations/004_key_auth.sql": "-- auth only\n",
        "crates/clawd/src/repo/auth.rs": "// auth only\n",
        "crates/clawd/src/repo/auth/schema.rs": "// auth only\n",
        "crates/clawd/src/repo/crypto_storage.rs": "exchange_api_credentials\n",
        "crates/clawd/src/skill_storage/migration.rs": "exchange_api_credentials\n",
        "crates/clawd/src/skill_storage/schema.rs": "exchange_api_credentials\n",
        "scripts/import-crypto-credentials.sh": "exchange_api_credentials\n",
        "crates/clawd/src/skills/runner.rs": (
            "fn run() { let _ = (skill_storage, registry.storage(), descriptor); }\n"
        ),
        "crates/clawd/src/skill_storage/resolver.rs": (
            'fn validate_skill_name() {} // data/skills state.db\n'
        ),
        "crates/skills/kb/src/main.rs": "fn main() { let _ = skill_storage; }\n",
    }
    for relative, text in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="skill-storage-ownership-") as tmp:
        root = Path(tmp)
        write_fixture(root)
        findings = evaluate(root)
        if findings:
            print(f"SELF_TEST_FAIL positive findings={findings}")
            return 1

        kb_main = root / "crates/skills/kb/src/main.rs"
        kb_main.write_text(
            'fn main() { let _ = ("data/rustclaw.db", skill_storage); }\n',
            encoding="utf-8",
        )
        findings = evaluate(root)
        expected = (
            "skill_reads_runtime_main_storage:"
            "crates/skills/kb/src/main.rs:data/rustclaw.db"
        )
        if expected not in findings:
            print(f"SELF_TEST_FAIL direct_main_db findings={findings}")
            return 1

        kb_main.write_text("fn main() { let _ = skill_storage; }\n", encoding="utf-8")
        registry = root / "configs/skills_registry.toml"
        registry.write_text(
            registry.read_text(encoding="utf-8").replace(
                'migration_owner = "crypto"', 'migration_owner = "runtime"', 1
            ),
            encoding="utf-8",
        )
        findings = evaluate(root)
        if (
            "invalid_storage_declaration:configs/skills_registry.toml:crypto"
            not in findings
        ):
            print(f"SELF_TEST_FAIL registry_owner findings={findings}")
            return 1

    print("SKILL_STORAGE_OWNERSHIP_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    findings = evaluate(repo_root())
    if findings:
        print("SKILL_STORAGE_OWNERSHIP_CHECK failed")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print("SKILL_STORAGE_OWNERSHIP_CHECK ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
