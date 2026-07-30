#!/usr/bin/env python3
"""Reject product-specific identifiers outside product identity inputs."""

from __future__ import annotations

import argparse
import re
import subprocess
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEGACY_LITERAL = re.compile(r"rustclaw", re.IGNORECASE)
COUPLED_SYMBOL = re.compile(
    r"\b(?:fn|struct|enum|type|mod|let|static|const)\s+[A-Za-z0-9_]*rustclaw",
    re.IGNORECASE,
)
MAX_ACTIONABLE_OCCURRENCES = 0

# Branding data is intentionally allowed to carry the selected product name.
# It is exercised by the cross-brand tests and must not consume the source-code
# coupling ratchet. All readers of this file still remain in scope.
IDENTITY_INPUT_FILES = {
    "configs/product_identity.toml",
    "scripts/fixtures/product_identity/brand-primary.toml",
    "scripts/fixtures/product_identity/brand-alternate.toml",
}

SKIP_PARTS = {
    ".git",
    ".agent-runtime",
    "data",
    "logs",
    "node_modules",
    "plan",
    "release-bin",
    "task_termination_logs",
    "target",
}
CODE_SUFFIXES = {
    ".css",
    ".html",
    ".js",
    ".json",
    ".jsonl",
    ".md",
    ".mod",
    ".py",
    ".rs",
    ".service",
    ".sh",
    ".spec",
    ".toml",
    ".ts",
    ".tsx",
    ".txt",
    ".yaml",
    ".yml",
}
CODE_NAMES = {"Dockerfile", "LICENSE"}
def is_code_file(relative: Path) -> bool:
    if any(part in SKIP_PARTS for part in relative.parts):
        return False
    # Runtime prompt markdown is executable model policy, not explanatory
    # documentation. Brand literals here reach every LLM request and must be
    # held to the same zero-coupling ratchet as Rust/UI/shell source.
    if relative.parts and relative.parts[0] == "prompts" and relative.suffix == ".md":
        return True
    return relative.suffix in CODE_SUFFIXES or relative.name in CODE_NAMES


def classify(relative: str, line: str) -> str | None:
    if not LEGACY_LITERAL.search(line):
        return None
    return "unclassified"


def repository_files() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    return [Path(raw.decode()) for raw in result.stdout.split(b"\0") if raw]


def run_inventory() -> int:
    categories: Counter[str] = Counter()
    unknown: list[str] = []
    coupled_paths: list[str] = []
    for relative in repository_files():
        relative_text = relative.as_posix()
        if relative_text == "scripts/check_product_identity_coupling.py":
            continue
        if relative_text in IDENTITY_INPUT_FILES:
            continue
        if not (ROOT / relative).is_file():
            continue
        if LEGACY_LITERAL.search(relative_text) and is_code_file(relative):
            coupled_paths.append(relative_text)
        if not is_code_file(relative):
            continue
        try:
            lines = (ROOT / relative).read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError):
            continue
        for line_number, line in enumerate(lines, 1):
            category = classify(relative_text, line)
            if category is None:
                continue
            categories[category] += len(LEGACY_LITERAL.findall(line))
            if category == "unclassified":
                unknown.append(f"{relative_text}:{line_number}:{line.strip()}")

    total = sum(categories.values())
    actionable = categories["unclassified"]
    print(
        "PRODUCT_IDENTITY_INVENTORY "
        + " ".join(f"{name}={categories[name]}" for name in sorted(categories))
        + f" total={total}"
    )
    if actionable > MAX_ACTIONABLE_OCCURRENCES:
        unknown.append(
            f"actionable occurrence ratchet exceeded: {actionable} > {MAX_ACTIONABLE_OCCURRENCES}"
        )
    if coupled_paths:
        unknown.extend(f"product-coupled code filename: {path}" for path in coupled_paths)
    if unknown:
        print("PRODUCT_IDENTITY_CHECK failed")
        for item in unknown[:100]:
            print(f"- {item}")
        if len(unknown) > 100:
            print(f"- ... {len(unknown) - 100} more")
        return 1
    print("PRODUCT_IDENTITY_CHECK ok")
    return 0


def self_test() -> int:
    assert classify("crates/demo/src/main.rs", "fn rustclaw_start() {}") == "unclassified"
    assert (
        classify(
            "crates/demo/src/main.rs",
            'const LEGACY_AUTH_HEADER: &str = "x-rustclaw-key";',
        )
        == "unclassified"
    )
    assert classify("UI/src/App.tsx", 'const copy = "RustClaw service";') == "unclassified"
    assert classify("scripts/example.sh", "# RustClaw historical note") == "unclassified"
    assert classify("crates/demo/src/main.rs", 'let name = "neutral";') is None
    print("PRODUCT_IDENTITY_CHECK self-test ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    return self_test() if args.self_test else run_inventory()


if __name__ == "__main__":
    raise SystemExit(main())
