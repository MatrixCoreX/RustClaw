#!/usr/bin/env python3
"""Protect Telegram media preflight, downgrade, and Web fallback boundaries."""
from __future__ import annotations

import argparse
import tempfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
REQUIRED_TOKENS_BY_PATH = {
    "crates/claw-core/src/channel_media_limits.rs": (
        "LocalMediaPreflightFailure",
        "LocalMediaPreflightError",
        "preflight_local_media_file",
        "channel_media_too_large",
    ),
    "crates/claw-core/src/task_delivery_artifacts.rs": (
        "is_managed_task_delivery_artifact_path",
        '.join("artifacts")',
        '.join("delivery")',
    ),
    "crates/clawd/src/channel_send.rs": (
        "materialize_channel_outbound_media",
        "validate_local_media_file",
        "telegram_image_max_bytes",
        '"sendPhoto"',
        '"sendDocument"',
        'provider_transport_error("telegram_bot", "send_media"',
        'telegram_message_id("send_media"',
    ),
    "crates/clawd/src/channel_send_tests.rs": (
        "telegram_success_response_projects_stable_provider_message_id",
        "telegram_http_rate_limit_keeps_retry_after_without_response_prose",
        "telegram_success_without_message_id_is_a_redacted_invalid_response",
    ),
    "crates/telegramd/src/task_delivery.rs": (
        "request_terminal_delivery",
        "request_terminal_delivery_with_content",
        "ChannelTaskDeliveryContent::MediaOnly",
    ),
}
FORBIDDEN_TOKENS_BY_PATH = {
    "crates/telegramd/src/task_delivery.rs": (
        "validate_local_media_file(",
        ".send_photo(",
        ".send_video(",
        ".send_voice(",
        ".send_audio(",
    ),
}


@dataclass(frozen=True)
class Finding:
    path: str
    kind: str
    token: str


def scan(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    for relative, required_tokens in REQUIRED_TOKENS_BY_PATH.items():
        path = root / relative
        if not path.is_file():
            findings.append(Finding(relative, "contract_file_missing", ""))
            continue
        text = path.read_text(encoding="utf-8")
        for token in required_tokens:
            if token not in text:
                findings.append(Finding(relative, "contract_token_missing", token))
    for relative, forbidden_tokens in FORBIDDEN_TOKENS_BY_PATH.items():
        path = root / relative
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8")
        for token in forbidden_tokens:
            if token in text:
                findings.append(Finding(relative, "legacy_upload_path_present", token))
    return findings


def write_complete_fixture(root: Path) -> None:
    for relative, tokens in REQUIRED_TOKENS_BY_PATH.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(tokens), encoding="utf-8")


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="telegram-media-contract-") as tmp:
        root = Path(tmp)
        write_complete_fixture(root)
        target = root / "crates/clawd/src/channel_send.rs"
        target.write_text("materialize_channel_outbound_media\n", encoding="utf-8")
        findings = scan(root)
        if not any(
            finding.path == "crates/clawd/src/channel_send.rs"
            and finding.kind == "contract_token_missing"
            for finding in findings
        ):
            print(f"SELF_TEST_FAIL findings={findings}")
            return 1

        write_complete_fixture(root)
        daemon_delivery = root / "crates/telegramd/src/task_delivery.rs"
        daemon_delivery.write_text(
            daemon_delivery.read_text(encoding="utf-8")
            + "\nvalidate_local_media_file(\n",
            encoding="utf-8",
        )
        findings = scan(root)
        if not any(
            finding.kind == "legacy_upload_path_present" for finding in findings
        ):
            print(f"SELF_TEST_FAIL findings={findings}")
            return 1
    print("TELEGRAM_MEDIA_DELIVERY_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = scan(REPO_ROOT)
    if findings:
        print("TELEGRAM_MEDIA_DELIVERY_CONTRACT_CHECK failed")
        for finding in findings:
            print(f"- {finding.path}:{finding.kind}:{finding.token}")
        return 1
    print(
        "TELEGRAM_MEDIA_DELIVERY_CONTRACT_CHECK ok "
        f"protected_files={len(REQUIRED_TOKENS_BY_PATH)} findings=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
