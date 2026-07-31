#!/usr/bin/env python3
"""Protect artifact-copy and transport-only channel history boundaries."""
from __future__ import annotations

import argparse
import re
import tempfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CHANNEL_RUNTIME_PATHS = (
    "crates/clawd/src/delivery_service.rs",
    "crates/telegramd/src",
    "crates/whatsappd/src",
    "crates/wechatd/src",
    "crates/feishud/src",
    "crates/larkd/src",
)
FORBIDDEN_HISTORY_WRITES = re.compile(
    r"(?:INSERT\s+INTO\s+(?:conversation|message)|"
    r"(?:append|insert|record|save|write)_[a-z0-9_]*assistant[a-z0-9_]*history)",
    re.IGNORECASE,
)
REQUIRED_TOKENS_BY_PATH = {
    "crates/claw-core/src/channel_delivery.rs": (
        "ChannelDeliveryHistoryDisposition",
        "AssistantResult",
        "TransportOnly",
        "self.notice.is_some()",
        "ChannelDeliverySource::ProactiveNotice",
        "preview.preview_artifact_ref == preview.artifact_ref",
        "ChannelDeliveryReceipt",
    ),
    "crates/clawd/src/http/task_artifacts.rs": (
        "browser_video_cache_path",
        ".video-browser-v1-",
        "video_poster_cache_path",
        ".video-poster-v1-",
    ),
    "crates/clawd/src/http/task_artifacts_tests.rs": (
        "browser_video_cache_is_separate_from_the_original_download",
        "assert_ne!",
    ),
}


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    snippet: str


def runtime_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for relative in CHANNEL_RUNTIME_PATHS:
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
    for path in runtime_files(root):
        text = path.read_text(encoding="utf-8")
        for match in FORBIDDEN_HISTORY_WRITES.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            findings.append(
                Finding(
                    path.relative_to(root).as_posix(),
                    line,
                    "channel_transport_assistant_history_write",
                    text.splitlines()[line - 1].strip(),
                )
            )
    for relative, required_tokens in REQUIRED_TOKENS_BY_PATH.items():
        path = root / relative
        if not path.is_file():
            findings.append(Finding(relative, 0, "contract_file_missing", ""))
            continue
        text = path.read_text(encoding="utf-8")
        for token in required_tokens:
            if token not in text:
                findings.append(Finding(relative, 0, "contract_token_missing", token))
    return findings


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="channel-delivery-history-") as tmp:
        root = Path(tmp)
        path = root / "crates/telegramd/src/main.rs"
        path.parent.mkdir(parents=True)
        path.write_text(
            'db.execute("INSERT INTO conversation_messages VALUES (?)", []);\n',
            encoding="utf-8",
        )
        findings = scan(root)
        if not any(
            finding.kind == "channel_transport_assistant_history_write"
            for finding in findings
        ):
            print(f"SELF_TEST_FAIL findings={findings}")
            return 1
    print("CHANNEL_DELIVERY_HISTORY_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = scan(REPO_ROOT)
    if findings:
        print("CHANNEL_DELIVERY_HISTORY_CONTRACT_CHECK failed")
        for finding in findings:
            print(
                f"- {finding.path}:{finding.line}:{finding.kind}:"
                f"{finding.snippet}"
            )
        return 1
    print(
        "CHANNEL_DELIVERY_HISTORY_CONTRACT_CHECK ok "
        f"runtime_files={len(runtime_files(REPO_ROOT))} findings=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
