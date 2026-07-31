#!/usr/bin/env python3
"""Keep legacy channel delivery-token decoding behind one compatibility module."""
from __future__ import annotations

import argparse
import re
import tempfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
PROTECTED_PATHS = (
    "crates/claw-core/src/wechat_reply_media.rs",
    "crates/claw-core/src/task_delivery_artifacts.rs",
    "crates/clawd/src",
    "crates/telegramd/src",
    "crates/whatsappd/src",
    "crates/wechatd/src",
    "crates/feishud/src",
    "crates/larkd/src",
)
TOKEN_PREFIX = (
    r"(?:IMAGE_FILE|VIDEO_FILE|VOICE_FILE|MUSIC_FILE|FILE_FILE|FILE|"
    r"IMAGE_URL|VIDEO_URL|FILE_URL|MEDIA_URL):"
)
FORBIDDEN_PATTERNS = (
    (
        "inline_legacy_prefix_parser",
        re.compile(
            rf"\.(?:strip_prefix|starts_with|trim_start_matches)\(\s*\"{TOKEN_PREFIX}\""
        ),
    ),
    (
        "duplicate_prefixed_path_parser",
        re.compile(r"fn\s+extract_prefixed_(?:paths|tokens)\s*\("),
    ),
)
REQUIRED_TOKENS_BY_PATH = {
    "crates/claw-core/src/channel_delivery_tokens.rs": (
        "parse_legacy_delivery_line_ref",
        "legacy_delivery_tokens",
        "strip_legacy_delivery_lines",
        "strip_legacy_local_delivery_lines",
        "legacy_local_delivery_lines",
    ),
    "crates/claw-core/src/wechat_reply_media.rs": (
        "legacy_delivery_tokens(answer)",
        "parse_legacy_delivery_line_ref(t)",
    ),
    "crates/clawd/src/finalize/helpers.rs": ("parse_legacy_delivery_line_ref(text)",),
    "crates/telegramd/src/telegram_formatting.rs": (
        "legacy_delivery_tokens(answer)",
        "strip_legacy_local_delivery_lines(answer)",
    ),
    "crates/whatsappd/src/main.rs": (
        "legacy_delivery_tokens(answer)",
        "strip_legacy_local_delivery_lines(answer)",
    ),
}


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
                if candidate.name != "channel_delivery_tokens.rs"
                and not candidate.name.endswith(("_tests.rs", "tests.rs"))
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
                findings.append(
                    Finding(relative, line, kind, text.splitlines()[line - 1].strip())
                )
    for relative, required_tokens in REQUIRED_TOKENS_BY_PATH.items():
        path = root / relative
        if not path.is_file():
            findings.append(Finding(relative, 0, "shared_decoder_user_missing", ""))
            continue
        text = path.read_text(encoding="utf-8")
        for token in required_tokens:
            if token not in text:
                findings.append(Finding(relative, 0, "shared_decoder_token_missing", token))
    return findings


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="channel-delivery-token-contract-") as tmp:
        root = Path(tmp)
        protected = root / "crates/telegramd/src/main.rs"
        protected.parent.mkdir(parents=True)
        protected.write_text(
            'let value = line.strip_prefix("IMAGE_FILE:");\n'
            'fn extract_prefixed_paths() {}\n',
            encoding="utf-8",
        )
        findings = scan(root)
        kinds = {finding.kind for finding in findings}
        if not {
            "inline_legacy_prefix_parser",
            "duplicate_prefixed_path_parser",
        }.issubset(kinds):
            print(f"SELF_TEST_FAIL findings={findings}")
            return 1
    print("CHANNEL_DELIVERY_TOKEN_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = scan(REPO_ROOT)
    if findings:
        print("CHANNEL_DELIVERY_TOKEN_CONTRACT_CHECK failed")
        for finding in findings:
            print(
                f"- {finding.path}:{finding.line}:{finding.kind}:"
                f"{finding.snippet}"
            )
        return 1
    print(
        "CHANNEL_DELIVERY_TOKEN_CONTRACT_CHECK ok "
        f"protected_files={len(protected_files(REPO_ROOT))} findings=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
