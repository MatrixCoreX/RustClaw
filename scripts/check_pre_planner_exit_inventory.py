#!/usr/bin/env python3
"""Guard pre-planner exits against untracked semantic route growth."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
INVENTORY_PATH = ROOT / "crates/clawd/src/ask_flow_pre_planner_exit.rs"
SRC_ROOT = ROOT / "crates/clawd/src"

REASON_RE = re.compile(r'reason_code:\s*"([^"]+)"')
ITEM_RE = re.compile(
    r"PrePlannerExitInventoryItem\s*\{(?P<body>.*?)\n\s*\}",
    re.DOTALL,
)
INVENTORY_ARRAY_RE = re.compile(
    r"PRE_PLANNER_EXIT_INVENTORY:[^=]+=\s*&\[(?P<body>.*?)\n\];",
    re.DOTALL,
)
FIELD_STRING_RE = re.compile(r'(?P<name>\w+):\s*"(?P<value>[^"]*)"')
KIND_RE = re.compile(r"kind:\s*PrePlannerExitKind::(?P<kind>\w+)")
ORDER_RE = re.compile(r"migration_order:\s*(?P<order>\d+)")
NL_REFS_RE = re.compile(r"nl_gate_refs:\s*&\[(?P<refs>.*?)\]", re.DOTALL)
CALL_NAME = "with_pre_planner_exit_snapshot"
KNOWN_KINDS = {
    "BoundarySafety",
    "ContractBoundary",
    "EvidenceProjection",
    "MachineFactFastPath",
    "CompatTrace",
    "AgentLoopActivation",
    "OrdinarySemantic",
}
KNOWN_DELETION_GATES = {
    "keep_boundary",
    "keep_contract_boundary",
    "keep_evidence_projection",
    "keep_machine_fact_fast_path",
    "keep_structured_agent_loop_activation_gate",
    "delete_after_agent_loop_default",
    "delete_after_selected_class_release_gate",
    "test_fixture_only",
}


def rust_files() -> list[Path]:
    return sorted(SRC_ROOT.rglob("*.rs"))


def load_inventory() -> set[str]:
    raw = INVENTORY_PATH.read_text(encoding="utf-8")
    return set(REASON_RE.findall(raw))


def rust_string_values(raw: str) -> list[str]:
    return re.findall(r'"([^"]+)"', raw)


def parse_inventory_items() -> list[dict[str, object]]:
    raw = INVENTORY_PATH.read_text(encoding="utf-8")
    array_match = INVENTORY_ARRAY_RE.search(raw)
    if not array_match:
        return []
    array_body = array_match.group("body")
    array_offset = array_match.start("body")
    items: list[dict[str, object]] = []
    for match in ITEM_RE.finditer(array_body):
        body = match.group("body")
        fields = {
            field.group("name"): field.group("value")
            for field in FIELD_STRING_RE.finditer(body)
        }
        kind = KIND_RE.search(body)
        order = ORDER_RE.search(body)
        refs_match = NL_REFS_RE.search(body)
        refs = rust_string_values(refs_match.group("refs")) if refs_match else []
        items.append(
            {
                "line": raw.count("\n", 0, array_offset + match.start()) + 1,
                "reason_code": fields.get("reason_code", ""),
                "kind": kind.group("kind") if kind else "",
                "migration_target": fields.get("migration_target", ""),
                "migration_stage": fields.get("migration_stage", ""),
                "migration_order": int(order.group("order")) if order else -1,
                "nl_gate_refs": refs,
                "deletion_gate": fields.get("deletion_gate", ""),
                "owner_layer": fields.get("owner_layer", ""),
            }
        )
    return items


def validate_inventory_items(items: list[dict[str, object]]) -> list[str]:
    findings: list[str] = []
    seen: set[str] = set()
    for item in items:
        line = item["line"]
        reason = str(item["reason_code"])
        kind = str(item["kind"])
        stage = str(item["migration_stage"])
        target = str(item["migration_target"])
        owner = str(item["owner_layer"])
        deletion_gate = str(item.get("deletion_gate", ""))
        order = int(item["migration_order"])
        refs = item["nl_gate_refs"]
        assert isinstance(refs, list)
        prefix = f"{INVENTORY_PATH.relative_to(ROOT)}:{line}"
        if not reason:
            findings.append(f"{prefix}: missing_reason_code")
        elif reason in seen:
            findings.append(f"{prefix}: duplicate_reason_code={reason}")
        seen.add(reason)
        if kind not in KNOWN_KINDS:
            findings.append(f"{prefix}: unknown_kind={kind or '<missing>'}")
        if not stage:
            findings.append(f"{prefix}: missing_migration_stage")
        if not target:
            findings.append(f"{prefix}: missing_migration_target")
        if not owner:
            findings.append(f"{prefix}: missing_owner_layer")
        if deletion_gate not in KNOWN_DELETION_GATES:
            findings.append(
                f"{prefix}: invalid_deletion_gate={deletion_gate or '<missing>'}"
            )
        if order < 0:
            findings.append(f"{prefix}: missing_migration_order")
        if kind == "OrdinarySemantic":
            if order <= 0:
                findings.append(f"{prefix}: ordinary_semantic_requires_positive_order")
            if not (1 <= len(refs) <= 3):
                findings.append(
                    f"{prefix}: ordinary_semantic_requires_1_to_3_nl_gate_refs"
                )
            if not deletion_gate.startswith("delete_after_"):
                findings.append(
                    f"{prefix}: ordinary_semantic_requires_delete_after_gate"
                )
        if kind == "AgentLoopActivation":
            if deletion_gate != "keep_structured_agent_loop_activation_gate":
                findings.append(
                    f"{prefix}: agent_loop_activation_requires_structured_gate"
                )
            if target != "agent_loop_authority":
                findings.append(
                    f"{prefix}: agent_loop_activation_requires_agent_loop_authority"
                )
            if owner not in {
                "ask_flow_planner_promotion",
                "direct_answer_gate",
            }:
                findings.append(
                    f"{prefix}: agent_loop_activation_requires_known_owner"
                )
        if kind == "ContractBoundary":
            if deletion_gate != "keep_contract_boundary":
                findings.append(f"{prefix}: contract_boundary_requires_keep_gate")
            if target != "planner_loop_contract_boundary":
                findings.append(
                    f"{prefix}: contract_boundary_requires_contract_target"
                )
        if kind == "EvidenceProjection":
            if deletion_gate != "keep_evidence_projection":
                findings.append(f"{prefix}: evidence_projection_requires_keep_gate")
            if "evidence_projection" not in target:
                findings.append(
                    f"{prefix}: evidence_projection_requires_projection_target"
                )
        for ref in refs:
            if not re.fullmatch(r"[a-z0-9_]+", str(ref)):
                findings.append(f"{prefix}: invalid_nl_gate_ref={ref}")
    return findings


def skip_rust_string(raw: str, index: int) -> int:
    quote = raw[index]
    index += 1
    while index < len(raw):
        if raw[index] == "\\":
            index += 2
            continue
        if raw[index] == quote:
            return index + 1
        index += 1
    return index


def call_span(raw: str, name_index: int) -> tuple[int, int] | None:
    open_index = raw.find("(", name_index + len(CALL_NAME))
    if open_index < 0:
        return None
    depth = 0
    index = open_index
    while index < len(raw):
        ch = raw[index]
        if ch in {'"', "'"}:
            index = skip_rust_string(raw, index)
            continue
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return open_index, index + 1
        index += 1
    return None


def rust_string_literals(raw: str) -> list[str]:
    values: list[str] = []
    index = 0
    while index < len(raw):
        ch = raw[index]
        if ch != '"':
            index += 1
            continue
        start = index + 1
        index += 1
        buf: list[str] = []
        while index < len(raw):
            if raw[index] == "\\":
                if index + 1 < len(raw):
                    buf.append(raw[index + 1])
                index += 2
                continue
            if raw[index] == '"':
                values.append("".join(buf) if buf else raw[start:index])
                index += 1
                break
            buf.append(raw[index])
            index += 1
    return values


def find_exit_reasons(path: Path) -> list[tuple[int, str | None]]:
    raw = path.read_text(encoding="utf-8")
    results: list[tuple[int, str | None]] = []
    search_start = 0
    while True:
        name_index = raw.find(CALL_NAME, search_start)
        if name_index < 0:
            return results
        line_start = raw.rfind("\n", 0, name_index) + 1
        if re.search(r"\bfn\s+$", raw[line_start:name_index]):
            search_start = name_index + len(CALL_NAME)
            continue
        line = raw.count("\n", 0, name_index) + 1
        span = call_span(raw, name_index)
        if span is None:
            results.append((line, None))
            search_start = name_index + len(CALL_NAME)
            continue
        literals = rust_string_literals(raw[span[0] : span[1]])
        results.append((line, literals[-1] if literals else None))
        search_start = span[1]


def main() -> int:
    inventory = load_inventory()
    findings: list[str] = []
    items = parse_inventory_items()
    findings.extend(validate_inventory_items(items))
    observed = 0
    for path in rust_files():
        for line, reason in find_exit_reasons(path):
            observed += 1
            rel = path.relative_to(ROOT)
            if not reason:
                findings.append(f"{rel}:{line}: missing_literal_reason")
            elif reason not in inventory:
                findings.append(f"{rel}:{line}: unknown_reason={reason}")
    if findings:
        print("PRE_PLANNER_EXIT_INVENTORY_CHECK findings={}".format(len(findings)))
        for finding in findings:
            print(finding)
        return 1
    print(
        "PRE_PLANNER_EXIT_INVENTORY_CHECK ok calls={} inventory={} items={}".format(
            observed, len(inventory), len(items)
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
