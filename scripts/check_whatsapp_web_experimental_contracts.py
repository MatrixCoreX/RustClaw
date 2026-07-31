#!/usr/bin/env python3
"""Protect WhatsApp Web experimental-adapter and local-policy boundaries."""
from __future__ import annotations

import argparse
import tempfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
REQUIRED_TOKENS_BY_PATH = {
    "configs/channels/whatsapp-web.toml": (
        "allow_proactive_send = false",
        "max_outbound_image_bytes",
        "max_outbound_video_bytes",
        "max_outbound_audio_bytes",
        "max_outbound_file_bytes",
        "本地内存/上传安全阈值",
    ),
    "docker/config/channels/whatsapp-web.toml": (
        "allow_proactive_send = false",
        "max_outbound_image_bytes",
        "max_outbound_video_bytes",
        "max_outbound_audio_bytes",
        "max_outbound_file_bytes",
        "本地内存/上传安全阈值",
    ),
    "crates/claw-core/src/channel_capabilities.rs": (
        "ChannelAdapterKind::WhatsappWeb",
        "ChannelCapabilitySourceKind::ExperimentalInference",
        "WHATSAPP_WEB_EVIDENCE",
        "ChannelCapabilitySourceKind::LocalSafetyPolicy",
        "LOCAL_MEDIA_POLICY",
    ),
    "crates/clawd/src/channel_send.rs": (
        "whatsapp_web_allow_proactive_send",
        "ChannelDeliverySource::ScheduledTask | ChannelDeliverySource::ProactiveNotice",
        'Some("proactive_send_disabled")',
        '"delivery_source": delivery_source',
        "It is not an official WhatsApp Web limit",
    ),
    "services/wa-web-bridge/index.js": (
        'const WA_WEB_ADAPTER_MODE = "experimental_unofficial"',
        'const WA_WEB_TRANSPORT = "baileys"',
        "official_bot_api: false",
        "function loginStatusSnapshot()",
        "function deliverySourceAllowed(source, allowProactiveSend)",
        'adapterError("proactive_send_disabled"',
        "local_safety_limits",
        "last_error_code",
        "last_diagnostic_id",
    ),
    "services/wa-web-bridge/test.js": (
        'deliverySourceAllowed("scheduled_task", false), false',
        'deliverySourceAllowed("unknown", false), false',
        'adapterStatus.adapter_mode, "experimental_unofficial"',
        "adapterStatus.official_bot_api, false",
        "adapterStatus.last_error, undefined",
        'updateLoginState("reconnecting"',
        'reconnectingStatus.last_error_code, "connection_closed"',
    ),
    "crates/clawd/src/http/ui_routes/messaging_login.rs": (
        '"whatsapp_web.login_status_unavailable"',
        '"whatsapp_web.login_status_invalid"',
        '"whatsapp_web.logout_failed"',
    ),
    "UI/src/hooks/useWhatsappWebRuntime.ts": (
        "function whatsappWebRequestError",
        'case "whatsapp_web.login_status_invalid"',
        'case "whatsapp_web.logout_failed"',
    ),
    "UI/src/components/CommunicationSetupPage.tsx": (
        't("实验性连接", "Experimental")',
        "不是 Meta 官方 Bot API",
        "不会写入 Agent 对话历史或记忆",
        "主动发送默认关闭",
        "本地保护上限（不是 WhatsApp 官方上限）",
        "last_error_code",
        "last_diagnostic_id",
    ),
}

FORBIDDEN_TOKENS_BY_PATH = {
    "configs/channels/whatsapp-web.toml": ("allow_proactive_send = true",),
    "docker/config/channels/whatsapp-web.toml": ("allow_proactive_send = true",),
    "services/wa-web-bridge/index.js": (
        "last_error: waLoginState.lastError",
        'return res.status(500).json({ ok: false, error: String(err',
    ),
    "UI/src/components/CommunicationSetupPage.tsx": (
        "whatsappWebLoginStatus.last_error",
    ),
    "crates/clawd/src/http/ui_routes/messaging_login.rs": (
        "bridge login status failed: status=",
        "bridge logout failed: status=",
    ),
    "crates/clawd/src/channel_send.rs": (
        "Max characters per WhatsApp text message (conservative; platform limit",
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
            Finding(relative, "unsafe_contract_present", token)
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
    with tempfile.TemporaryDirectory(prefix="whatsapp-web-experimental-") as tmp:
        root = Path(tmp)
        write_complete_fixture(root)
        if scan(root):
            print(f"SELF_TEST_FAIL complete_fixture findings={scan(root)}")
            return 1

        bridge = root / "services/wa-web-bridge/index.js"
        bridge.write_text("function loginStatusSnapshot() {}\n", encoding="utf-8")
        findings = scan(root)
        if not any(finding.kind == "contract_token_missing" for finding in findings):
            print(f"SELF_TEST_FAIL missing_contract findings={findings}")
            return 1

        write_complete_fixture(root)
        ui = root / "UI/src/components/CommunicationSetupPage.tsx"
        ui.write_text(
            ui.read_text(encoding="utf-8") + "\nwhatsappWebLoginStatus.last_error\n",
            encoding="utf-8",
        )
        findings = scan(root)
        if not any(finding.kind == "unsafe_contract_present" for finding in findings):
            print(f"SELF_TEST_FAIL unsafe_contract findings={findings}")
            return 1

    print("WHATSAPP_WEB_EXPERIMENTAL_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = scan(REPO_ROOT)
    if findings:
        print("WHATSAPP_WEB_EXPERIMENTAL_CONTRACT_CHECK failed")
        for finding in findings:
            print(f"- {finding.path}:{finding.kind}:{finding.token}")
        return 1
    print(
        "WHATSAPP_WEB_EXPERIMENTAL_CONTRACT_CHECK ok "
        f"protected_files={len(REQUIRED_TOKENS_BY_PATH)} findings=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
