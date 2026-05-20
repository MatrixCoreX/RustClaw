#!/usr/bin/env python3
"""Guard against adding user-language hard matching to runtime Rust code.

This is intentionally conservative: it catches suspicious user_text/prompt
`contains("natural language")` checks in production code, while allowing known
legacy debt to remain visible until the structured-contract migration removes it.
"""
from __future__ import annotations

import argparse
import dataclasses
import re
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ROOTS = (REPO_ROOT / "crates",)

USER_TEXT_SEED_NAMES = {
    "current_user_request",
    "original_user_text",
    "prompt",
    "resolved_prompt",
    "source_request",
    "user_request",
    "user_text",
}

CONTAINS_RE = re.compile(
    r"\b(?P<receiver>[A-Za-z_][A-Za-z0-9_]*)\.contains\(\s*"
    r"(?:[bcfru]*#*)?\"(?P<literal>(?:\\.|[^\"\\])*)\""
)
CONTAINS_RECEIVER_RE = re.compile(
    r"\b(?P<receiver>[A-Za-z_][A-Za-z0-9_]*)\.contains\("
)
FN_RE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
STRING_RE = re.compile(r'"((?:\\.|[^"\\])*)"')
ARRAY_ANY_CONTAINS_RE = re.compile(
    r"\[(?P<body>.{0,1200}?)\]\s*\.iter\(\)\s*\.any\s*\([^)]*contains\(",
    re.S,
)


@dataclasses.dataclass(frozen=True)
class KnownLegacy:
    path: str
    function: str
    reason: str
    removal_plan: str


KNOWN_LEGACY: tuple[KnownLegacy, ...] = ()


@dataclasses.dataclass
class Finding:
    path: str
    line: int
    function: str
    kind: str
    snippet: str
    literal: str
    known: KnownLegacy | None = None


def rel(path: Path) -> str:
    resolved = path.resolve()
    try:
        return resolved.relative_to(REPO_ROOT).as_posix()
    except ValueError:
        return resolved.as_posix()


def decode_rust_string_literal(value: str) -> str:
    if "\\" not in value:
        return value
    try:
        return bytes(value, "utf-8").decode("unicode_escape")
    except UnicodeDecodeError:
        return value


def has_cjk(value: str) -> bool:
    return any("\u3400" <= ch <= "\u9fff" for ch in value)


def has_latin_word(value: str) -> bool:
    return re.search(r"[A-Za-z]{2,}", value) is not None


def looks_structural_literal(value: str) -> bool:
    value = value.strip()
    if not value:
        return True
    if value in {"true", "false", "null"}:
        return True
    if "{{" in value or "}}" in value or "://" in value:
        return True
    if "/" in value or "\\" in value:
        return True
    if re.fullmatch(r"[A-Z0-9_]+", value):
        return True
    if re.fullmatch(r"[a-z0-9_]+", value) and "_" in value:
        return True
    if re.fullmatch(r"[A-Za-z0-9_.:-]+", value) and any(
        ch in value for ch in (".", ":", "-")
    ):
        return True
    return False


def looks_natural_language_literal(value: str) -> bool:
    value = decode_rust_string_literal(value).strip()
    if looks_structural_literal(value):
        return False
    return has_cjk(value) or has_latin_word(value)


def strip_cfg_test_modules(lines: list[str]) -> list[str]:
    stripped = lines[:]
    pending_cfg_test = False
    skip_depth = 0
    for idx, line in enumerate(lines):
        if skip_depth > 0:
            stripped[idx] = ""
            skip_depth += line.count("{") - line.count("}")
            continue
        if "#[cfg(test)]" in line:
            stripped[idx] = ""
            pending_cfg_test = True
            if "mod " in line and "{" in line:
                skip_depth = max(1, line.count("{") - line.count("}"))
                pending_cfg_test = False
            continue
        if pending_cfg_test:
            stripped[idx] = ""
            if re.match(r"\s*mod\s+[A-Za-z_][A-Za-z0-9_]*\s*\{", line):
                skip_depth = max(1, line.count("{") - line.count("}"))
                pending_cfg_test = False
                continue
            if line.strip() and not line.lstrip().startswith("#["):
                pending_cfg_test = False
            continue
    return stripped


def function_names_by_line(lines: list[str]) -> list[str]:
    names: list[str] = []
    current = "<module>"
    for line in lines:
        match = FN_RE.match(line)
        if match:
            current = match.group("name")
        names.append(current)
    return names


def tainted_user_text_vars_by_line(lines: list[str]) -> list[set[str]]:
    tainted: set[str] = set()
    by_line: list[set[str]] = []
    seed_pattern = re.compile(
        r"\b(?P<name>" + "|".join(sorted(USER_TEXT_SEED_NAMES)) + r")\s*:"
    )
    let_pattern = re.compile(r"\blet\s+(?:mut\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=")
    assign_pattern = re.compile(r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=")
    push_str_pattern = re.compile(r"\b(?P<name>[A-Za-z_][A-Za-z0-9_]*)\.push_str\(")

    for line in lines:
        if FN_RE.match(line):
            tainted = set()

        for match in seed_pattern.finditer(line):
            tainted.add(match.group("name"))

        rhs_uses_tainted = any(re.search(rf"\b{re.escape(name)}\b", line) for name in tainted)
        let_match = let_pattern.search(line)
        if let_match and rhs_uses_tainted:
            tainted.add(let_match.group("name"))
        assign_match = assign_pattern.search(line)
        if assign_match and rhs_uses_tainted:
            tainted.add(assign_match.group("name"))
        push_match = push_str_pattern.search(line)
        if push_match and rhs_uses_tainted:
            tainted.add(push_match.group("name"))

        by_line.append(set(tainted))
    return by_line


def known_legacy_for(path: str, function: str) -> KnownLegacy | None:
    for item in KNOWN_LEGACY:
        if item.path == path and item.function == function:
            return item
    return None


def scan_source(path: Path, text: str) -> list[Finding]:
    path_rel = rel(path) if path.is_absolute() else path.as_posix()
    lines = text.splitlines()
    effective = strip_cfg_test_modules(lines)
    fn_names = function_names_by_line(effective)
    tainted_by_line = tainted_user_text_vars_by_line(effective)
    findings: list[Finding] = []

    for idx, line in enumerate(effective, start=1):
        for match in CONTAINS_RE.finditer(line):
            receiver = match.group("receiver")
            literal = match.group("literal")
            if receiver not in tainted_by_line[idx - 1]:
                continue
            if not looks_natural_language_literal(literal):
                continue
            function = fn_names[idx - 1] if idx - 1 < len(fn_names) else "<module>"
            findings.append(
                Finding(
                    path=path_rel,
                    line=idx,
                    function=function,
                    kind="contains_call",
                    snippet=line.strip(),
                    literal=decode_rust_string_literal(literal),
                    known=known_legacy_for(path_rel, function),
                )
            )

    effective_text = "\n".join(effective)
    line_offsets = [0]
    total = 0
    for line in effective:
        total += len(line) + 1
        line_offsets.append(total)
    for match in ARRAY_ANY_CONTAINS_RE.finditer(effective_text):
        match_text = match.group(0)
        start = match.start()
        line_no = 1
        for idx, offset in enumerate(line_offsets):
            if offset > start:
                break
            line_no = idx + 1
        tainted_at_match = tainted_by_line[min(line_no - 1, len(tainted_by_line) - 1)]
        contains_receivers = {
            item.group("receiver") for item in CONTAINS_RECEIVER_RE.finditer(match_text)
        }
        if not (contains_receivers & tainted_at_match):
            continue
        literals = [
            decode_rust_string_literal(item)
            for item in STRING_RE.findall(match.group("body"))
        ]
        natural = [item for item in literals if looks_natural_language_literal(item)]
        if len(natural) < 2:
            continue
        function = fn_names[min(line_no - 1, len(fn_names) - 1)] if fn_names else "<module>"
        findings.append(
            Finding(
                path=path_rel,
                line=line_no,
                function=function,
                kind="literal_array_contains",
                snippet="array literal followed by iter().any(...contains(...))",
                literal=", ".join(natural[:5]),
                known=known_legacy_for(path_rel, function),
            )
        )
    return findings


def iter_rust_files(roots: tuple[Path, ...]) -> list[Path]:
    files: list[Path] = []
    for root in roots:
        if root.is_file() and root.suffix == ".rs":
            files.append(root)
        elif root.is_dir():
            files.extend(sorted(root.rglob("*.rs")))
    return files


def scan_repo(roots: tuple[Path, ...]) -> list[Finding]:
    findings: list[Finding] = []
    for path in iter_rust_files(roots):
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        findings.extend(scan_source(path, text))
    return findings


def print_report(findings: list[Finding]) -> int:
    unknown = [item for item in findings if item.known is None]
    known = [item for item in findings if item.known is not None]
    print(
        f"NL_HARDMATCH_SCAN unknown={len(unknown)} known_legacy={len(known)}"
    )
    if unknown:
        print("\nUnknown natural-language hard-match candidates:")
        for item in unknown:
            print(f"  - {item.path}:{item.line} fn={item.function} kind={item.kind}")
            print(f"    literal={item.literal!r}")
            print(f"    {item.snippet}")
    if known:
        print("\nKnown legacy candidates:")
        for item in known:
            assert item.known is not None
            print(f"  - {item.path}:{item.line} fn={item.function}")
            print(f"    literal={item.literal!r}")
            print(f"    reason={item.known.reason}")
            print(f"    removal_plan={item.known.removal_plan}")
    return 1 if unknown else 0


def run_self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        bad = root / "bad.rs"
        bad.write_text(
            'fn route(prompt: &str) -> bool {\n    prompt.contains("当前机器")\n}\n',
            encoding="utf-8",
        )
        good = root / "good.rs"
        good.write_text(
            'fn route(action: &str, token: &str) -> bool {\n'
            '    action == "read_field" || token.contains("://")\n'
            "}\n",
            encoding="utf-8",
        )
        tests = root / "tests.rs"
        tests.write_text(
            "#[cfg(test)]\nmod tests {\n"
            "    #[test]\n    fn fixture(prompt: &str) { assert!(prompt.contains(\"当前机器\")); }\n"
            "}\n",
            encoding="utf-8",
        )
        array_bad = root / "array_bad.rs"
        array_bad.write_text(
            'fn route(prompt: &str) -> bool {\n'
            '    ["设置", "修改"].iter().any(|marker| prompt.contains(marker))\n'
            "}\n",
            encoding="utf-8",
        )

        bad_findings = scan_source(bad, bad.read_text(encoding="utf-8"))
        good_findings = scan_source(good, good.read_text(encoding="utf-8"))
        test_findings = scan_source(tests, tests.read_text(encoding="utf-8"))
        array_findings = scan_source(array_bad, array_bad.read_text(encoding="utf-8"))
        assert len(bad_findings) == 1, bad_findings
        assert not good_findings, good_findings
        assert not test_findings, test_findings
        assert len(array_findings) == 1, array_findings
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run script self-tests and exit",
    )
    parser.add_argument(
        "paths",
        nargs="*",
        help="optional Rust files or directories to scan; defaults to crates/",
    )
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    roots = tuple((REPO_ROOT / path).resolve() for path in args.paths) if args.paths else DEFAULT_ROOTS
    return print_report(scan_repo(roots))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
