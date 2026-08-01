#!/usr/bin/env python3
"""Protect WhatsApp Cloud window, template, media, receipt, and webhook contracts."""
from __future__ import annotations

import argparse
import tempfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
REQUIRED_TOKENS_BY_PATH = {
    "crates/claw-core/src/channel_whatsapp_cloud.rs": (
        "https://developers.facebook.com/docs/whatsapp/cloud-api/reference/media",
        "https://developers.facebook.com/docs/whatsapp/cloud-api/reference/messages",
        "https://developers.facebook.com/docs/whatsapp/cloud-api/webhooks/components",
        "WHATSAPP_CUSTOMER_SERVICE_WINDOW_SECONDS",
        "decode_message_ids",
        "pub fn from_config(name: &str, language: &str)",
        "WhatsappAcceptedDeliveryEvent",
        "WhatsappWebhookPayload",
        '"sent" => Some(WhatsappDeliveryEventStatus::Accepted)',
        '"delivered" => Some(WhatsappDeliveryEventStatus::Delivered)',
        '"read" => Some(WhatsappDeliveryEventStatus::Read)',
        '"failed" | "deleted" => Some(WhatsappDeliveryEventStatus::Failed)',
        "provider_error_from_response",
        "131047",
        "131048",
    ),
    "crates/claw-core/src/channel_media_limits.rs": (
        "prepare_whatsapp_cloud_media",
        "whatsapp_video_codecs_are_compatible",
        'stream.codec_name == "h264"',
        'stream.codec_name == "aac"',
        'stream.codec_name == "opus"',
        'Command::new("ffprobe")',
        'Command::new("ffmpeg")',
        "compatible_copy_created",
    ),
    "crates/clawd/src/channel_send.rs": (
        "send_whatsapp_cloud_template_message",
        "whatsapp_out_of_window_template_name",
        '"policy": "deterministic"',
        "decode_message_ids(\"send_text\"",
        "decode_message_ids(\"send_media\"",
        "provider_message_ids",
    ),
    "crates/clawd/src/repo/channel_delivery_receipt.rs": (
        "whatsapp_cloud_conversation_windows",
        "channel_delivery_provider_messages",
        "whatsapp_cloud_pending_provider_statuses",
        "record_whatsapp_cloud_inbound",
        "record_whatsapp_cloud_provider_status",
        "provider_status_is_regression",
        "replay_pending_whatsapp_statuses",
        "provider_error_code",
        "diagnostic_id",
    ),
    "crates/clawd/src/whatsapp_cloud_events.rs": (
        "x-hub-signature-256",
        "x-channel-event-signature-256",
        "record_whatsapp_cloud_provider_status",
        "record_channel_delivery_receipt",
    ),
    "crates/whatsappd/src/main.rs": (
        "with_received_at_ts(now_ts())",
        "forward_delivery_statuses",
        "last_inbound_at_by_user",
        "x-hub-signature-256",
        "request_unified_terminal_delivery",
        "channel_delivery_client::request_task_delivery",
    ),
    "configs/channels/whatsapp-cloud.toml": (
        "out_of_window_template_name",
        "out_of_window_template_language",
        "LLM 无权选择模板",
    ),
}

FORBIDDEN_TOKENS_BY_PATH = {
    "crates/clawd/src/channel_send.rs": (
        '.map(|_| crate::channel_send::ChannelSendOutcome::default())',
    ),
    "crates/whatsappd/src/main.rs": (
        'ChannelProviderError::from_http_response(\n            "whatsapp_cloud"',
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
    with tempfile.TemporaryDirectory(prefix="whatsapp-cloud-delivery-") as tmp:
        root = Path(tmp)
        write_complete_fixture(root)
        if scan(root):
            print(f"SELF_TEST_FAIL complete_fixture findings={scan(root)}")
            return 1

        contract = root / "crates/claw-core/src/channel_whatsapp_cloud.rs"
        contract.write_text("WHATSAPP_CUSTOMER_SERVICE_WINDOW_SECONDS\n", encoding="utf-8")
        findings = scan(root)
        if not any(finding.kind == "contract_token_missing" for finding in findings):
            print(f"SELF_TEST_FAIL missing_contract findings={findings}")
            return 1

        write_complete_fixture(root)
        sender = root / "crates/clawd/src/channel_send.rs"
        sender.write_text(
            sender.read_text(encoding="utf-8")
            + "\n.map(|_| crate::channel_send::ChannelSendOutcome::default())\n",
            encoding="utf-8",
        )
        findings = scan(root)
        if not any(finding.kind == "legacy_contract_present" for finding in findings):
            print(f"SELF_TEST_FAIL legacy_contract findings={findings}")
            return 1

    print("WHATSAPP_CLOUD_DELIVERY_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = scan(REPO_ROOT)
    if findings:
        print("WHATSAPP_CLOUD_DELIVERY_CONTRACT_CHECK failed")
        for finding in findings:
            print(f"- {finding.path}:{finding.kind}:{finding.token}")
        return 1
    print(
        "WHATSAPP_CLOUD_DELIVERY_CONTRACT_CHECK ok "
        f"protected_files={len(REQUIRED_TOKENS_BY_PATH)} findings=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
