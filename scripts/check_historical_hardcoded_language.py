#!/usr/bin/env python3
"""Inventory historical hardcoded language and NL hard-match debt.

This is an audit tool, not a complete parser. It scans production Rust for:

1. string literals containing CJK/Kana/Hangul characters; and
2. `contains` / `starts_with` / `ends_with` / `Regex::new` style checks using
   natural-language literals.

The goal is to keep multilingual behavior aligned with the Codex/Claude-style
boundary: runtime consumes machine fields, while the model/finalizer/i18n layer
renders visible prose.
"""
from __future__ import annotations

import argparse
import dataclasses
import json
import re
import sys
from collections import Counter
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ROOTS = (REPO_ROOT / "crates", REPO_ROOT / "optional_skills")

NORMAL_STRING_RE = re.compile(r'"((?:\\.|[^"\\])*)"')
RAW_STRING_RE = re.compile(r'r(?P<hashes>#+)?"(?P<body>.*?)"(?P=hashes)', re.S)
FN_RE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
LANG_MATCH_RE = re.compile(
    r"\.(?:contains|starts_with|ends_with)\(\s*(?:[bcfru]*#*)?\""
    r"(?P<literal>(?:\\.|[^\"\\])*)\""
)
REGEX_NEW_RE = re.compile(
    r"Regex::new\(\s*(?:[bcfru]*#*)?\"(?P<literal>(?:\\.|[^\"\\])*)\""
)

TEST_PATH_PARTS = {"tests", "fixtures"}
TEST_FILE_SUFFIXES = ("_tests.rs", "tests.rs", "_test_support.rs")

PROMPT_PATH_HINTS = (
    "/prompt",
    "prompt_",
    "_prompt",
    "/bootstrap/prompts.rs",
    "/providers/fixture_replay.rs",
)

RUNTIME_VISIBLE_HINTS = (
    "crates/clawd/src/agent_engine/",
    "crates/clawd/src/finalize/",
    "crates/clawd/src/delivery_utils/",
    "crates/clawd/src/routing_context.rs",
    "crates/clawd/src/memory.rs",
    "crates/clawd/src/task_journal_evidence_coverage.rs",
    "crates/claw-core/src/wechat_reply_media.rs",
)

CHANNEL_VISIBLE_HINTS = (
    "crates/telegramd/src/",
    "crates/feishud/src/",
    "crates/larkd/src/",
    "crates/wechatd/src/",
    "crates/whatsappd/src/",
)

UI_VISIBLE_HINTS = (
    "crates/clawd/src/http/ui_routes",
)

DOMAIN_METADATA_HINTS = (
    "crates/skills/audio_synthesize/src/",
    "optional_skills/photo_organize/src/",
)

I18N_HINTS = (
    "/i18n.rs",
    "crates/claw-core/src/channel_i18n.rs",
)

MACHINE_OR_LOG_HINTS = (
    "tracing::",
    "debug!",
    "info!",
    "warn!",
    "error!",
    "anyhow!",
    ".context(",
)


@dataclasses.dataclass(frozen=True)
class Finding:
    path: str
    line: int
    function: str
    category: str
    kind: str
    literal: str
    snippet: str
    owner: str
    migration: str


@dataclasses.dataclass(frozen=True)
class AllowedLanguageLiteral:
    path: str
    kind: str
    literal: str
    line_fragment: str
    category: str
    owner: str
    migration: str


ALLOWED_LANGUAGE_LITERALS: tuple[AllowedLanguageLiteral, ...] = (
    AllowedLanguageLiteral(
        path="crates/clawd/src/finalize/helpers.rs",
        kind="string_literal",
        literal="**执行过程**",
        line_fragment="EXECUTION_SUMMARY_MESSAGE_PREFIX",
        category="allowed_legacy_scrub_marker",
        owner="finalizer-history-scrub",
        migration=(
            "Legacy execution-summary removal marker only; production emits "
            "clawd.msg.execution.summary machine JSON and this literal is used "
            "only to strip historical delivery text before final answer output."
        ),
    ),
    AllowedLanguageLiteral(
        path="crates/clawd/src/finalize/helpers.rs",
        kind="string_literal",
        literal="**実行過程**",
        line_fragment="EXECUTION_SUMMARY_MESSAGE_PREFIX_JA",
        category="allowed_legacy_scrub_marker",
        owner="finalizer-history-scrub",
        migration=(
            "Legacy execution-summary removal marker only; production emits "
            "clawd.msg.execution.summary machine JSON and this literal is used "
            "only to strip historical delivery text before final answer output."
        ),
    ),
    AllowedLanguageLiteral(
        path="crates/clawd/src/finalize/helpers.rs",
        kind="string_literal",
        literal="**실행 과정**",
        line_fragment="EXECUTION_SUMMARY_MESSAGE_PREFIX_KO",
        category="allowed_legacy_scrub_marker",
        owner="finalizer-history-scrub",
        migration=(
            "Legacy execution-summary removal marker only; production emits "
            "clawd.msg.execution.summary machine JSON and this literal is used "
            "only to strip historical delivery text before final answer output."
        ),
    ),
)


def rel(path: Path) -> str:
    resolved = path.resolve()
    try:
        return resolved.relative_to(REPO_ROOT).as_posix()
    except ValueError:
        return resolved.as_posix()


def is_test_path(path: Path) -> bool:
    rel_path = rel(path)
    parts = Path(rel_path).parts
    if rel_path.endswith(TEST_FILE_SUFFIXES):
        return True
    return any(part in TEST_PATH_PARTS or part.endswith("_tests") for part in parts)


def is_production_rust(path: Path) -> bool:
    rel_path = rel(path)
    return rel_path.startswith("crates/") and rel_path.endswith(".rs") and not is_test_path(path)


def has_user_language(value: str) -> bool:
    for ch in value:
        code = ord(ch)
        if 0x3400 <= code <= 0x9FFF:
            return True
        if 0x3040 <= code <= 0x30FF:
            return True
        if 0xAC00 <= code <= 0xD7AF:
            return True
    return False


def decode_rust_string_literal(value: str) -> str:
    if "\\" not in value:
        return value

    def replace_unicode(match: re.Match[str]) -> str:
        try:
            return chr(int(match.group(1), 16))
        except (ValueError, OverflowError):
            return match.group(0)

    decoded = value
    decoded = re.sub(r"\\u\{([0-9A-Fa-f]{1,6})\}", replace_unicode, decoded)
    decoded = re.sub(r"\\u([0-9A-Fa-f]{4})", replace_unicode, decoded)
    decoded = re.sub(r"\\U([0-9A-Fa-f]{8})", replace_unicode, decoded)
    decoded = re.sub(r"\\x([0-9A-Fa-f]{2})", replace_unicode, decoded)
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


def strip_line_comment(line: str) -> str:
    escaped = False
    in_string = False
    for idx, ch in enumerate(line):
        if in_string:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_string = False
            continue
        if ch == '"':
            in_string = True
            continue
        if ch == "/" and idx + 1 < len(line) and line[idx + 1] == "/":
            return line[:idx]
    return line


def function_names_by_line(lines: list[str]) -> list[str]:
    names: list[str] = []
    current = "<module>"
    for line in lines:
        match = FN_RE.match(line)
        if match:
            current = match.group("name")
        names.append(current)
    return names


def iter_rust_files(roots: tuple[Path, ...]) -> list[Path]:
    files: list[Path] = []
    for root in roots:
        if root.is_file() and root.suffix == ".rs":
            files.append(root)
        elif root.is_dir():
            files.extend(sorted(root.rglob("*.rs")))
    return [path for path in files if is_production_rust(path)]


def line_no_for_offset(line_offsets: list[int], offset: int) -> int:
    # Small and dependency-free; source files are short enough for linear search.
    line_no = 1
    for idx, start in enumerate(line_offsets):
        if start > offset:
            break
        line_no = idx + 1
    return line_no


def build_line_offsets(lines: list[str]) -> list[int]:
    offsets: list[int] = []
    total = 0
    for line in lines:
        offsets.append(total)
        total += len(line) + 1
    return offsets


def classify(path_rel: str, line: str, kind: str, literal: str) -> tuple[str, str, str]:
    for allowed in ALLOWED_LANGUAGE_LITERALS:
        if (
            path_rel == allowed.path
            and kind == allowed.kind
            and literal == allowed.literal
            and allowed.line_fragment in line
        ):
            return allowed.category, allowed.owner, allowed.migration

    if kind in {"contains_match", "regex_match"}:
        if any(hint in path_rel for hint in I18N_HINTS):
            return (
                "allowed_i18n",
                "i18n",
                "Keep only if this maps locale keys or locale tags, not user semantic routing.",
            )
        return (
            "semantic_hardmatch",
            "runtime-boundary",
            "Move ordinary language understanding to planner/normalizer schema and consume machine fields.",
        )

    if any(hint in path_rel for hint in I18N_HINTS):
        return (
            "allowed_i18n",
            "i18n",
            "Allowed only as keyed UI/channel copy; adapters must not parse it as protocol.",
        )
    if any(hint in path_rel for hint in PROMPT_PATH_HINTS):
        return (
            "prompt_only",
            "prompt",
            "Allowed only as prompt instruction text, not as runtime-visible reply template.",
        )
    if any(hint in path_rel for hint in RUNTIME_VISIBLE_HINTS):
        return (
            "runtime_visible",
            "runtime/finalizer",
            "Replace with structured evidence, message_key, status enum, or finalizer/model rendering.",
        )
    if any(hint in path_rel for hint in CHANNEL_VISIBLE_HINTS):
        return (
            "channel_visible",
            "channel-adapter",
            "Move stable channel copy to i18n resources and dynamic task status to structured fields.",
        )
    if any(hint in path_rel for hint in UI_VISIBLE_HINTS):
        return (
            "ui_visible",
            "ui",
            "Stable console copy may live in i18n/resources; task-specific prose should still come from structured evidence and finalizer/model rendering.",
        )
    if path_rel.startswith(("crates/skills/", "optional_skills/")):
        if any(hint in path_rel for hint in DOMAIN_METADATA_HINTS):
            return (
                "domain_metadata",
                "skill-domain",
                "Keep only if this is provider/domain metadata; do not use it as user-NL routing.",
            )
        return (
            "skill_visible",
            "skill",
            "Add extra.message_key/error_code/structured fields and avoid fixed visible prose.",
        )
    if path_rel.startswith("crates/claw-core/"):
        return (
            "core_visible",
            "core",
            "Prefer machine metadata or i18n resources over localized visible protocol strings.",
        )
    if any(hint in line for hint in MACHINE_OR_LOG_HINTS):
        return (
            "log_or_error",
            "observability",
            "Usually acceptable for logs/errors, but do not surface as final user reply.",
        )
    return (
        "unclassified",
        "unknown",
        "Review and classify before relying on this literal in production behavior.",
    )


def scan_file(path: Path) -> list[Finding]:
    path_rel = rel(path)
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return []
    lines = text.splitlines()
    fn_by_line = function_names_by_line(lines)
    line_offsets = build_line_offsets(lines)
    findings: list[Finding] = []

    for idx, raw_line in enumerate(lines, start=1):
        line = strip_line_comment(raw_line)
        for match in LANG_MATCH_RE.finditer(line):
            literal = decode_rust_string_literal(match.group("literal"))
            if not has_user_language(literal):
                continue
            category, owner, migration = classify(path_rel, line, "contains_match", literal)
            findings.append(
                Finding(
                    path=path_rel,
                    line=idx,
                    function=fn_by_line[idx - 1],
                    category=category,
                    kind="contains_match",
                    literal=literal,
                    snippet=line.strip(),
                    owner=owner,
                    migration=migration,
                )
            )
        for match in REGEX_NEW_RE.finditer(line):
            literal = decode_rust_string_literal(match.group("literal"))
            if not has_user_language(literal):
                continue
            category, owner, migration = classify(path_rel, line, "regex_match", literal)
            findings.append(
                Finding(
                    path=path_rel,
                    line=idx,
                    function=fn_by_line[idx - 1],
                    category=category,
                    kind="regex_match",
                    literal=literal,
                    snippet=line.strip(),
                    owner=owner,
                    migration=migration,
                )
            )

        for literal in NORMAL_STRING_RE.findall(line):
            decoded = decode_rust_string_literal(literal)
            if not has_user_language(decoded):
                continue
            category, owner, migration = classify(path_rel, line, "string_literal", decoded)
            findings.append(
                Finding(
                    path=path_rel,
                    line=idx,
                    function=fn_by_line[idx - 1],
                    category=category,
                    kind="string_literal",
                    literal=decoded,
                    snippet=line.strip(),
                    owner=owner,
                    migration=migration,
                )
            )

    # Raw strings can span lines; scan whole file after line-local scan.
    for match in RAW_STRING_RE.finditer(text):
        literal = match.group("body")
        if not has_user_language(literal):
            continue
        line_no = line_no_for_offset(line_offsets, match.start())
        line = lines[line_no - 1] if 0 <= line_no - 1 < len(lines) else ""
        category, owner, migration = classify(path_rel, line, "raw_string_literal", literal)
        findings.append(
            Finding(
                path=path_rel,
                line=line_no,
                function=fn_by_line[line_no - 1] if fn_by_line else "<module>",
                category=category,
                kind="raw_string_literal",
                literal=literal,
                snippet=line.strip(),
                owner=owner,
                migration=migration,
            )
        )

    # Deduplicate when a raw/normal pattern overlaps.
    unique: dict[tuple[str, int, str, str, str], Finding] = {}
    for item in findings:
        key = (item.path, item.line, item.category, item.kind, item.literal)
        unique[key] = item
    return list(unique.values())


def scan_repo(roots: tuple[Path, ...]) -> list[Finding]:
    findings: list[Finding] = []
    for path in iter_rust_files(roots):
        findings.extend(scan_file(path))
    return findings


def truncate(value: str, max_chars: int = 180) -> str:
    value = value.replace("\n", "\\n")
    if len(value) <= max_chars:
        return value
    return value[: max_chars - 3] + "..."


def print_text_report(findings: list[Finding], max_items: int) -> None:
    counts = Counter(item.category for item in findings)
    total = sum(counts.values())
    ordered = sorted(counts.items(), key=lambda item: (-item[1], item[0]))
    print(f"HISTORICAL_HARDCODED_LANGUAGE_SCAN total={total}")
    for category, count in ordered:
        print(f"  {category}: {count}")
    print()
    for category, _ in ordered:
        items = [item for item in findings if item.category == category]
        print(f"[{category}] showing={min(max_items, len(items))} total={len(items)}")
        for item in items[:max_items]:
            print(f"  - {item.path}:{item.line} fn={item.function} kind={item.kind}")
            print(f"    owner={item.owner}")
            print(f"    literal={truncate(item.literal)!r}")
            print(f"    migration={item.migration}")
            print(f"    {truncate(item.snippet, 220)}")
        print()


def run_self_test() -> int:
    for item in ALLOWED_LANGUAGE_LITERALS:
        assert item.path.startswith("crates/"), item
        assert item.kind, item
        assert item.category.startswith("allowed_"), item
        assert item.owner, item
        assert item.migration, item
        assert item.line_fragment, item
        assert has_user_language(item.literal), item

    samples = {
        REPO_ROOT / "crates/clawd/src/finalize/example.rs": 'fn f() { let s = "处理完成。"; }\n',
        REPO_ROOT / "crates/clawd/src/delivery_utils/example.rs": (
            'fn f(request: &str) -> bool { request.contains("发给我") }\n'
        ),
        REPO_ROOT / "crates/clawd/src/http/ui_routes/example.rs": 'fn f() { let s = "操作完成"; }\n',
        REPO_ROOT / "crates/skills/demo/src/main.rs": 'fn f() { let s = "未配置 API Key"; }\n',
        REPO_ROOT / "crates/skills/demo/src/i18n.rs": 'fn f() { let s = "处理完成"; }\n',
    }
    expected = {
        "runtime_visible",
        "semantic_hardmatch",
        "ui_visible",
        "skill_visible",
        "allowed_i18n",
    }
    got: set[str] = set()
    for path, text in samples.items():
        lines = text.splitlines()
        fn_by_line = function_names_by_line(lines)
        for idx, line in enumerate(lines, start=1):
            for literal in NORMAL_STRING_RE.findall(line):
                if not has_user_language(literal):
                    continue
                category, _, _ = classify(rel(path), line, "string_literal", literal)
                got.add(category)
            for match in LANG_MATCH_RE.finditer(line):
                literal = match.group("literal")
                category, _, _ = classify(rel(path), line, "contains_match", literal)
                got.add(category)
        assert fn_by_line
    assert expected <= got, got
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--json", action="store_true", help="emit JSON report")
    parser.add_argument(
        "--max-items",
        type=int,
        default=12,
        help="max text items per category",
    )
    parser.add_argument(
        "--fail-on-runtime",
        action="store_true",
        help="exit non-zero when runtime_visible or semantic_hardmatch findings exist",
    )
    parser.add_argument(
        "--fail-on-ui-visible",
        action="store_true",
        help="exit non-zero when ui_visible findings exist",
    )
    parser.add_argument(
        "paths",
        nargs="*",
        help="optional Rust files or directories to scan; defaults to crates/",
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()

    roots = (
        tuple((REPO_ROOT / path).resolve() for path in args.paths)
        if args.paths
        else DEFAULT_ROOTS
    )
    findings = scan_repo(roots)
    findings.sort(key=lambda item: (item.category, item.path, item.line, item.kind))
    if args.json:
        print(json.dumps([dataclasses.asdict(item) for item in findings], ensure_ascii=False, indent=2))
    else:
        print_text_report(findings, args.max_items)
    if args.fail_on_runtime and any(
        item.category in {"runtime_visible", "semantic_hardmatch"} for item in findings
    ):
        return 1
    if args.fail_on_ui_visible and any(item.category == "ui_visible" for item in findings):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
