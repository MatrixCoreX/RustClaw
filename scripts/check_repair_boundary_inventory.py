#!/usr/bin/env python3
"""Validate repair boundary inventory retention/deletion gates."""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SOURCE = REPO_ROOT / "crates/clawd/src/repair_boundary_inventory.rs"

ITEM_RE = re.compile(r"RepairBoundaryInventoryItem\s*\{(?P<body>.*?)\n\s*\}", re.S)
STRING_FIELD_RE = re.compile(r'(?P<key>\w+):\s*"(?P<value>[^"]*)"')
CLASS_RE = re.compile(r"repair_class:\s*RepairBoundaryClass::(?P<value>\w+)")

KNOWN_DELETION_GATES = {
    "keep_schema_compat_boundary",
    "keep_boundary_safety",
    "keep_loop_bounded_recovery",
    "keep_policy_boundary",
    "keep_lifecycle_recovery",
    "delete_after_agent_loop_default",
    "delete_after_agent_loop_followup_gate",
}

LOOP_MIGRATION_TARGETS = {
    "migrate_to_agent_loop_followup_recovery",
    "defer_selected_agent_loop_routes_to_loop",
}


def parse_items() -> list[dict[str, str]]:
    text = SOURCE.read_text(encoding="utf-8")
    _, _, inventory_text = text.partition("pub(crate) const REPAIR_BOUNDARY_INVENTORY")
    if not inventory_text:
        return []
    items: list[dict[str, str]] = []
    for match in ITEM_RE.finditer(inventory_text):
        body = match.group("body")
        fields = {m.group("key"): m.group("value") for m in STRING_FIELD_RE.finditer(body)}
        class_match = CLASS_RE.search(body)
        if class_match:
            fields["repair_class"] = class_match.group("value")
        fields["_line"] = str(text[: text.find(inventory_text) + match.start()].count("\n") + 1)
        items.append(fields)
    return items


def machine_token(value: str) -> bool:
    return bool(re.fullmatch(r"[a-z0-9_]+", value))


def validate(items: list[dict[str, str]]) -> list[str]:
    errors: list[str] = []
    seen: set[str] = set()
    for item in items:
        reason = item.get("reason_code", "")
        prefix = f"{SOURCE.relative_to(REPO_ROOT)}:{item.get('_line', '?')}:{reason or '<missing>'}"
        if not reason:
            errors.append(f"{prefix}: missing reason_code")
            continue
        if reason in seen:
            errors.append(f"{prefix}: duplicate reason_code")
        seen.add(reason)

        gate = item.get("deletion_gate", "")
        if gate not in KNOWN_DELETION_GATES:
            errors.append(f"{prefix}: invalid deletion_gate={gate or '<missing>'}")
        if gate and not machine_token(gate):
            errors.append(f"{prefix}: deletion_gate is not a machine token")

        repair_class = item.get("repair_class", "")
        migration_target = item.get("migration_target", "")
        if repair_class == "OrdinarySemanticRepair" and not gate.startswith("delete_after_"):
            errors.append(f"{prefix}: ordinary semantic repair must use delete_after_*")
        if migration_target in LOOP_MIGRATION_TARGETS and not gate.startswith("delete_after_"):
            errors.append(f"{prefix}: loop migration repair must use delete_after_*")
        if gate.startswith("delete_after_") and not migration_target:
            errors.append(f"{prefix}: deletion gate requires migration_target")
    if not items:
        errors.append(f"{SOURCE.relative_to(REPO_ROOT)}: no repair inventory items found")
    return errors


def main() -> int:
    errors = validate(parse_items())
    if errors:
        print("REPAIR_BOUNDARY_INVENTORY_CHECK failed")
        for error in errors:
            print(f"  - {error}")
        return 1
    print("REPAIR_BOUNDARY_INVENTORY_CHECK ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
