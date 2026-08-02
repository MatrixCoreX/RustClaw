#!/usr/bin/env python3
"""Prevent the removed mixed WhatsApp config from becoming authoritative again."""

from __future__ import annotations

import argparse
import tempfile
from dataclasses import dataclass
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib


REPO_ROOT = Path(__file__).resolve().parents[1]
LEGACY_FILE_NAME = "whatsapp" + ".toml"
LEGACY_CONFIG_PATHS = (
    Path("configs/channels") / LEGACY_FILE_NAME,
    Path("docker/config/channels") / LEGACY_FILE_NAME,
)
REQUIRED_TOKENS_BY_PATH = {
    "crates/claw-core/src/config/runtime.rs": (
        "channels/whatsapp-cloud.toml",
        "channels/whatsapp-web.toml",
    ),
    "setup-config.sh": (
        "configs/channels/whatsapp-cloud.toml",
        "configs/channels/whatsapp-web.toml",
        "scripts/whatsapp_web_config_state.py",
    ),
    "scripts/export_runtime_env_from_configs.sh": (
        'configs/channels/whatsapp-cloud.toml',
        'whatsapp = section(whatsapp_cloud_cfg, "whatsapp")',
        'emit("WHATSAPP_CLOUD_ACCESS_TOKEN", value(whatsapp, "access_token"))',
    ),
    "component_start/start-wa-web-bridge.sh": (
        "scripts/whatsapp_web_config_state.py",
        "whatsapp-web.toml",
    ),
}
CONFIG_SECTION_PAIRS = (
    (
        "configs/channels/whatsapp-cloud.toml",
        "docker/config/channels/whatsapp-cloud.toml",
    ),
    (
        "configs/channels/whatsapp-web.toml",
        "docker/config/channels/whatsapp-web.toml",
    ),
)
SCAN_ROOTS = ("crates", "component_start", "scripts", "configs", "docker")
SCAN_SUFFIXES = {".rs", ".sh", ".py", ".toml", ".js", ".ts", ".tsx"}
HISTORICAL_PATH_PREFIXES = (("scripts", "nl_suite_logs"),)


@dataclass(frozen=True)
class Finding:
    path: str
    kind: str
    detail: str = ""


def toml_sections(path: Path) -> set[str]:
    with path.open("rb") as handle:
        data = tomllib.load(handle)
    return set(data) if isinstance(data, dict) else set()


def candidate_files(root: Path):
    for relative_root in SCAN_ROOTS:
        directory = root / relative_root
        if not directory.is_dir():
            continue
        for path in directory.rglob("*"):
            relative_parts = path.relative_to(root).parts
            if any(
                relative_parts[: len(prefix)] == prefix
                for prefix in HISTORICAL_PATH_PREFIXES
            ):
                continue
            if path.is_file() and path.suffix in SCAN_SUFFIXES:
                yield path
    for pattern in ("setup-config.sh", "install*.sh", "start*.sh", "build*.sh"):
        yield from (path for path in root.glob(pattern) if path.is_file())


def scan(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    legacy_reference = "channels/" + LEGACY_FILE_NAME
    for relative in LEGACY_CONFIG_PATHS:
        if (root / relative).exists():
            findings.append(Finding(relative.as_posix(), "legacy_file_present"))
    for path in candidate_files(root):
        relative = path.relative_to(root).as_posix()
        text = path.read_text(encoding="utf-8", errors="ignore")
        if legacy_reference in text:
            findings.append(Finding(relative, "legacy_reference_present", legacy_reference))
    for relative, tokens in REQUIRED_TOKENS_BY_PATH.items():
        path = root / relative
        if not path.is_file():
            findings.append(Finding(relative, "required_file_missing"))
            continue
        text = path.read_text(encoding="utf-8")
        findings.extend(
            Finding(relative, "required_token_missing", token)
            for token in tokens
            if token not in text
        )
    for host_relative, docker_relative in CONFIG_SECTION_PAIRS:
        host_path = root / host_relative
        docker_path = root / docker_relative
        if not host_path.is_file() or not docker_path.is_file():
            findings.append(
                Finding(f"{host_relative}|{docker_relative}", "split_config_missing")
            )
            continue
        try:
            host_sections = toml_sections(host_path)
            docker_sections = toml_sections(docker_path)
        except (OSError, tomllib.TOMLDecodeError) as error:
            findings.append(
                Finding(f"{host_relative}|{docker_relative}", "split_config_invalid", str(error))
            )
            continue
        if host_sections != docker_sections:
            findings.append(
                Finding(
                    f"{host_relative}|{docker_relative}",
                    "section_mismatch",
                    f"host={sorted(host_sections)} docker={sorted(docker_sections)}",
                )
            )
    return findings


def write_complete_fixture(root: Path) -> None:
    for relative, tokens in REQUIRED_TOKENS_BY_PATH.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(tokens), encoding="utf-8")
    configs = {
        "configs/channels/whatsapp-cloud.toml": "[whatsapp]\nenabled=false\n[whatsapp_cloud]\nenabled=false\n",
        "docker/config/channels/whatsapp-cloud.toml": "[whatsapp]\nenabled=false\n[whatsapp_cloud]\nenabled=false\n",
        "configs/channels/whatsapp-web.toml": "[whatsapp_web]\nenabled=false\n",
        "docker/config/channels/whatsapp-web.toml": "[whatsapp_web]\nenabled=false\n",
    }
    for relative, content in configs.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="whatsapp-split-contract-") as raw:
        root = Path(raw)
        write_complete_fixture(root)
        if scan(root):
            print(f"SELF_TEST_FAIL complete_fixture findings={scan(root)}")
            return 1

        legacy_path = root / LEGACY_CONFIG_PATHS[0]
        legacy_path.write_text("# removed entry\n", encoding="utf-8")
        if not any(f.kind == "legacy_file_present" for f in scan(root)):
            print("SELF_TEST_FAIL legacy file was accepted")
            return 1

        legacy_path.unlink()
        setup = root / "setup-config.sh"
        setup.write_text(
            setup.read_text(encoding="utf-8")
            + "\n# "
            + "channels/"
            + LEGACY_FILE_NAME
            + "\n",
            encoding="utf-8",
        )
        if not any(f.kind == "legacy_reference_present" for f in scan(root)):
            print("SELF_TEST_FAIL legacy reference was accepted")
            return 1

        write_complete_fixture(root)
        docker_cloud = root / "docker/config/channels/whatsapp-cloud.toml"
        docker_cloud.write_text("[whatsapp]\nenabled=false\n", encoding="utf-8")
        if not any(f.kind == "section_mismatch" for f in scan(root)):
            print("SELF_TEST_FAIL section mismatch was accepted")
            return 1

    print("WHATSAPP_SPLIT_CONFIG_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()
    findings = scan(REPO_ROOT)
    if findings:
        print("WHATSAPP_SPLIT_CONFIG_CONTRACT_CHECK failed")
        for finding in findings:
            print(f"- {finding.path}:{finding.kind}:{finding.detail}")
        return 1
    print("WHATSAPP_SPLIT_CONFIG_CONTRACT_CHECK ok findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
