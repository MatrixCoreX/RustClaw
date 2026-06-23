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
CALL_NAME = "with_pre_planner_exit_snapshot"


def rust_files() -> list[Path]:
    return sorted(SRC_ROOT.rglob("*.rs"))


def load_inventory() -> set[str]:
    raw = INVENTORY_PATH.read_text(encoding="utf-8")
    return set(REASON_RE.findall(raw))


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
        "PRE_PLANNER_EXIT_INVENTORY_CHECK ok calls={} inventory={}".format(
            observed, len(inventory)
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
