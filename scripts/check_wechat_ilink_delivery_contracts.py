#!/usr/bin/env python3
"""Protect WeChat iLink context, lifecycle, media, and account-scope contracts."""
from __future__ import annotations

import argparse
import tempfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
REQUIRED_TOKENS_BY_PATH = {
    "crates/wechat-ilink/src/contract.rs": (
        "https://github.com/Tencent/openclaw-weixin/blob/main/src/api/types.ts",
        "UPLOAD_MEDIA_TYPE_IMAGE: i64 = 1",
        "UPLOAD_MEDIA_TYPE_VIDEO: i64 = 2",
        "UPLOAD_MEDIA_TYPE_FILE: i64 = 3",
        "UPLOAD_MEDIA_TYPE_VOICE: i64 = 4",
        "MESSAGE_STATE_NEW: i64 = 0",
        "MESSAGE_STATE_GENERATING: i64 = 1",
        "MESSAGE_STATE_FINISH: i64 = 2",
        "MESSAGE_ITEM_TEXT: i64 = 1",
        "MESSAGE_ITEM_IMAGE: i64 = 2",
        "MESSAGE_ITEM_VOICE: i64 = 3",
        "MESSAGE_ITEM_FILE: i64 = 4",
        "MESSAGE_ITEM_VIDEO: i64 = 5",
        "TYPING_STATUS_TYPING: i64 = 1",
        "TYPING_STATUS_CANCEL: i64 = 2",
        "pub struct WechatConversationScope",
        '"account_id": self.account_id',
        '"channel": self.channel',
        '"peer_id": self.peer_id',
        "pub fn generating(",
        "pub fn generating_with_item(",
        "pub fn finish(",
        'required_wire_value("context_token", context_token)',
    ),
    "crates/wechat-ilink/src/cdn.rs": (
        "pub upload_full_url: Option<String>",
        "let upload_full_url = up",
        "UPLOAD_MEDIA_TYPE_IMAGE",
        "UPLOAD_MEDIA_TYPE_VIDEO",
        "UPLOAD_MEDIA_TYPE_FILE",
        "WechatMessageItem::image",
        "WechatMessageItem::video",
        "WechatMessageItem::file",
    ),
    "crates/wechatd/src/task_flow.rs": (
        "pub(super) async fn pin_inbound_task_context",
        "normalized_context_token(inbound_context_token)?",
        "borrow a later or older token from the cache",
        "WechatConversationScope::wechat_ilink",
        "pub(super) struct WechatTypingHeartbeat",
        "pub(super) async fn finish(&mut self)",
        "TYPING_STATUS_TYPING",
        "TYPING_STATUS_CANCEL",
        "finish_typing_heartbeat(&mut typing_heartbeat).await",
        "TaskStatus::Succeeded",
        "TaskStatus::Failed",
        "TaskStatus::Canceled",
        "TaskStatus::Timeout",
        "send_generating_message_state",
        "request_unified_terminal_delivery",
        "channel_delivery_client::request_task_delivery",
    ),
    "crates/wechatd/src/incoming.rs": (
        "pin_inbound_task_context",
        "msg.context_token.as_deref()",
        "start_typing_heartbeat_for_peer(&state, &task_context)",
        "spawn_inbound_attachment_flow(",
    ),
    "crates/wechatd/src/config_cache.rs": (
        "scope: &WechatConversationScope",
        "let cache_key = scope.storage_key()",
    ),
    "crates/wechatd/src/binding.rs": (
        "scope: &WechatConversationScope",
        ".contains(&scope.storage_key())",
        "let key = scope.storage_key()",
        "&scope.storage_key()",
        "New writes always use the scoped key",
    ),
    "crates/clawd/src/worker/channels.rs": (
        'pointer("/channel_ingress/reply_target/external_id")',
        'pointer("/channel_ingress/context_token")',
        "wechat_delivery_uses_raw_reply_target_not_scoped_conversation_id",
    ),
    "crates/clawd/src/channel_send.rs": (
        "pub(crate) async fn send_wechat_text_message",
        "extract_wechat_outbound_media",
        "send_weixin_image_from_file",
        "send_weixin_video_from_file",
        "send_weixin_file_from_file",
        "Some(context_token)",
    ),
    "crates/wechat-ilink/src/contract_tests.rs": (
        "conversation_scope_isolated_by_account_channel_and_peer",
        "official_item_types_keep_exact_fields",
        "generating_and_finish_requests_share_run_and_context",
        "send_request_rejects_missing_context_token",
    ),
    "crates/wechat-ilink/src/cdn_tests.rs": (
        "provider_upload_full_url_takes_precedence_over_legacy_url_construction",
        "video_and_file_use_distinct_official_upload_and_item_types",
    ),
    "crates/wechatd/src/main_tests.rs": (
        "task_terminal_mapping_covers_success_failure_cancel_and_timeout",
        "context_token_cache_key_is_account_channel_peer_scoped",
        "typing_heartbeat_finish_waits_for_exactly_one_cancel",
    ),
    "crates/claw-core/src/channel_capabilities.rs": (
        "https://github.com/Tencent/openclaw-weixin#backend-api-protocol",
        "ChannelCapabilitySourceKind::LocalSafetyPolicy",
    ),
}

FORBIDDEN_TOKENS_BY_PATH = {
    "crates/wechatd/src/task_flow.rs": (
        "resolve_delivery_context_token",
        'json!({"msg":',
    ),
    "crates/wechatd/src/incoming.rs": ("resolve_delivery_context_token",),
    "crates/wechat-ilink/src/cdn.rs": (
        '"message_state": 2',
        '"type": 2',
        '"type": 4',
        '"type": 5',
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
    with tempfile.TemporaryDirectory(prefix="wechat-ilink-delivery-") as tmp:
        root = Path(tmp)
        write_complete_fixture(root)
        if scan(root):
            print(f"SELF_TEST_FAIL complete_fixture findings={scan(root)}")
            return 1

        task_flow = root / "crates/wechatd/src/task_flow.rs"
        task_flow.write_text("TaskStatus::Succeeded\n", encoding="utf-8")
        findings = scan(root)
        if not any(finding.kind == "contract_token_missing" for finding in findings):
            print(f"SELF_TEST_FAIL missing_contract findings={findings}")
            return 1

        write_complete_fixture(root)
        incoming = root / "crates/wechatd/src/incoming.rs"
        incoming.write_text(
            incoming.read_text(encoding="utf-8")
            + "\nresolve_delivery_context_token\n",
            encoding="utf-8",
        )
        findings = scan(root)
        if not any(finding.kind == "legacy_contract_present" for finding in findings):
            print(f"SELF_TEST_FAIL legacy_contract findings={findings}")
            return 1

    print("WECHAT_ILINK_DELIVERY_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = scan(REPO_ROOT)
    if findings:
        print("WECHAT_ILINK_DELIVERY_CONTRACT_CHECK failed")
        for finding in findings:
            print(f"- {finding.path}:{finding.kind}:{finding.token}")
        return 1
    print(
        "WECHAT_ILINK_DELIVERY_CONTRACT_CHECK ok "
        f"protected_files={len(REQUIRED_TOKENS_BY_PATH)} findings=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
