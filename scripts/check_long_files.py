#!/usr/bin/env python3
"""Guard RustClaw against growing oversized Rust source files.

This check intentionally treats the current oversized files as baseline debt:
they may stay or shrink, but new over-threshold files and growth in existing
over-threshold files fail the check.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


PRODUCTION_THRESHOLD = 2_000
TEST_THRESHOLD = 2_000

# Baseline captured on 2026-06-11 after the first agent-loop canary work.
# Existing files above the threshold are debt: future changes should split or
# shrink them instead of growing them further.
BASELINE_LONG_FILES = {
    "crates/claw-core/src/config.rs": 2257,
    "crates/clawd/src/agent_engine/dispatch_support.rs": 1964,
    "crates/clawd/src/agent_engine/loop_control.rs": 3021,
    "crates/clawd/src/agent_engine/loop_control_tests.rs": 2653,
    "crates/clawd/src/agent_engine/observed_output.rs": 10233,
    "crates/clawd/src/agent_engine/observed_output_tests.rs": 10023,
    "crates/clawd/src/agent_engine/planning.rs": 27374,
    "crates/clawd/src/agent_engine/planning_tests.rs": 25819,
    "crates/clawd/src/agent_engine/skill_execution.rs": 1860,
    "crates/clawd/src/answer_verifier.rs": 4226,
    "crates/clawd/src/answer_verifier_tests.rs": 3313,
    "crates/clawd/src/ask_flow.rs": 6600,
    "crates/clawd/src/ask_flow_tests.rs": 6563,
    "crates/clawd/src/contract_matrix.rs": 2432,
    "crates/clawd/src/contract_matrix_tests.rs": 3068,
    "crates/clawd/src/conversation_state.rs": 2051,
    "crates/clawd/src/delivery_utils/tests.rs": 1995,
    "crates/clawd/src/execution_recipe.rs": 1695,
    "crates/clawd/src/finalize/loop_reply.rs": 14165,
    "crates/clawd/src/finalize/loop_reply_tests.rs": 13306,
    "crates/clawd/src/finalize/task.rs": 1864,
    "crates/clawd/src/http/ui_routes.rs": 10004,
    "crates/clawd/src/intent_router.rs": 14722,
    "crates/clawd/src/intent_router_tests.rs": 16715,
    "crates/clawd/src/main.rs": 1651,
    "crates/clawd/src/memory.rs": 1963,
    "crates/clawd/src/prompt_utils.rs": 2247,
    "crates/clawd/src/repo/auth.rs": 1616,
    "crates/clawd/src/runtime/state.rs": 1559,
    "crates/clawd/src/skills.rs": 1869,
    "crates/clawd/src/task_journal.rs": 4872,
    "crates/clawd/src/task_journal_tests.rs": 5417,
    "crates/clawd/src/verifier_tests.rs": 2497,
    "crates/clawd/src/worker/ask_pipeline.rs": 6573,
    "crates/clawd/src/worker/ask_pipeline_tests.rs": 9184,
    "crates/clawd/src/worker/ask_prepare.rs": 2182,
    "crates/clawd/src/worker/ask_prepare_tests.rs": 3259,
    "crates/feishud/src/main.rs": 1637,
    "crates/larkd/src/main.rs": 1517,
    "crates/skills/crypto/src/main.rs": 5288,
    "crates/skills/extension_manager/src/main.rs": 2495,
    "crates/skills/image_edit/src/main.rs": 2444,
    "crates/skills/image_generate/src/main.rs": 1580,
    "crates/skills/image_vision/src/main.rs": 2259,
    "crates/skills/map_merchant/src/main.rs": 1698,
    "crates/skills/photo_organize/src/main.rs": 2351,
    "crates/skills/service_control/src/main.rs": 1920,
    "crates/skills/system_basic/src/main.rs": 2943,
    "crates/telegramd/src/main.rs": 5562,
    "crates/wechatd/src/main.rs": 2751,
}

SKIP_DIRS = {".git", "target", "node_modules", "UI/dist"}


def rel_path(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def is_skipped(path: Path, root: Path) -> bool:
    rel = path.relative_to(root)
    parts = set(rel.parts)
    if parts & {".git", "target", "node_modules"}:
        return True
    return rel.as_posix().startswith("UI/dist/")


def is_test_file(path: Path) -> bool:
    name = path.name
    return name.endswith("_tests.rs") or name == "tests.rs" or "tests" in path.parts


def count_lines(path: Path) -> int:
    with path.open("rb") as handle:
        return sum(1 for _ in handle)


def scan(root: Path) -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    violations: list[dict[str, object]] = []
    debt: list[dict[str, object]] = []
    for path in sorted((root / "crates").rglob("*.rs")):
        if is_skipped(path, root):
            continue
        rel = rel_path(path, root)
        lines = count_lines(path)
        threshold = TEST_THRESHOLD if is_test_file(path) else PRODUCTION_THRESHOLD
        if lines <= threshold:
            continue
        baseline = BASELINE_LONG_FILES.get(rel)
        record = {
            "path": rel,
            "lines": lines,
            "threshold": threshold,
            "baseline": baseline,
        }
        if baseline is None:
            record["reason"] = "new_over_threshold_file"
            violations.append(record)
        elif lines > baseline:
            record["reason"] = "baseline_file_grew"
            violations.append(record)
        else:
            debt.append(record)
    return violations, debt


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument("--json", action="store_true", help="print machine-readable JSON")
    args = parser.parse_args()

    root = Path(args.root).resolve()
    violations, debt = scan(root)
    if args.json:
        print(json.dumps({"violations": violations, "baseline_debt": debt}, indent=2))
    else:
        if violations:
            print("LONG_FILE_CHECK failed")
            for item in violations:
                print(
                    f"- {item['path']}: {item['lines']} lines "
                    f"(threshold {item['threshold']}, baseline {item['baseline']}) "
                    f"reason={item['reason']}"
                )
        else:
            print(f"LONG_FILE_CHECK ok baseline_debt_files={len(debt)}")
    return 1 if violations else 0


if __name__ == "__main__":
    sys.exit(main())
