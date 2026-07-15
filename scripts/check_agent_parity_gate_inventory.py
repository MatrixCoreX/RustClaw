#!/usr/bin/env python3
"""Ensure top-level check scripts are either gated or explicitly exempt."""
from __future__ import annotations

import argparse
import shutil
import tempfile
from pathlib import Path


EXEMPT_CHECKS = {
    "check_agent_loop_trace_release_gate.py": "requires_live_nl_run_dirs",
    "check_route_delta_release_gate.py": "historical_compat_entrypoint_for_live_trace_gate",
}


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def top_level_check_scripts(root: Path) -> list[str]:
    return sorted(path.name for path in (root / "scripts").glob("check_*.py"))


def gate_text(root: Path) -> str:
    return (root / "scripts/nl_tests/run_agent_parity_gate.sh").read_text(encoding="utf-8")


def evaluate_inventory(root: Path, exempt_checks: dict[str, str]) -> tuple[list[str], dict[str, int]]:
    checks = top_level_check_scripts(root)
    gate = gate_text(root)
    findings: list[str] = []
    gated = 0
    exempt = 0

    for name in checks:
        in_gate = name in gate
        is_exempt = name in exempt_checks
        if in_gate and is_exempt:
            findings.append(f"exempt_check_in_default_gate:{name}:{exempt_checks[name]}")
        elif in_gate:
            gated += 1
        elif is_exempt:
            exempt += 1
        else:
            findings.append(f"ungated_check:{name}")

    missing_exempt_files = sorted(set(exempt_checks) - set(checks))
    for name in missing_exempt_files:
        findings.append(f"exempt_check_missing:{name}:{exempt_checks[name]}")

    return findings, {"total": len(checks), "gated": gated, "exempt": exempt}


def write_fixture(root: Path, checks: list[str], gate_lines: list[str]) -> None:
    scripts_dir = root / "scripts"
    gate_dir = scripts_dir / "nl_tests"
    gate_dir.mkdir(parents=True)
    for name in checks:
        (scripts_dir / name).write_text("# fixture\n", encoding="utf-8")
    (gate_dir / "run_agent_parity_gate.sh").write_text(
        "\n".join(gate_lines) + "\n",
        encoding="utf-8",
    )


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="agent-parity-gate-inventory-") as tmp:
        fixture_root = Path(tmp)
        checks = [
            "check_agent_parity_gate_inventory.py",
            "check_agent_loop_trace_release_gate.py",
            "check_route_delta_release_gate.py",
        ]
        write_fixture(
            fixture_root,
            checks,
            ["python3 scripts/check_agent_parity_gate_inventory.py"],
        )
        findings, counts = evaluate_inventory(fixture_root, EXEMPT_CHECKS)
        if findings or counts != {"total": 3, "gated": 1, "exempt": 2}:
            print(f"SELF_TEST_FAIL positive findings={findings} counts={counts}")
            return 1

        missing_root = fixture_root / "missing"
        shutil.copytree(fixture_root / "scripts", missing_root / "scripts")
        (missing_root / "scripts/check_new_guard.py").write_text("# fixture\n", encoding="utf-8")
        findings, _ = evaluate_inventory(missing_root, EXEMPT_CHECKS)
        if "ungated_check:check_new_guard.py" not in findings:
            print(f"SELF_TEST_FAIL missing_check findings={findings}")
            return 1

        exempt_in_gate_root = fixture_root / "exempt-in-gate"
        shutil.copytree(fixture_root / "scripts", exempt_in_gate_root / "scripts")
        (
            exempt_in_gate_root / "scripts/nl_tests/run_agent_parity_gate.sh"
        ).write_text(
            "\n".join(
                [
                    "python3 scripts/check_agent_parity_gate_inventory.py",
                    "python3 scripts/check_agent_loop_trace_release_gate.py",
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        findings, _ = evaluate_inventory(exempt_in_gate_root, EXEMPT_CHECKS)
        expected = (
            "exempt_check_in_default_gate:"
            "check_agent_loop_trace_release_gate.py:requires_live_nl_run_dirs"
        )
        if expected not in findings:
            print(f"SELF_TEST_FAIL exempt_in_gate findings={findings}")
            return 1

    print("AGENT_PARITY_GATE_INVENTORY_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings, counts = evaluate_inventory(repo_root(), EXEMPT_CHECKS)
    if findings:
        print("AGENT_PARITY_GATE_INVENTORY_CHECK failed")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print(
        "AGENT_PARITY_GATE_INVENTORY_CHECK ok "
        f"total={counts['total']} gated={counts['gated']} exempt={counts['exempt']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
