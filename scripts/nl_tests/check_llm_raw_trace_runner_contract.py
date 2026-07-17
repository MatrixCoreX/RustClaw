#!/usr/bin/env python3
"""Guard NL shell runners' raw LLM trace printing contract.

NL/live NL tests must show each case and its numbered `LLM#1..N` raw return
fields in the Codex chat. The shell runners use `print_llm_raw_trace.py` to
tail `logs/model_io.log` while a task is polling; this checker prevents future
runner edits from silently dropping that plumbing.
"""
from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


@dataclass(frozen=True)
class RunnerContract:
    path: str
    poll_marker: str
    init_marker: str
    require_max_chars: bool = False


RUNNERS = (
    RunnerContract(
        path="scripts/nl_tests/run_manual_test.sh",
        poll_marker='print_new_llm_trace "$task_id" "$llm_offset_file"',
        init_marker='init_llm_trace_offset "$llm_offset_file"',
    ),
    RunnerContract(
        path="scripts/nl_tests/run_multi_turn_suite.sh",
        poll_marker='print_new_llm_trace "$task_id" "$llm_offset_file"',
        init_marker='init_llm_trace_offset "$llm_offset_file"',
    ),
    RunnerContract(
        path="scripts/nl_tests/run_client_like_continuous_suite.sh",
        poll_marker='print_new_llm_trace "$turn" "$task_id"',
        init_marker='init_llm_trace_offset "$LLM_TRACE_STATE_FILE"',
        require_max_chars=True,
    ),
)


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def extract_function(source: str, name: str) -> str | None:
    pattern = re.compile(
        rf"^{re.escape(name)}\(\)\s*\{{\n(?P<body>.*?)(?=^\}}\s*$)",
        re.MULTILINE | re.DOTALL,
    )
    match = pattern.search(source)
    if match is None:
        return None
    return match.group("body")


def has_model_io_log(body: str) -> bool:
    normalized = body.replace("${ROOT_DIR}/", "$ROOT_DIR/")
    return "logs/model_io.log" in body or "logs/model_io.log" in normalized


def require_substrings(text: str, substrings: list[str], label: str) -> list[str]:
    return [f"{label}: missing `{item}`" for item in substrings if item not in text]


def check_runner(root: Path, contract: RunnerContract) -> list[str]:
    path = root / contract.path
    if not path.exists():
        return [f"{contract.path}: missing runner"]

    source = read_text(path)
    failures: list[str] = []

    init_body = extract_function(source, "init_llm_trace_offset")
    if init_body is None:
        failures.append(f"{contract.path}: missing init_llm_trace_offset()")
    else:
        failures.extend(
            require_substrings(
                init_body,
                [
                    "print_llm_raw_trace.py",
                    "--log",
                    "--state-file",
                    "--init-state",
                ],
                f"{contract.path}: init_llm_trace_offset()",
            )
        )
        if "offset_file" not in init_body:
            failures.append(f"{contract.path}: init_llm_trace_offset() must use offset_file")
        if not has_model_io_log(init_body):
            failures.append(
                f"{contract.path}: init_llm_trace_offset() must read logs/model_io.log"
            )

    print_body = extract_function(source, "print_new_llm_trace")
    if print_body is None:
        failures.append(f"{contract.path}: missing print_new_llm_trace()")
    else:
        failures.extend(
            require_substrings(
                print_body,
                [
                    'PRINT_LLM_TRACE:-1',
                    "print_llm_raw_trace.py",
                    "--log",
                    "--task-id",
                    "--state-file",
                ],
                f"{contract.path}: print_new_llm_trace()",
            )
        )
        if "task_id" not in print_body:
            failures.append(f"{contract.path}: print_new_llm_trace() must bind task_id")
        if not has_model_io_log(print_body):
            failures.append(
                f"{contract.path}: print_new_llm_trace() must read logs/model_io.log"
            )
        if contract.require_max_chars and "--max-field-chars" not in print_body:
            failures.append(
                f"{contract.path}: continuous runner must pass --max-field-chars"
            )

    failures.extend(
        require_substrings(
            source,
            [
                'PRINT_LLM_TRACE_VALUE="${PRINT_LLM_TRACE:-1}"',
                contract.init_marker,
                contract.poll_marker,
            ],
            contract.path,
        )
    )
    if contract.path == "scripts/nl_tests/run_multi_turn_suite.sh":
        if 'line.split("|", turn_count)' in source:
            failures.append(
                f"{contract.path}: case parser must reject extra turns instead of merging them"
            )
        if 'line.split("|")' not in source:
            failures.append(
                f"{contract.path}: case parser must split all declared turn fields"
            )
    return failures


def check_helper(root: Path) -> list[str]:
    helper = root / "scripts/nl_tests/print_llm_raw_trace.py"
    if not helper.exists():
        return ["scripts/nl_tests/print_llm_raw_trace.py: missing helper"]
    source = read_text(helper)
    return require_substrings(
        source,
        [
            "[LLM#",
            "raw_fields=",
            "--task-id",
            "--state-file",
            "--init-state",
            "--max-field-chars",
        ],
        "scripts/nl_tests/print_llm_raw_trace.py",
    )


def check_shared_lib(root: Path) -> list[str]:
    path = root / "scripts/lib.sh"
    if not path.exists():
        return ["scripts/lib.sh: missing shared shell library"]
    source = read_text(path)
    failures = require_substrings(
        source,
        [
            'RUSTCLAW_SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"',
            'source "${RUSTCLAW_SCRIPTS_DIR}/shell_compat.sh"',
        ],
        "scripts/lib.sh",
    )
    if re.search(r"^SCRIPT_DIR=", source, re.MULTILINE):
        failures.append(
            "scripts/lib.sh: must not overwrite a sourcing runner's SCRIPT_DIR"
        )
    return failures


def run_self_test() -> int:
    good = """
PRINT_LLM_TRACE_VALUE="${PRINT_LLM_TRACE:-1}"
init_llm_trace_offset() {
  local offset_file="$1"
  python3 "${SCRIPT_DIR}/print_llm_raw_trace.py" \\
    --log "$ROOT_DIR/logs/model_io.log" \\
    --state-file "$offset_file" \\
    --init-state
}
print_new_llm_trace() {
  local task_id="$1"
  local offset_file="$2"
  [[ "${PRINT_LLM_TRACE:-1}" == "1" ]] || return 0
  python3 "${SCRIPT_DIR}/print_llm_raw_trace.py" \\
    --log "$ROOT_DIR/logs/model_io.log" \\
    --task-id "$task_id" \\
    --state-file "$offset_file"
}
poll_until_terminal() {
  print_new_llm_trace "$task_id" "$llm_offset_file"
}
init_llm_trace_offset "$llm_offset_file"
"""
    bad = good.replace("--task-id", "--case-id")
    assert extract_function(good, "init_llm_trace_offset") is not None
    assert extract_function(good, "print_new_llm_trace") is not None
    assert "--task-id" in extract_function(good, "print_new_llm_trace")
    assert "--task-id" not in extract_function(bad, "print_new_llm_trace")
    print("SELF_TEST_OK")
    print("LLM_RAW_TRACE_RUNNER_CONTRACT_SELF_TEST ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        return run_self_test()

    failures = check_helper(ROOT)
    failures.extend(check_shared_lib(ROOT))
    for contract in RUNNERS:
        failures.extend(check_runner(ROOT, contract))

    if failures:
        for failure in failures:
            print(f"[llm-raw-trace-contract] {failure}", file=sys.stderr)
        return 1

    print(f"LLM_RAW_TRACE_RUNNER_CONTRACT ok runners={len(RUNNERS)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
