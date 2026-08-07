#!/usr/bin/env python3
"""Generate and verify the long-task execution-mode/timeout inventory."""

from __future__ import annotations

import argparse
import json
import tomllib
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "configs/skills_registry.toml"
CONFIG = ROOT / "configs/config.toml"
OUTPUT = ROOT / "configs/long_task_execution_inventory.json"
EXECUTION_MODES = {"sync_short", "async_preferred", "async_required"}


def load_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def timeout_inventory(value: object, prefix: str = "") -> list[dict]:
    rows: list[dict] = []
    if not isinstance(value, dict):
        return rows
    for key, child in sorted(value.items()):
        path = f"{prefix}.{key}" if prefix else key
        if isinstance(child, dict):
            rows.extend(timeout_inventory(child, path))
        elif "timeout" in key or "retention" in key or "lease" in key:
            rows.append({"path": path, "value": child})
    return rows


def build_inventory() -> tuple[dict, list[str]]:
    registry = load_toml(REGISTRY)
    config = load_toml(CONFIG)
    capabilities: list[dict] = []
    findings: list[str] = []
    for skill in registry.get("skills", []):
        if not isinstance(skill, dict):
            continue
        skill_name = str(skill.get("name", ""))
        skill_timeout = skill.get("timeout_seconds")
        skill_progress = bool(skill.get("progress_frames", False))
        for mapping in skill.get("planner_capabilities", []):
            if not isinstance(mapping, dict):
                continue
            name = str(mapping.get("name", ""))
            mode = str(mapping.get("execution_mode", "sync_short"))
            adapter = mapping.get("async_adapter_kind")
            isolation = mapping.get("isolation_profile")
            timeout = mapping.get("timeout_seconds", skill_timeout)
            if mode not in EXECUTION_MODES:
                findings.append(f"invalid_execution_mode:{skill_name}:{name}:{mode}")
            if mode == "async_required" and not adapter:
                findings.append(f"async_required_adapter_missing:{skill_name}:{name}")
            if isolation == "remote_executor":
                findings.append(f"remote_api_mislabeled_as_executor:{skill_name}:{name}")
            if skill_progress:
                progress_contract = "native_progress"
            elif isinstance(adapter, str) and adapter.endswith("_poll"):
                progress_contract = "poll_status"
            elif mode != "sync_short":
                progress_contract = "alive_only"
            else:
                progress_contract = "not_long_tail"
            capabilities.append(
                {
                    "skill": skill_name,
                    "capability": name,
                    "execution_mode": mode,
                    "adapter_kind": adapter,
                    "progress_contract": progress_contract,
                    "isolation_profile": isolation,
                    "timeout_seconds": timeout,
                }
            )
    capabilities.sort(key=lambda item: (item["skill"], item["capability"]))
    mode_counts = Counter(item["execution_mode"] for item in capabilities)
    progress_counts = Counter(item["progress_contract"] for item in capabilities)
    inventory = {
        "schema_version": 1,
        "sources": [
            "configs/skills_registry.toml",
            "configs/config.toml",
            "configs/agent_guard.toml",
        ],
        "summary": {
            "capability_count": len(capabilities),
            "execution_modes": dict(sorted(mode_counts.items())),
            "progress_contracts": dict(sorted(progress_counts.items())),
            "remote_executor_capability_count": sum(
                item["isolation_profile"] == "remote_executor"
                for item in capabilities
            ),
        },
        "timeout_and_lease_fields": timeout_inventory(config),
        "capabilities": capabilities,
    }
    return inventory, findings


def encoded(inventory: dict) -> str:
    return json.dumps(inventory, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def self_test() -> None:
    assert "async_required_adapter_missing" in (
        "async_required_adapter_missing:skill:capability"
    )
    assert timeout_inventory({"worker": {"lease_seconds": 5}}) == [
        {"path": "worker.lease_seconds", "value": 5}
    ]
    print("LONG_TASK_EXECUTION_INVENTORY_SELF_TEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    inventory, findings = build_inventory()
    body = encoded(inventory)
    if args.write:
        OUTPUT.write_text(body, encoding="utf-8")
    elif not OUTPUT.is_file() or OUTPUT.read_text(encoding="utf-8") != body:
        findings.append("long_task_execution_inventory_out_of_date")
    if findings:
        print(f"LONG_TASK_EXECUTION_INVENTORY findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print(
        "LONG_TASK_EXECUTION_INVENTORY ok "
        f"capabilities={inventory['summary']['capability_count']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
