#!/usr/bin/env python3
"""Check adaptive-limit ownership, P0 fixtures, and new semantic truncation.

This gate does not ban limits. It makes every task-semantic boundary either a
reviewed baseline entry or an explicit inventory migration, so a new `.take`,
`.truncate`, hard count, or display slice cannot silently become product data
loss.
"""

from __future__ import annotations

import argparse
import re
import tempfile
import tomllib
from collections import Counter
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
INVENTORY_PATH = ROOT / "configs/adaptive_limits_inventory.toml"
FIXTURE_PATH = ROOT / "scripts/fixtures/adaptive_limits/p0_cases.toml"
BASELINE_PATH = ROOT / "scripts/baselines/adaptive_semantic_limits_baseline.txt"

EXPECTED_IDS = {f"HL-{index:02d}" for index in range(1, 17)}
EXPECTED_P0_CASES = {
    "productive_model_call_41",
    "capability_25_and_schema_field_9",
    "three_skill_plan",
    "context_beyond_24k",
    "result_9_and_large_result",
    "child_depth_2_and_child_17",
    "repair_4_with_new_evidence",
}
LIMIT_CLASSES = {
    "model_window",
    "task_resource",
    "safety",
    "protocol",
    "external_service",
    "display_cache",
}
DISPOSITIONS = {
    "migrate",
    "migrated",
    "retain_with_recovery",
    "external_constraint",
}
RECOVERIES = {
    "none",
    "opaque_continuation",
    "artifact_range",
    "checkpoint_requeue",
    "retry_after",
    "verified_shard",
}
SOURCE_ROOTS = ("crates", "optional_skills", "UI/src")
SOURCE_SUFFIXES = {".rs", ".ts", ".tsx", ".js", ".jsx"}
MIGRATED_FORBIDDEN_TOKENS = {
    "crates/clawd/src/llm_gateway.rs": (
        "task_llm_budget_exceeded",
        "DEFAULT_MAX_LLM_CALLS_PER_TASK",
        "DEFAULT_MAX_LLM_TOTAL_MS_PER_TASK",
    ),
    "crates/clawd/src/llm_gateway_model_turn.rs": ("task_llm_budget_exceeded",),
    "crates/clawd/src/runtime/state.rs": (
        "fn task_llm_budget_exceeded",
        "llm_max_calls_per_task: u64",
        "llm_total_timeout_ms: u64",
    ),
    "configs/config.toml": (
        "llm_max_calls_per_task =",
        "llm_total_timeout_seconds =",
    ),
}
OP_RE = re.compile(r"\.(take|truncate|clamp|slice)\s*\(([^()\n]{0,240})\)")
HARD_COUNT_RE = re.compile(
    r"\b(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+"
    r"(?P<name>(?:(?:DEFAULT|HARD|SOFT|MIN|MAX)_)+[A-Z0-9_]*"
    r"(?:CALLS|CHARS|BYTES|TOKENS|ITEMS|ENTRIES|FILES|RESULTS|MESSAGES|"
    r"THREADS|RUNS|STEPS|ROUNDS|DEPTH|CHILDREN|PLAYBOOKS|FIELDS|ACTIONS|"
    r"ATTACHMENTS|CURSOR|WINDOW|LIMIT)[A-Z0-9_]*)\b"
)
SEMANTIC_HINT_RE = re.compile(
    r"limit|max_|min_|top_k|count|result|item|entry|file|message|thread|"
    r"history|run|page|cursor|depth|child|playbook|field|action|attachment|"
    r"byte|char|token|timeout|radius",
    re.IGNORECASE,
)


@dataclass(frozen=True)
class Candidate:
    path: str
    kind: str
    expression: str

    def key(self) -> str:
        return "\t".join((self.path, self.kind, self.expression))


def load_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def split_location(value: str) -> tuple[str, str]:
    if "::" not in value:
        raise ValueError("location_missing_symbol")
    return tuple(value.rsplit("::", 1))  # type: ignore[return-value]


def validate_inventory(root: Path, data: dict) -> list[str]:
    findings: list[str] = []
    if data.get("schema_version") != 1:
        findings.append("inventory_schema_version_invalid")
    entries = data.get("limits")
    if not isinstance(entries, list):
        return findings + ["inventory_limits_missing"]
    ids = [entry.get("id") for entry in entries if isinstance(entry, dict)]
    if set(ids) != EXPECTED_IDS or len(ids) != len(EXPECTED_IDS):
        findings.append(f"inventory_ids_invalid:{sorted(str(item) for item in ids)}")
    for entry in entries:
        if not isinstance(entry, dict):
            findings.append("inventory_entry_not_table")
            continue
        limit_id = str(entry.get("id", "missing"))
        for field in (
            "title",
            "owner",
            "terminal_behavior",
            "acceptance_case",
        ):
            if not isinstance(entry.get(field), str) or not entry[field].strip():
                findings.append(f"{limit_id}:missing_{field}")
        if entry.get("class") not in LIMIT_CLASSES:
            findings.append(f"{limit_id}:class_invalid:{entry.get('class')}")
        disposition = entry.get("disposition")
        if disposition not in DISPOSITIONS:
            findings.append(f"{limit_id}:disposition_invalid:{disposition}")
        if entry.get("recovery") not in RECOVERIES:
            findings.append(f"{limit_id}:recovery_invalid:{entry.get('recovery')}")
        locations = entry.get("locations")
        if not isinstance(locations, list) or not locations:
            findings.append(f"{limit_id}:locations_missing")
            continue
        if disposition == "migrated" and not entry.get("migration_tests"):
            findings.append(f"{limit_id}:migration_tests_missing")
        if disposition == "retain_with_recovery" and not entry.get("boundary_tests"):
            findings.append(f"{limit_id}:boundary_tests_missing")
        test_field = (
            "migration_tests"
            if disposition == "migrated"
            else "boundary_tests"
            if disposition == "retain_with_recovery"
            else None
        )
        if test_field:
            for test_anchor in entry.get(test_field, []):
                try:
                    rel_path, symbol = split_location(str(test_anchor))
                except ValueError:
                    findings.append(f"{limit_id}:{test_field}_invalid:{test_anchor}")
                    continue
                path = root / rel_path
                source = path.read_text(encoding="utf-8") if path.is_file() else ""
                if symbol not in source:
                    findings.append(
                        f"{limit_id}:{test_field}_missing:{rel_path}:{symbol}"
                    )
            if disposition == "migrated":
                continue
        for location in locations:
            try:
                rel_path, symbol = split_location(str(location))
            except ValueError:
                findings.append(f"{limit_id}:location_invalid:{location}")
                continue
            path = root / rel_path
            if not path.is_file():
                findings.append(f"{limit_id}:location_file_missing:{rel_path}")
                continue
            try:
                source = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                findings.append(f"{limit_id}:location_not_utf8:{rel_path}")
                continue
            if symbol not in source:
                findings.append(f"{limit_id}:location_symbol_missing:{rel_path}:{symbol}")
    return findings


def validate_migrated_boundary_absence(root: Path) -> list[str]:
    findings: list[str] = []
    for rel_path, tokens in MIGRATED_FORBIDDEN_TOKENS.items():
        path = root / rel_path
        source = path.read_text(encoding="utf-8") if path.is_file() else ""
        for token in tokens:
            if token in source:
                findings.append(f"migrated_boundary_reintroduced:{rel_path}:{token}")
    return findings


def validate_fixtures(root: Path, data: dict, inventory: dict) -> list[str]:
    findings: list[str] = []
    if data.get("schema_version") != 1:
        findings.append("fixture_schema_version_invalid")
    cases = data.get("cases")
    if not isinstance(cases, list):
        return findings + ["fixture_cases_missing"]
    case_ids = {case.get("id") for case in cases if isinstance(case, dict)}
    if case_ids != EXPECTED_P0_CASES or len(cases) != len(EXPECTED_P0_CASES):
        findings.append(f"fixture_ids_invalid:{sorted(str(item) for item in case_ids)}")
    inventory_ids = {
        entry.get("id") for entry in inventory.get("limits", []) if isinstance(entry, dict)
    }
    for case in cases:
        if not isinstance(case, dict):
            findings.append("fixture_entry_not_table")
            continue
        case_id = str(case.get("id", "missing"))
        if case.get("limit_id") not in inventory_ids:
            findings.append(f"{case_id}:unknown_limit_id:{case.get('limit_id')}")
        if case.get("legacy_state") not in {"legacy_failure", "migrated_success"}:
            findings.append(f"{case_id}:legacy_state_invalid")
        if not isinstance(case.get("trigger"), int) or case["trigger"] <= 0:
            findings.append(f"{case_id}:trigger_invalid")
        rel_path = case.get("source_path")
        symbol = case.get("legacy_symbol")
        if not isinstance(rel_path, str) or not isinstance(symbol, str):
            findings.append(f"{case_id}:source_anchor_missing")
            continue
        path = root / rel_path
        source = path.read_text(encoding="utf-8") if path.is_file() else ""
        if case.get("legacy_state") == "legacy_failure" and symbol not in source:
            findings.append(f"{case_id}:legacy_failure_anchor_missing:{rel_path}:{symbol}")
        if case.get("legacy_state") == "migrated_success" and not case.get("test_command"):
            findings.append(f"{case_id}:migrated_test_command_missing")
    return findings


def is_test_path(path: Path) -> bool:
    name = path.name.lower()
    parts = {part.lower() for part in path.parts}
    return (
        "tests" in parts
        or name.endswith(("_tests.rs", ".test.ts", ".test.tsx", ".spec.ts", ".spec.tsx"))
        or name == "main_tests.rs"
    )


def source_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for rel_root in SOURCE_ROOTS:
        source_root = root / rel_root
        if not source_root.exists():
            continue
        for path in source_root.rglob("*"):
            if path.is_file() and path.suffix in SOURCE_SUFFIXES and not is_test_path(path):
                files.append(path)
    return sorted(files)


def normalize_expression(value: str) -> str:
    return re.sub(r"\s+", " ", value.strip())


def scan_file(root: Path, path: Path) -> list[Candidate]:
    rel_path = path.relative_to(root).as_posix()
    source = path.read_text(encoding="utf-8")
    candidates: list[Candidate] = []
    for line in source.splitlines():
        hard_count = HARD_COUNT_RE.search(line)
        if hard_count:
            candidates.append(
                Candidate(rel_path, "hard_count", hard_count.group("name"))
            )
        for match in OP_RE.finditer(line):
            operation, arguments = match.groups()
            arguments = normalize_expression(arguments)
            expression = normalize_expression(match.group(0))
            if operation == "take" and not arguments:
                continue
            if operation in {"slice", "clamp"} and not SEMANTIC_HINT_RE.search(line):
                continue
            candidates.append(Candidate(rel_path, operation, expression))
    return candidates


def scan_candidates(root: Path) -> list[Candidate]:
    return [candidate for path in source_files(root) for candidate in scan_file(root, path)]


def read_baseline(path: Path) -> Counter[str]:
    rows: Counter[str] = Counter()
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line and not line.startswith("#"):
            rows[line] += 1
    return rows


def write_baseline(path: Path, candidates: list[Candidate]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    rows = sorted(candidate.key() for candidate in candidates)
    path.write_text(
        "\n".join(
            [
                "# Reviewed semantic limit baseline.",
                "# Format: path<TAB>kind<TAB>normalized expression.",
                "# Removed rows are allowed; new rows require explicit review.",
                *rows,
                "",
            ]
        ),
        encoding="utf-8",
    )


def compare_baseline(candidates: list[Candidate], baseline: Counter[str]) -> tuple[list[str], int]:
    current = Counter(candidate.key() for candidate in candidates)
    new_rows = sorted((current - baseline).elements())
    removed_count = sum((baseline - current).values())
    return new_rows, removed_count


def run_self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="adaptive-limit-contract-") as tmp:
        root = Path(tmp)
        source = root / "crates/example/src/lib.rs"
        source.parent.mkdir(parents=True)
        source.write_text(
            "const MAX_RESULTS: usize = 8;\n"
            "fn bounded(v: Vec<u8>) { let _ = v.into_iter().take(MAX_RESULTS); }\n",
            encoding="utf-8",
        )
        candidates = scan_candidates(root)
        assert {candidate.kind for candidate in candidates} == {"hard_count", "take"}
        baseline = Counter(candidate.key() for candidate in candidates)
        assert compare_baseline(candidates, baseline) == ([], 0)
        source.write_text(source.read_text(encoding="utf-8") + "fn cut(mut v: Vec<u8>) { v.truncate(3); }\n", encoding="utf-8")
        new_rows, _ = compare_baseline(scan_candidates(root), baseline)
        assert len(new_rows) == 1 and "\ttruncate\t" in new_rows[0]
    print("ADAPTIVE_LIMIT_CONTRACT_SELF_TEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--write-baseline", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        return 0

    inventory = load_toml(INVENTORY_PATH)
    fixtures = load_toml(FIXTURE_PATH)
    findings = validate_inventory(ROOT, inventory)
    findings.extend(validate_fixtures(ROOT, fixtures, inventory))
    findings.extend(validate_migrated_boundary_absence(ROOT))
    candidates = scan_candidates(ROOT)
    if args.write_baseline:
        write_baseline(BASELINE_PATH, candidates)
        print(f"ADAPTIVE_SEMANTIC_LIMIT_BASELINE_WRITTEN candidates={len(candidates)}")
        return 1 if findings else 0
    if not BASELINE_PATH.is_file():
        findings.append("semantic_limit_baseline_missing")
        new_rows: list[str] = []
        removed_count = 0
    else:
        new_rows, removed_count = compare_baseline(candidates, read_baseline(BASELINE_PATH))
        findings.extend(f"new_semantic_limit:{row}" for row in new_rows)
    if findings:
        print(f"ADAPTIVE_LIMIT_INVENTORY_CHECK failed findings={len(findings)}")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print(
        "ADAPTIVE_LIMIT_INVENTORY_CHECK ok "
        f"items={len(inventory['limits'])} fixtures={len(fixtures['cases'])}"
    )
    print(
        "ADAPTIVE_SEMANTIC_LIMIT_BASELINE_CHECK ok "
        f"candidates={len(candidates)} new=0 removed={removed_count}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
