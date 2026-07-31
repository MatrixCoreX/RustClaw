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
    "crates/telegramd/src/telegram_media_delivery.rs": (
        "preflight_local_media_file",
        "is_managed_task_delivery_artifact_path",
        "TelegramUploadMethod::Document",
        "telegram.msg.delivery_media_failed_ui_fallback",
        "telegram.msg.delivery_media_failed_retry",
        "deliver_missing_telegram_media_path",
    ),
    "crates/telegramd/src/telegram_formatting.rs": (
        "deliver_missing_telegram_media_path",
        "TelegramMediaKind::Image",
        "TelegramMediaKind::Video",
        "TelegramMediaKind::File",
        "TelegramMediaKind::Voice",
        "TelegramMediaKind::Audio",
    ),
    "crates/telegramd/src/telegram_media_delivery_tests.rs": (
        "image_uses_photo_with_document_fallback_until_photo_limit",
        "oversized_managed_artifact_points_to_ui_without_exposing_path",
        "unmanaged_file_failure_requests_retry_instead_of_claiming_ui_copy",
    ),
}
FORBIDDEN_TOKENS_BY_PATH = {
    "crates/telegramd/src/telegram_formatting.rs": (
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
        target = root / "crates/telegramd/src/telegram_media_delivery.rs"
        target.write_text("preflight_local_media_file\n", encoding="utf-8")
        findings = scan(root)
        if not any(
            finding.path == "crates/telegramd/src/telegram_media_delivery.rs"
            and finding.kind == "contract_token_missing"
            for finding in findings
        ):
            print(f"SELF_TEST_FAIL findings={findings}")
            return 1

        write_complete_fixture(root)
        formatting = root / "crates/telegramd/src/telegram_formatting.rs"
        formatting.write_text(
            formatting.read_text(encoding="utf-8") + "\nvalidate_local_media_file(\n",
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
