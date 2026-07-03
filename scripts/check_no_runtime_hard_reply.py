#!/usr/bin/env python3
"""Advisory scan for newly-added user-facing prose in runtime Rust diff.

This complements check_no_nl_hardmatch.py. It does not try to prove whether a
string is user-visible; it highlights sentence-like literals added to
production Rust files so reviewers can confirm they are i18n/prompt/test data
or machine-only facts, not hardcoded final replies.
"""
from __future__ import annotations

import argparse
import dataclasses
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
STRING_RE = re.compile(r'"((?:\\.|[^"\\])*)"')
DIFF_HEADER_RE = re.compile(r"^\+\+\+ b/(?P<path>.+)$")
HUNK_RE = re.compile(r"^@@ -\d+(?:,\d+)? \+(?P<line>\d+)(?:,\d+)? @@")

ALLOWED_PATH_PARTS = {
    "tests",
}

ALLOWED_FILE_SUFFIXES = (
    "_tests.rs",
    "tests.rs",
)

ALLOWED_FILE_NAMES = {
    "intent_router_prompt_render.rs",
}


@dataclasses.dataclass
class Candidate:
    path: str
    line: int
    literal: str
    source_line: str


def decode_rust_string_literal(value: str) -> str:
    if "\\" not in value:
        return value
    try:
        return bytes(value, "utf-8").decode("unicode_escape")
    except UnicodeDecodeError:
        return value


def is_test_path(path: str) -> bool:
    parts = Path(path).parts
    if Path(path).name in ALLOWED_FILE_NAMES:
        return True
    if path.endswith(ALLOWED_FILE_SUFFIXES):
        return True
    return any(part in ALLOWED_PATH_PARTS or part.endswith("_tests") for part in parts)


def is_production_rust_path(path: str) -> bool:
    if not path.startswith("crates/") or not path.endswith(".rs"):
        return False
    return not is_test_path(path)


def has_cjk(value: str) -> bool:
    return any("\u3400" <= ch <= "\u9fff" for ch in value)


def looks_machine_literal(value: str) -> bool:
    value = value.strip()
    if not value:
        return True
    if value in {"true", "false", "null"}:
        return True
    if "{{" in value or "}}" in value or "://" in value:
        return True
    if "/" in value or "\\" in value:
        return True
    if value.startswith(("__RC_", "clawd.", "agent.", "contract_marker:", "schema:")):
        return True
    if re.fullmatch(r"### [A-Z0-9_]+", value):
        return True
    if re.fullmatch(r"### [A-Z0-9_]+\n\{[A-Za-z0-9_]+\}\n### [A-Z0-9_]+", value):
        return True
    if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*:\s*[A-Za-z0-9_.:-]+", value):
        return True
    if re.fullmatch(r"[A-Z0-9_]+", value):
        return True
    if re.fullmatch(r"[a-z0-9_]+", value):
        return True
    if re.fullmatch(r"[A-Za-z0-9_.:-]+", value) and any(
        ch in value for ch in (".", ":", "-")
    ):
        return True
    if "=" in value and not has_cjk(value):
        return True
    return False


def looks_sentence_like(value: str) -> bool:
    value = decode_rust_string_literal(value).strip()
    if looks_machine_literal(value):
        return False
    if has_cjk(value):
        return True
    words = re.findall(r"[A-Za-z]{2,}", value)
    if len(words) >= 4:
        return True
    return bool(re.search(r"[.!?]\s*$", value)) and len(words) >= 2


def current_diff() -> str:
    result = subprocess.run(
        ["git", "diff", "--unified=0", "--", "crates/clawd/src"],
        cwd=REPO_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode not in (0, 1):
        raise RuntimeError(result.stderr.strip() or "git diff failed")
    return result.stdout


def scan_diff(diff_text: str) -> list[Candidate]:
    candidates: list[Candidate] = []
    current_path: str | None = None
    current_line = 0
    for raw_line in diff_text.splitlines():
        header = DIFF_HEADER_RE.match(raw_line)
        if header:
            current_path = header.group("path")
            current_line = 0
            continue
        hunk = HUNK_RE.match(raw_line)
        if hunk:
            current_line = int(hunk.group("line"))
            continue
        if not raw_line.startswith("+") or raw_line.startswith("+++"):
            continue
        if current_path is None or not is_production_rust_path(current_path):
            current_line += 1
            continue
        source_line = raw_line[1:]
        for literal in STRING_RE.findall(source_line):
            decoded = decode_rust_string_literal(literal)
            if looks_sentence_like(decoded):
                candidates.append(
                    Candidate(
                        path=current_path,
                        line=current_line,
                        literal=decoded,
                        source_line=source_line.strip(),
                    )
                )
        current_line += 1
    return candidates


def print_report(candidates: list[Candidate], fail_on_candidates: bool) -> int:
    print(f"RUNTIME_HARD_REPLY_DIFF_SCAN candidates={len(candidates)}")
    for item in candidates:
        print(f"  - {item.path}:{item.line}")
        print(f"    literal={item.literal!r}")
        print(f"    {item.source_line}")
    return 1 if fail_on_candidates and candidates else 0


def run_self_test() -> int:
    sample = """diff --git a/crates/clawd/src/finalize/task.rs b/crates/clawd/src/finalize/task.rs
+++ b/crates/clawd/src/finalize/task.rs
@@ -10,0 +11,3 @@
+let a = "status_code";
+let b = "I cannot continue with this plan yet.";
+let c = "reason_code=provider_gap";
diff --git a/crates/clawd/src/finalize/task_tests.rs b/crates/clawd/src/finalize/task_tests.rs
+++ b/crates/clawd/src/finalize/task_tests.rs
@@ -10,0 +11,1 @@
+let fixture = "I cannot continue with this plan yet.";
"""
    candidates = scan_diff(sample)
    assert len(candidates) == 1, candidates
    assert candidates[0].literal == "I cannot continue with this plan yet."
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--fail-on-candidates",
        action="store_true",
        help="exit non-zero when candidates are found",
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    return print_report(scan_diff(current_diff()), args.fail_on_candidates)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
