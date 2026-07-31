#!/usr/bin/env python3
"""Protect the shared Feishu/Lark contract and their isolated runtime scopes."""
from __future__ import annotations

import argparse
import tempfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
REQUIRED_TOKENS_BY_PATH = {
    "crates/claw-core/src/channel_open_platform.rs": (
        "OPEN_PLATFORM_TEXT_CONTENT_MAX_BYTES: usize = 150 * 1024",
        "OPEN_PLATFORM_STRUCTURED_CONTENT_MAX_BYTES: usize = 30 * 1024",
        "OPEN_PLATFORM_TARGET_MIN_INTERVAL_MILLIS: u64 = 200",
        'rate_bucket_namespace: "feishu_open_platform_target"',
        'rate_bucket_namespace: "lark_open_platform_target"',
        'receipt_namespace: "feishu_open_platform_delivery"',
        'receipt_namespace: "lark_open_platform_delivery"',
        "chunk_open_platform_text",
        "OpenPlatformTargetRateLimiter",
        "OpenPlatformTokenCache",
        '"230020"',
        '"99991400"',
        '"99991403"',
        "QuotaExhausted",
        "plan_open_platform_media",
        'form_file_type: "mp4"',
        'form_file_type: "opus"',
        "open_platform_message_id",
        "open.feishu.cn/document",
        "open.larksuite.com/document",
    ),
    "crates/feishud/src/main.rs": (
        "open_platform_contract(OpenPlatformRegion::Feishu).source_adapter",
        "chunk_open_platform_text",
        "validate_open_platform_content",
        "preflight_open_platform_media",
        "process_open_platform_rate_limiter",
        "plan_open_platform_media",
        "OpenPlatformTokenCache",
        "open_platform_message_id",
        ".base_url(section.api_base_url.trim_end_matches('/'))",
        "image upload fallback diagnostic_id={}",
    ),
    "crates/larkd/src/main.rs": (
        "open_platform_contract(OpenPlatformRegion::Lark).source_adapter",
        "chunk_open_platform_text",
        "validate_open_platform_content",
        "preflight_open_platform_media",
        "process_open_platform_rate_limiter",
        "plan_open_platform_media",
        "OpenPlatformTokenCache",
        "open_platform_message_id",
        ".base_url(section.api_base_url.trim_end_matches('/'))",
        "image upload fallback diagnostic_id={}",
    ),
    "crates/feishud/src/main_tests.rs": (
        "feishu_long_connection_uses_its_configured_api_base",
        "feishu_provider_code_overrides_legacy_http_status_classification",
        "feishu_delivery_errors_resolve_to_localized_copy_without_machine_payloads",
        "feishu_outbound_delivery_sends_text_image_video_and_opus_audio",
        'vec!["text", "image", "media", "audio"]',
    ),
    "crates/larkd/src/main_tests.rs": (
        "lark_long_connection_uses_its_configured_api_base",
        "lark_provider_code_overrides_legacy_http_status_classification",
        "lark_delivery_errors_resolve_to_localized_copy_without_machine_payloads",
        "lark_outbound_delivery_sends_text_image_video_and_opus_audio",
        'vec!["text", "image", "media", "audio"]',
    ),
    "crates/clawd/src/channel_send.rs": (
        "process_open_platform_token_cache",
        "chunk_open_platform_text",
        "preflight_open_platform_media",
        "process_open_platform_rate_limiter",
        "plan_open_platform_media",
        "open_platform_message_id",
        "outcome.provider_message_ids.push(message_id)",
    ),
    "crates/clawd/src/delivery_service.rs": (
        "scoped_open_platform_receipt_key",
        "OpenPlatformRegion::Feishu",
        "OpenPlatformRegion::Lark",
    ),
}

FORBIDDEN_TOKENS_BY_PATH = {
    "crates/feishud/src/main.rs": (
        "chunk_text_utf8",
        "RwLock<Option<TenantToken",
        "falling back to file path=",
    ),
    "crates/larkd/src/main.rs": (
        "chunk_text_utf8",
        "RwLock<Option<TenantToken",
        "falling back to file path=",
    ),
    "crates/clawd/src/channel_send.rs": (
        "chunk_feishu_lark_text_utf8",
        "falling back to file path=",
    ),
}


@dataclass(frozen=True)
class Finding:
    path: str
    kind: str
    token: str


def scan(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    for relative, tokens in REQUIRED_TOKENS_BY_PATH.items():
        path = root / relative
        if not path.is_file():
            findings.append(Finding(relative, "contract_file_missing", ""))
            continue
        text = path.read_text(encoding="utf-8")
        findings.extend(
            Finding(relative, "contract_token_missing", token)
            for token in tokens
            if token not in text
        )
    for relative, tokens in FORBIDDEN_TOKENS_BY_PATH.items():
        path = root / relative
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8")
        findings.extend(
            Finding(relative, "legacy_contract_present", token)
            for token in tokens
            if token in text
        )
    return findings


def write_complete_fixture(root: Path) -> None:
    for relative, tokens in REQUIRED_TOKENS_BY_PATH.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(tokens), encoding="utf-8")


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="feishu-lark-open-platform-") as tmp:
        root = Path(tmp)
        write_complete_fixture(root)
        if scan(root):
            print(f"SELF_TEST_FAIL complete_fixture findings={scan(root)}")
            return 1

        core = root / "crates/claw-core/src/channel_open_platform.rs"
        core.write_text("OpenPlatformTokenCache\n", encoding="utf-8")
        findings = scan(root)
        if not any(finding.kind == "contract_token_missing" for finding in findings):
            print(f"SELF_TEST_FAIL missing_contract findings={findings}")
            return 1

        write_complete_fixture(root)
        lark = root / "crates/larkd/src/main.rs"
        lark.write_text(
            lark.read_text(encoding="utf-8") + "\nchunk_text_utf8\n",
            encoding="utf-8",
        )
        findings = scan(root)
        if not any(finding.kind == "legacy_contract_present" for finding in findings):
            print(f"SELF_TEST_FAIL legacy_contract findings={findings}")
            return 1

    print("FEISHU_LARK_OPEN_PLATFORM_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = scan(REPO_ROOT)
    if findings:
        print("FEISHU_LARK_OPEN_PLATFORM_CONTRACT_CHECK failed")
        for finding in findings:
            print(f"- {finding.path}:{finding.kind}:{finding.token}")
        return 1
    print(
        "FEISHU_LARK_OPEN_PLATFORM_CONTRACT_CHECK ok "
        f"protected_files={len(REQUIRED_TOKENS_BY_PATH)} findings=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
