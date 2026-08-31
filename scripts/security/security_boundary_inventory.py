#!/usr/bin/env python3
"""Inventory security boundary regressions and emit a machine-readable result."""

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterable


ROOT = Path(__file__).resolve().parents[2]


@dataclass(frozen=True)
class Finding:
    control_id: str
    path: str
    line: int
    evidence: str


@dataclass(frozen=True)
class TextRule:
    control_id: str
    pattern: re.Pattern[str]


TEXT_RULES = (
    TextRule("SEC-CORS-001", re.compile(r"CorsLayer\s*::\s*permissive\s*\(")),
    TextRule("SEC-CORS-002", re.compile(r"\.allow_origin\s*\(\s*(?:tower_http::cors::)?Any\s*\)")),
    TextRule("SEC-TLS-001", re.compile(r"danger_accept_invalid_(?:certs|hostnames)\s*\(\s*true\s*\)")),
    TextRule("SEC-TLS-002", re.compile(r"NODE_TLS_REJECT_UNAUTHORIZED\s*=\s*['\"]?0")),
    TextRule("SEC-TLS-003", re.compile(r"\bcurl\b[^\n]*(?:\s-k(?:\s|$)|--insecure\b)")),
    TextRule("SEC-SKILL-001", re.compile(r"^\s*(?:runner_name|build_command)\s*=", re.MULTILINE)),
    TextRule(
        "SEC-KEY-001",
        re.compile(r"localStorage\.setItem\s*\([^\n]*(?:userKey|private[_-]?key|api[_-]?key)", re.IGNORECASE),
    ),
    TextRule("SEC-KEY-002", re.compile(r"\.route\s*\(\s*\"/nni/owner/generate\"")),
    TextRule("SEC-KEY-003", re.compile(r"apiFetch\s*\(\s*`?/v1/nni/owner/generate")),
    TextRule("SEC-YOLO-001", re.compile(r"\b(?:APP_UNRESTRICTED_ADMIN|unrestricted_admin)\b")),
)

REQUIRED_CONTROL_IDS = {
    "SEC-BIND-001",
    "SEC-BIND-002",
    "SEC-CSRF-001",
    "SEC-CSRF-002",
    "SEC-CSRF-003",
}

SOURCE_SUFFIXES = {".rs", ".ts", ".tsx", ".js", ".mjs", ".cjs", ".py", ".sh", ".toml"}
def source_files(repository_root: Path) -> Iterable[Path]:
    preferred_roots = (
        repository_root / "crates",
        repository_root / "UI" / "src",
        repository_root / "llm-relay-server",
        repository_root / "optional_skills",
        repository_root / "external_skills",
        repository_root / "scripts",
        repository_root / "server",
        repository_root / "src",
        repository_root / "deploy",
    )
    source_roots = [path for path in preferred_roots if path.exists()]
    if not source_roots:
        source_roots = [repository_root]
    for source_root in source_roots:
        if not source_root.exists():
            continue
        for path in source_root.rglob("*"):
            if not path.is_file() or path.suffix not in SOURCE_SUFFIXES:
                continue
            if path.resolve() == Path(__file__).resolve():
                continue
            relative = path.relative_to(repository_root)
            if any(part in {"target", "node_modules", "dist", "archive", "__pycache__"} for part in relative.parts):
                continue
            yield path


def scan_text(path: str, content: str) -> list[Finding]:
    findings: list[Finding] = []
    for rule in TEXT_RULES:
        for match in rule.pattern.finditer(content):
            findings.append(
                Finding(
                    control_id=rule.control_id,
                    path=path,
                    line=content.count("\n", 0, match.start()) + 1,
                    evidence=match.group(0)[:160],
                )
            )
    return findings


def required_control_findings(repository_root: Path) -> list[Finding]:
    findings: list[Finding] = []
    required_files = (
        (
            repository_root / "crates" / "clawd" / "src" / "main.rs",
            (
                ("SEC-BIND-001", "APP_INTERNAL_LISTEN must use a loopback address"),
                ("SEC-BIND-002", "address.ip().is_loopback()"),
            ),
        ),
        (
            repository_root / "crates" / "webd" / "src" / "main.rs",
            (
                ("SEC-CSRF-001", "require_session_csrf(req.headers(), &session.csrf_token)"),
                ("SEC-CSRF-002", "eq_ignore_ascii_case(WEBD_CSRF_HEADER)"),
            ),
        ),
        (
            repository_root / "UI" / "src" / "lib" / "webd-csrf.ts",
            (
                ("SEC-CSRF-003", 'if (withAuth && authMode === "webd") return "include";'),
                ("SEC-CSRF-003", 'return requested ?? "omit";'),
            ),
        ),
    )
    for path, required_markers in required_files:
        if not path.exists():
            continue
        content = path.read_text(encoding="utf-8")
        for control_id, marker in required_markers:
            if marker not in content:
                findings.append(
                    Finding(
                        control_id=control_id,
                        path=str(path.relative_to(repository_root)),
                        line=1,
                        evidence="required_security_guard_missing",
                    )
                )
    return findings


def run_inventory(repository_root: Path = ROOT) -> dict[str, object]:
    findings: list[Finding] = []
    scanned = 0
    for path in sorted(set(source_files(repository_root))):
        scanned += 1
        try:
            content = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        findings.extend(scan_text(str(path.relative_to(repository_root)), content))
    findings.extend(required_control_findings(repository_root))
    findings.sort(key=lambda item: (item.control_id, item.path, item.line))
    return {
        "schema_version": 1,
        "status": "pass" if not findings else "fail",
        "repository": repository_root.name,
        "scanned_files": scanned,
        "controls": sorted({rule.control_id for rule in TEXT_RULES} | REQUIRED_CONTROL_IDS),
        "findings": [asdict(finding) for finding in findings],
    }


def self_test() -> int:
    fixtures = {
        "SEC-CORS-001": "let cors = CorsLayer::permissive();",
        "SEC-CORS-002": "CorsLayer::new().allow_origin(Any)",
        "SEC-TLS-001": "client.danger_accept_invalid_certs(true);",
        "SEC-TLS-002": "NODE_TLS_REJECT_UNAUTHORIZED=0",
        "SEC-TLS-003": "curl --insecure https://example.test/tool.sh",
        "SEC-SKILL-001": 'build_command = "sh build.sh"',
        "SEC-KEY-001": "localStorage.setItem(STORAGE_KEYS.userKey, secret)",
        "SEC-KEY-002": '.route("/nni/owner/generate", post(handler))',
        "SEC-KEY-003": "apiFetch(`/v1/nni/owner/generate`)",
        "SEC-YOLO-001": "APP_UNRESTRICTED_ADMIN=1",
    }
    detected = {
        finding.control_id
        for control_id, fixture in fixtures.items()
        for finding in scan_text(f"self-test/{control_id}", fixture)
    }
    missing = sorted(set(fixtures) - detected)
    with tempfile.TemporaryDirectory(prefix="security-boundary-self-test-") as fixture_root:
        root = Path(fixture_root)
        clawd_main = root / "crates" / "clawd" / "src" / "main.rs"
        webd_main = root / "crates" / "webd" / "src" / "main.rs"
        csrf_ui = root / "UI" / "src" / "lib" / "webd-csrf.ts"
        for path in (clawd_main, webd_main, csrf_ui):
            path.parent.mkdir(parents=True, exist_ok=True)
        clawd_main.write_text(
            "APP_INTERNAL_LISTEN must use a loopback address\naddress.ip().is_loopback()\n",
            encoding="utf-8",
        )
        webd_main.write_text(
            "require_session_csrf(req.headers(), &session.csrf_token)\n"
            "eq_ignore_ascii_case(WEBD_CSRF_HEADER)\n",
            encoding="utf-8",
        )
        csrf_ui.write_text(
            'if (withAuth && authMode === "webd") return "include";\n'
            'return requested ?? "omit";\n',
            encoding="utf-8",
        )
        if required_control_findings(root):
            missing.append("required_control_positive_fixture")
        webd_main.write_text(
            "eq_ignore_ascii_case(WEBD_CSRF_HEADER)\n",
            encoding="utf-8",
        )
        required_findings = required_control_findings(root)
        if not any(item.control_id == "SEC-CSRF-001" for item in required_findings):
            missing.append("SEC-CSRF-001-negative-fixture")
    missing = sorted(set(missing))
    result = {
        "schema_version": 1,
        "status": "pass" if not missing else "fail",
        "missing_controls": missing,
    }
    print(json.dumps(result, ensure_ascii=True, sort_keys=True))
    return 0 if not missing else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    repository_root = args.root.expanduser().resolve()
    if not repository_root.is_dir():
        parser.error(f"repository root is not a directory: {repository_root}")
    result = run_inventory(repository_root)
    encoded = json.dumps(result, ensure_ascii=True, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
    else:
        sys.stdout.write(encoded)
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
