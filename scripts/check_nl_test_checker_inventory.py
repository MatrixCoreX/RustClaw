#!/usr/bin/env python3
"""Ensure NL-suite checker scripts are wired into executable release runners."""
from __future__ import annotations

import argparse
import shutil
import tempfile
from pathlib import Path


RUNNER_PATHS = (
    "scripts/nl_tests/run_agent_parity_gate.sh",
    "scripts/nl_tests/run_chinese_provider_smoke_matrix.sh",
)


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def nl_test_checkers(root: Path) -> list[str]:
    return sorted(path.name for path in (root / "scripts/nl_tests").glob("check_*.py"))


def runner_text(root: Path) -> str:
    chunks: list[str] = []
    for rel_path in RUNNER_PATHS:
        path = root / rel_path
        if path.is_file():
            chunks.append(path.read_text(encoding="utf-8"))
    return "\n".join(chunks)


def evaluate_inventory(root: Path) -> tuple[list[str], dict[str, int]]:
    checkers = nl_test_checkers(root)
    text = runner_text(root)
    findings: list[str] = []
    referenced = 0
    for name in checkers:
        if name in text:
            referenced += 1
        else:
            findings.append(f"unreferenced_nl_test_checker:{name}")
    for rel_path in RUNNER_PATHS:
        if not (root / rel_path).is_file():
            findings.append(f"missing_runner:{rel_path}")
    return findings, {"total": len(checkers), "referenced": referenced}


def write_fixture(root: Path, checkers: list[str], runner_lines: list[str]) -> None:
    nl_tests = root / "scripts/nl_tests"
    nl_tests.mkdir(parents=True)
    for name in checkers:
        (nl_tests / name).write_text("# fixture\n", encoding="utf-8")
    (nl_tests / "run_agent_parity_gate.sh").write_text(
        "\n".join(runner_lines) + "\n",
        encoding="utf-8",
    )
    (nl_tests / "run_chinese_provider_smoke_matrix.sh").write_text(
        "# fixture runner\n",
        encoding="utf-8",
    )


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="nl-test-checker-inventory-") as tmp:
        fixture_root = Path(tmp)
        write_fixture(
            fixture_root,
            [
                "check_suite_artifact_contract.py",
                "check_secret_scan_contract.py",
            ],
            [
                "python3 scripts/nl_tests/check_suite_artifact_contract.py --self-test",
                "python3 scripts/nl_tests/check_secret_scan_contract.py --json",
            ],
        )
        findings, counts = evaluate_inventory(fixture_root)
        if findings or counts != {"total": 2, "referenced": 2}:
            print(f"SELF_TEST_FAIL positive findings={findings} counts={counts}")
            return 1

        missing_root = fixture_root / "missing"
        shutil.copytree(fixture_root / "scripts", missing_root / "scripts")
        (missing_root / "scripts/nl_tests/check_new_contract.py").write_text(
            "# fixture\n",
            encoding="utf-8",
        )
        findings, _ = evaluate_inventory(missing_root)
        if "unreferenced_nl_test_checker:check_new_contract.py" not in findings:
            print(f"SELF_TEST_FAIL missing_checker findings={findings}")
            return 1

        missing_runner_root = fixture_root / "missing-runner"
        shutil.copytree(fixture_root / "scripts", missing_runner_root / "scripts")
        (missing_runner_root / "scripts/nl_tests/run_chinese_provider_smoke_matrix.sh").unlink()
        findings, _ = evaluate_inventory(missing_runner_root)
        expected = "missing_runner:scripts/nl_tests/run_chinese_provider_smoke_matrix.sh"
        if expected not in findings:
            print(f"SELF_TEST_FAIL missing_runner findings={findings}")
            return 1

    print("NL_TEST_CHECKER_INVENTORY_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings, counts = evaluate_inventory(repo_root())
    if findings:
        print("NL_TEST_CHECKER_INVENTORY_CHECK failed")
        for finding in findings:
            print(f"- {finding}")
        return 1
    print(
        "NL_TEST_CHECKER_INVENTORY_CHECK ok "
        f"total={counts['total']} referenced={counts['referenced']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
