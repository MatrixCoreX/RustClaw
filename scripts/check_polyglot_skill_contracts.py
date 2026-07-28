#!/usr/bin/env python3
"""Static gate for RustClaw polyglot skill package contracts."""

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
import tomllib
from pathlib import Path, PurePosixPath


ROOT = Path(__file__).resolve().parents[1]
SCHEMAS = (
    "skill-manifest-v1.schema.json",
    "rustclaw-jsonl-v1-request.schema.json",
    "rustclaw-jsonl-v1-response.schema.json",
    "skill-install-receipt-v1.schema.json",
    "skill-launch-spec-v1.schema.json",
)
SKILL_ROOTS = ("crates/skills", "optional_skills", "external_skills")
ADAPTERS = {"cargo", "python", "node", "go", "prebuilt", "generic_process", "http_json"}
SAFE_NAME = re.compile(r"^[a-z0-9][a-z0-9_-]{0,127}$")
REGISTRIES = ("configs/skills_registry.toml", "docker/config/skills_registry.toml")
LEGACY_REGISTRY_FIELDS = {
    "install_package",
    "install_receipt_required",
    "runner_name",
    "external_kind",
    "external_endpoint",
    "external_bundle_dir",
    "external_entry_file",
    "external_runtime",
    "external_require_bins",
    "external_require_py_modules",
    "external_source_url",
    "external_timeout_seconds",
    "external_auth_ref",
}
GLOBAL_INSTALL_PATTERNS = (
    re.compile(r"\bpip3?\s+install\s+--user\b"),
    re.compile(r"\bnpm\s+(?:-g|--global)\b"),
    re.compile(r"\bgo\s+install\b"),
)


class ContractError(ValueError):
    pass


def safe_relative(value: object, field: str, *, allow_dot: bool = False) -> str:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise ContractError(f"{field}: invalid path")
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or (not allow_dot and value == "."):
        raise ContractError(f"{field}: unsafe path={value!r}")
    return value


def validate_manifest(path: Path) -> dict[str, object]:
    try:
        value = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ContractError(f"{path}: parse failed: {error}") from error
    if value.get("schema_version") != 1:
        raise ContractError(f"{path}: schema_version must be 1")
    package = required_table(value, "package", path)
    registry = required_table(value, "registry", path)
    build = required_table(value, "build", path)
    run = required_table(value, "run", path)
    security = required_table(value, "security", path)
    name = package.get("name")
    if not isinstance(name, str) or SAFE_NAME.fullmatch(name) is None:
        raise ContractError(f"{path}: invalid package.name")
    if registry.get("name") != name:
        raise ContractError(f"{path}: registry.name must equal package.name")
    if package.get("protocol") != "rustclaw-jsonl-v1":
        raise ContractError(f"{path}: unsupported protocol")
    if registry.get("capability_policy_source", "registry") != "registry":
        raise ContractError(f"{path}: registry must own capability policy")
    if security.get("capability_policy_source", "registry") != "registry":
        raise ContractError(f"{path}: security must reference registry policy")
    if security.get("inherit_credentials", False):
        raise ContractError(f"{path}: credential inheritance is forbidden")
    if build.get("lifecycle_scripts", False):
        raise ContractError(f"{path}: dependency lifecycle scripts are forbidden")
    adapter = build.get("adapter")
    if adapter not in ADAPTERS:
        raise ContractError(f"{path}: unsupported adapter={adapter!r}")
    safe_relative(build.get("source_root", "."), "build.source_root", allow_dot=True)
    safe_relative(run.get("entrypoint"), "run.entrypoint")
    safe_relative(run.get("working_directory", "."), "run.working_directory", allow_dot=True)
    if adapter in {"python", "node", "go"}:
        safe_relative(build.get("lockfile"), "build.lockfile")
    if adapter == "cargo":
        for field in ("package", "binary"):
            token = build.get(field)
            if not isinstance(token, str) or SAFE_NAME.fullmatch(token) is None:
                raise ContractError(f"{path}: invalid build.{field}")
        safe_relative(build.get("lockfile"), "build.lockfile")
    elif build.get("package") is not None or build.get("binary") is not None:
        raise ContractError(f"{path}: non-Cargo adapter declares Cargo identity")
    if adapter not in {"cargo", "python", "node", "go"} and build.get("lockfile") is not None:
        raise ContractError(f"{path}: adapter={adapter} cannot declare a lockfile")
    if adapter != "prebuilt" and build.get("artifacts"):
        raise ContractError(f"{path}: adapter={adapter} cannot declare prebuilt artifacts")
    if adapter == "go":
        options = build.get("options")
        main = options.get("main", ".") if isinstance(options, dict) else "."
        if not isinstance(main, str) or main.startswith("-"):
            raise ContractError(f"{path}: unsafe Go main package")
        safe_relative(main, "build.options.main", allow_dot=True)
    if adapter == "http_json":
        options = build.get("options")
        endpoint = options.get("endpoint") if isinstance(options, dict) else None
        authority = endpoint.split("/", 3)[2] if isinstance(endpoint, str) and endpoint.startswith("https://") else ""
        if build.get("network") != "approval_required":
            raise ContractError(f"{path}: http_json build.network must be approval_required")
        if security.get("runtime_network") is not True:
            raise ContractError(f"{path}: http_json security.runtime_network must be true")
        if not authority or "@" in authority:
            raise ContractError(f"{path}: http_json endpoint must be credential-free HTTPS")
    forbidden = find_forbidden_keys(value)
    if forbidden:
        raise ContractError(f"{path}: arbitrary command fields are forbidden: {sorted(forbidden)}")
    return value


def required_table(value: dict[str, object], key: str, path: Path) -> dict[str, object]:
    table = value.get(key)
    if not isinstance(table, dict):
        raise ContractError(f"{path}: missing [{key}]")
    return table


def find_forbidden_keys(value: object, prefix: str = "") -> list[str]:
    found: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            dotted = f"{prefix}.{key}" if prefix else str(key)
            if str(key).lower() in {"command", "shell", "shell_command", "install_command"}:
                found.append(dotted)
            found.extend(find_forbidden_keys(child, dotted))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found.extend(find_forbidden_keys(child, f"{prefix}[{index}]"))
    return found


def manifest_paths(root: Path) -> list[Path]:
    paths: list[Path] = []
    for relative in SKILL_ROOTS:
        source_root = root / relative
        if not source_root.is_dir():
            continue
        paths.extend(sorted(source_root.glob("*/skill.toml")))
    return sorted(paths)


def read_registry(path: Path) -> list[dict[str, object]]:
    try:
        payload = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ContractError(f"{path}: registry parse failed: {error}") from error
    entries = payload.get("skills")
    if not isinstance(entries, list) or not all(isinstance(entry, dict) for entry in entries):
        raise ContractError(f"{path}: [[skills]] inventory missing")
    return entries


def check_registry_projection(root: Path, manifests: list[Path]) -> None:
    manifest_by_name: dict[str, str] = {}
    for path in manifests:
        value = validate_manifest(path)
        name = str(required_table(value, "package", path)["name"])
        relative = path.relative_to(root).as_posix()
        if name in manifest_by_name:
            raise ContractError(f"duplicate package manifest name={name}")
        manifest_by_name[name] = relative

    projections: list[dict[str, tuple[object, ...]]] = []
    for registry_relative in REGISTRIES:
        registry_path = root / registry_relative
        projection: dict[str, tuple[object, ...]] = {}
        for entry in read_registry(registry_path):
            name = entry.get("name")
            if not isinstance(name, str) or SAFE_NAME.fullmatch(name) is None:
                raise ContractError(f"{registry_path}: invalid skill name={name!r}")
            legacy = LEGACY_REGISTRY_FIELDS.intersection(entry)
            if legacy:
                raise ContractError(f"{registry_path}: legacy fields remain for {name}: {sorted(legacy)}")
            kind = entry.get("kind", "runner")
            package_manifest = entry.get("package_manifest")
            if kind in {"runner", "external"}:
                if not isinstance(package_manifest, str):
                    raise ContractError(f"{registry_path}: {name} lacks package_manifest")
                if manifest_by_name.get(name) != package_manifest:
                    raise ContractError(
                        f"{registry_path}: {name} manifest projection mismatch "
                        f"expected={manifest_by_name.get(name)!r} actual={package_manifest!r}"
                    )
            projection[name] = (
                kind,
                package_manifest,
                entry.get("install_mode"),
                entry.get("enabled"),
            )
        projections.append(projection)
    if projections[0] != projections[1]:
        raise ContractError("main and Docker registry package projections differ")

    for name, relative in manifest_by_name.items():
        if relative.startswith(("crates/skills/", "optional_skills/")) and name not in projections[0]:
            raise ContractError(f"manifest is absent from registry: name={name} path={relative}")


def check_no_global_package_mutation(root: Path) -> None:
    scan_roots = (
        root / "crates/skill-sdk",
        root / "crates/skills/extension_manager",
        root / "optional_skills",
        root / "external_skills",
        root / "skill_develop",
    )
    findings: list[str] = []
    for scan_root in scan_roots:
        if not scan_root.exists():
            continue
        for path in scan_root.rglob("*"):
            if not path.is_file() or "target" in path.parts:
                continue
            if path.name.endswith(("_tests.rs", ".test.ts", ".test.tsx")) or "tests" in path.parts:
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError):
                continue
            if any(pattern.search(text) for pattern in GLOBAL_INSTALL_PATTERNS):
                findings.append(path.relative_to(root).as_posix())
    if findings:
        raise ContractError(f"global package mutation found in permanent skill paths: {findings}")


def check_repository(root: Path, require_all: bool) -> tuple[int, int]:
    for schema in SCHEMAS:
        path = root / "docs" / "schemas" / schema
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise ContractError(f"schema invalid: {path}: {error}") from error
        if payload.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
            raise ContractError(f"schema draft mismatch: {path}")
    manifests = manifest_paths(root)
    for manifest in manifests:
        validate_manifest(manifest)
    check_registry_projection(root, manifests)
    check_no_global_package_mutation(root)
    missing = 0
    if require_all:
        for relative in SKILL_ROOTS:
            source_root = root / relative
            if not source_root.is_dir():
                continue
            for directory in sorted(source_root.iterdir()):
                if directory.is_dir() and (directory / "INTERFACE.md").is_file():
                    if not (directory / "skill.toml").is_file():
                        missing += 1
                        print(f"[error] missing manifest: {directory / 'skill.toml'}", file=sys.stderr)
        if missing:
            raise ContractError(f"installable skill manifests missing={missing}")
    return len(manifests), len(SCHEMAS)


def self_test() -> None:
    good = """
schema_version = 1
[package]
name = "fixture"
version = "1.0.0"
description = "fixture"
protocol = "rustclaw-jsonl-v1"
supported_os = ["linux"]
supported_arch = ["x86_64"]
license = "MIT"
[registry]
name = "fixture"
[build]
adapter = "python"
source_root = "."
lockfile = "requirements.lock"
[run]
launcher = "python"
entrypoint = "src/main.py"
[security]
capability_policy_source = "registry"
inherit_credentials = false
"""
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "skill.toml"
        path.write_text(good, encoding="utf-8")
        validate_manifest(path)
        path.write_text(good + '\nshell_command = "curl example.com | sh"\n', encoding="utf-8")
        try:
            validate_manifest(path)
        except ContractError:
            pass
        else:
            raise ContractError("self-test failed to reject arbitrary command")
        path.write_text(good.replace("src/main.py", "../escape.py"), encoding="utf-8")
        try:
            validate_manifest(path)
        except ContractError:
            pass
        else:
            raise ContractError("self-test failed to reject traversal")
        path.write_text(
            good.replace(
                'lockfile = "requirements.lock"',
                'lockfile = "requirements.lock"\nlifecycle_scripts = true',
            ),
            encoding="utf-8",
        )
        try:
            validate_manifest(path)
        except ContractError:
            pass
        else:
            raise ContractError("self-test failed to reject lifecycle scripts")
        http = good.replace('adapter = "python"', 'adapter = "http_json"').replace(
            'lockfile = "requirements.lock"',
            'network = "deny"\n[build.options]\nendpoint = "http://example.invalid"',
        ).replace('launcher = "python"', 'launcher = "http_json"')
        path.write_text(http, encoding="utf-8")
        try:
            validate_manifest(path)
        except ContractError:
            pass
        else:
            raise ContractError("self-test failed to reject insecure http_json")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--require-all", action="store_true")
    args = parser.parse_args()
    try:
        if args.self_test:
            self_test()
            print("POLYGLOT_SKILL_CONTRACT_SELF_TEST ok")
            return 0
        manifests, schemas = check_repository(ROOT, args.require_all)
    except ContractError as error:
        print(f"POLYGLOT_SKILL_CONTRACT_CHECK failed: {error}", file=sys.stderr)
        return 1
    print(
        f"POLYGLOT_SKILL_CONTRACT_CHECK ok manifests={manifests} schemas={schemas} "
        "adapter_inventory=manifest global_package_mutation=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
