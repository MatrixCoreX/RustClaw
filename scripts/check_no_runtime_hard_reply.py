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
from collections import Counter
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

    def stable_key(self) -> str:
        def encode_field(value: str) -> str:
            return (
                value.replace("\\", "\\\\")
                .replace("\r", "\\r")
                .replace("\n", "\\n")
                .replace("\t", "\\t")
            )

        return "\t".join(
            (
                self.path,
                encode_field(self.literal),
                encode_field(self.source_line.strip()),
            )
        )


def decode_rust_string_literal(value: str) -> str:
    if "\\" not in value:
        return value
    decoded = value
    decoded = re.sub(
        r"\\u\{([0-9A-Fa-f]{1,6})\}",
        lambda match: chr(int(match.group(1), 16)),
        decoded,
    )
    decoded = re.sub(
        r"\\u([0-9A-Fa-f]{4})",
        lambda match: chr(int(match.group(1), 16)),
        decoded,
    )
    replacements = {
        r"\n": "\n",
        r"\r": "\r",
        r"\t": "\t",
        r'\"': '"',
        r"\\": "\\",
    }
    for source, target in replacements.items():
        decoded = decoded.replace(source, target)
    return decoded


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
    if re.fullmatch(r"[A-Za-z0-9_.:-]*(?:\{[A-Za-z0-9_]+\}[A-Za-z0-9_.:-]*)+", value):
        return True
    if re.fullmatch(r"any_of\([A-Za-z0-9_|.:-]+\)", value):
        return True
    if re.fullmatch(r":\([a-z0-9_,!-]+\)[A-Za-z0-9_./*-]+", value):
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


def production_rust_files() -> list[Path]:
    return sorted(
        path
        for path in (REPO_ROOT / "crates").rglob("*.rs")
        if is_production_rust_path(path.relative_to(REPO_ROOT).as_posix())
    )


def scan_file(path: Path) -> list[Candidate]:
    rel_path = path.relative_to(REPO_ROOT).as_posix()
    candidates: list[Candidate] = []
    for line_no, source_line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        for literal in STRING_RE.findall(source_line):
            decoded = decode_rust_string_literal(literal)
            if looks_sentence_like(decoded):
                candidates.append(
                    Candidate(
                        path=rel_path,
                        line=line_no,
                        literal=decoded,
                        source_line=source_line.strip(),
                    )
                )
    return candidates


def scan_all() -> list[Candidate]:
    candidates: list[Candidate] = []
    for path in production_rust_files():
        candidates.extend(scan_file(path))
    return candidates


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


def read_baseline(path: Path) -> Counter[str]:
    if not path.exists():
        raise FileNotFoundError(f"baseline not found: {path}")
    rows = Counter()
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        rows[line] += 1
    return rows


def write_baseline(path: Path, candidates: list[Candidate]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    rows = sorted(candidate.stable_key() for candidate in candidates)
    body = [
        "# Runtime hard reply baseline.",
        "# Format: path<TAB>literal<TAB>source_line. Do not hand-edit line numbers into this file.",
        "# Existing rows are migration debt; new production Rust sentence-like literals should use",
        "# machine fields, message_key, i18n, or LLM/finalizer rendering instead of fixed replies.",
        *rows,
        "",
    ]
    path.write_text("\n".join(body), encoding="utf-8")


def print_baseline_report(
    candidates: list[Candidate],
    baseline_path: Path,
    fail_on_new: bool,
) -> int:
    baseline = read_baseline(baseline_path)
    current = Counter(candidate.stable_key() for candidate in candidates)
    new_rows = sorted((current - baseline).elements())
    removed_rows = sorted((baseline - current).elements())
    print(
        "RUNTIME_HARD_REPLY_ALL_SCAN "
        f"candidates={sum(current.values())} baseline={sum(baseline.values())} "
        f"new={len(new_rows)} removed={len(removed_rows)}"
    )
    if new_rows:
        candidate_by_key = {candidate.stable_key(): candidate for candidate in candidates}
        for key in new_rows:
            item = candidate_by_key.get(key)
            if item is None:
                print(f"  - {key}")
                continue
            print(f"  - {item.path}:{item.line}")
            print(f"    literal={item.literal!r}")
            print(f"    {item.source_line}")
    return 1 if fail_on_new and new_rows else 0


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
    assert candidates[0].stable_key().startswith("crates/clawd/src/finalize/task.rs\t")
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--all",
        action="store_true",
        help="scan all production Rust instead of only the current diff",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        help="baseline file for --all mode; new rows can fail the check",
    )
    parser.add_argument(
        "--update-baseline",
        action="store_true",
        help="write --baseline from the current --all scan",
    )
    parser.add_argument(
        "--fail-on-candidates",
        action="store_true",
        help="exit non-zero when candidates are found",
    )
    parser.add_argument(
        "--fail-on-new",
        action="store_true",
        help="with --all --baseline, exit non-zero when current scan has rows outside baseline",
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    if args.update_baseline and not args.all:
        parser.error("--update-baseline requires --all")
    if args.update_baseline and args.baseline is None:
        parser.error("--update-baseline requires --baseline")
    if args.all:
        candidates = scan_all()
        if args.update_baseline:
            write_baseline(args.baseline, candidates)
            print(
                "RUNTIME_HARD_REPLY_BASELINE_UPDATED "
                f"path={args.baseline} candidates={len(candidates)}"
            )
            return 0
        if args.baseline is not None:
            return print_baseline_report(candidates, args.baseline, args.fail_on_new)
        return print_report(candidates, args.fail_on_candidates)
    if args.baseline is not None or args.fail_on_new:
        parser.error("--baseline/--fail-on-new require --all")
    return print_report(scan_diff(current_diff()), args.fail_on_candidates)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
