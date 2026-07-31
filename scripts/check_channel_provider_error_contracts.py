#!/usr/bin/env python3
"""Keep channel provider response bodies behind the shared machine error boundary."""
from __future__ import annotations

import argparse
import re
import tempfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
PROTECTED_PATHS = (
    "crates/clawd/src/channel_send.rs",
    "crates/telegramd/src",
    "crates/whatsappd/src",
    "crates/wechatd/src",
    "crates/wechat-ilink/src",
    "crates/feishud/src",
    "crates/larkd/src",
)

FORBIDDEN_PATTERNS = (
    (
        "named_response_body_interpolation",
        re.compile(r"body=\{(?:body|text|response_body|body_preview)\}"),
    ),
    (
        "localized_response_body_parameter",
        re.compile(r'\(\s*"body"\s*,\s*&(body|text|response_body|body_preview)\b'),
    ),
    (
        "positional_http_response_body",
        re.compile(
            r'"(?=[^"\n]*(?:\bhttp\b|\bstatus\s*=))'
            r'(?=[^"\n]*\bbody\s*=)[^"\n]*"'
        ),
    ),
    (
        "raw_application_error_return",
        re.compile(r"return\s+Err\s*\(\s*body\.error\b"),
    ),
    (
        "response_body_content_routing",
        re.compile(r"\b(?:body|text|response_body|body_preview)\.contains\(\s*\"<"),
    ),
)


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    snippet: str


def protected_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for relative in PROTECTED_PATHS:
        path = root / relative
        if path.is_file():
            files.append(path)
        elif path.is_dir():
            files.extend(
                candidate
                for candidate in sorted(path.rglob("*.rs"))
                if not candidate.name.endswith(("_tests.rs", "tests.rs"))
                and "tests" not in candidate.parts
            )
    return files


def scan(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    for path in protected_files(root):
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(root).as_posix()
        for kind, pattern in FORBIDDEN_PATTERNS:
            for match in pattern.finditer(text):
                line = text.count("\n", 0, match.start()) + 1
                snippet = text.splitlines()[line - 1].strip()
                findings.append(Finding(relative, line, kind, snippet))
    return findings


def check_shared_contract(root: Path) -> list[str]:
    path = root / "crates/claw-core/src/channel_provider_error.rs"
    if not path.is_file():
        return ["shared_contract_missing"]
    text = path.read_text(encoding="utf-8")
    required = (
        "CHANNEL_PROVIDER_ERROR_SCHEMA_VERSION",
        "ChannelProviderError",
        "from_http_response",
        "provider_error_code",
        "message_key",
        "retryable",
        "diagnostic_id",
    )
    return [f"shared_contract_field_missing:{value}" for value in required if value not in text]


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="channel-provider-error-contract-") as tmp:
        root = Path(tmp)
        channel_dir = root / "crates/telegramd/src"
        channel_dir.mkdir(parents=True)
        (channel_dir / "main.rs").write_text(
            'return Err(anyhow!("request status={} body={}", status, body));\n'
            'let _ = body.contains("<html");\n',
            encoding="utf-8",
        )
        findings = scan(root)
        kinds = {finding.kind for finding in findings}
        if kinds != {"positional_http_response_body", "response_body_content_routing"}:
            print(f"SELF_TEST_FAIL forbidden findings={findings}")
            return 1

        (channel_dir / "main.rs").write_text(
            'ChannelProviderError::from_http_response("telegram_bot", "send_text", '
            'status.as_u16(), &body);\n',
            encoding="utf-8",
        )
        if scan(root):
            print(f"SELF_TEST_FAIL safe findings={scan(root)}")
            return 1

    print("CHANNEL_PROVIDER_ERROR_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = scan(REPO_ROOT)
    contract_findings = check_shared_contract(REPO_ROOT)
    if findings or contract_findings:
        print("CHANNEL_PROVIDER_ERROR_CONTRACT_CHECK failed")
        for finding in findings:
            print(
                f"- {finding.path}:{finding.line}:{finding.kind}:"
                f"{finding.snippet}"
            )
        for finding in contract_findings:
            print(f"- {finding}")
        return 1
    print(
        "CHANNEL_PROVIDER_ERROR_CONTRACT_CHECK ok "
        f"protected_files={len(protected_files(REPO_ROOT))} findings=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
