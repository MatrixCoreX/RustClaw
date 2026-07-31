#!/usr/bin/env python3
"""Protect Telegram polling/webhook exclusivity and webhook secret boundaries."""
from __future__ import annotations

import argparse
import tempfile
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
REQUIRED_TOKENS_BY_PATH = {
    "crates/claw-core/src/config.rs": (
        "pub update_mode: String",
        "pub webhook_listen: String",
        "pub webhook_public_url: String",
        "pub webhook_secret_env: String",
    ),
    "configs/channels/telegram.toml": (
        'update_mode = "polling"',
        'webhook_listen = "127.0.0.1:8090"',
        'webhook_public_url = ""',
        'webhook_secret_env = "TELEGRAM_WEBHOOK_SECRET"',
    ),
    "crates/telegramd/src/telegram_update_transport.rs": (
        '"polling" => Ok(TelegramUpdateTransport::Polling)',
        '"webhook" => resolve_telegram_webhook_runtime(',
        "telegram_update_mode_invalid",
        "listen.ip().is_loopback()",
        'public_url.scheme() != "https"',
        "port_or_known_default()",
        "env_value(secret_env)",
        "telegram_webhook_secret_environment_missing",
        "telegram_webhook_secret_invalid",
    ),
    "crates/telegramd/src/main.rs": (
        "resolve_telegram_update_transport(",
        "bot.delete_webhook()",
        "polling_default(bot)",
        "webhooks::Options::new(",
        ".secret_token(webhook.secret_token)",
        "webhooks::axum(bot, options)",
    ),
    "crates/telegramd/src/telegram_update_transport_tests.rs": (
        "polling_is_the_only_default_transport_and_never_reads_webhook_secret",
        "webhook_requires_https_loopback_listener_and_environment_secret",
        "webhook_fails_closed_for_missing_or_invalid_secret",
        "webhook_rejects_external_listener_insecure_url_and_ambiguous_mode",
    ),
}
FORBIDDEN_TOKENS_BY_PATH = {
    "configs/channels/telegram.toml": (
        "webhook_secret =",
        "webhook_secret_token =",
    ),
    "crates/telegramd/src/main.rs": (
        'std::env::var("TELEGRAM_WEBHOOK_SECRET")',
        ".dispatch().await",
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
        for token in tokens:
            if token not in text:
                findings.append(Finding(relative, "contract_token_missing", token))
    for relative, tokens in FORBIDDEN_TOKENS_BY_PATH.items():
        path = root / relative
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8")
        for token in tokens:
            if token in text:
                findings.append(Finding(relative, "unsafe_transport_contract", token))
    return findings


def write_complete_fixture(root: Path) -> None:
    for relative, tokens in REQUIRED_TOKENS_BY_PATH.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(tokens), encoding="utf-8")


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="telegram-update-transport-") as tmp:
        root = Path(tmp)
        write_complete_fixture(root)
        transport = root / "crates/telegramd/src/telegram_update_transport.rs"
        transport.write_text("telegram_update_mode_invalid\n", encoding="utf-8")
        findings = scan(root)
        if not any(finding.kind == "contract_token_missing" for finding in findings):
            print(f"SELF_TEST_FAIL missing_contract findings={findings}")
            return 1

        write_complete_fixture(root)
        config = root / "configs/channels/telegram.toml"
        config.write_text(
            config.read_text(encoding="utf-8") + "\nwebhook_secret = \"fixture\"\n",
            encoding="utf-8",
        )
        findings = scan(root)
        if not any(finding.kind == "unsafe_transport_contract" for finding in findings):
            print(f"SELF_TEST_FAIL secret_literal findings={findings}")
            return 1
    print("TELEGRAM_UPDATE_TRANSPORT_CONTRACT_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings = scan(REPO_ROOT)
    if findings:
        print("TELEGRAM_UPDATE_TRANSPORT_CONTRACT_CHECK failed")
        for finding in findings:
            print(f"- {finding.path}:{finding.kind}:{finding.token}")
        return 1
    print(
        "TELEGRAM_UPDATE_TRANSPORT_CONTRACT_CHECK ok "
        f"protected_files={len(REQUIRED_TOKENS_BY_PATH)} findings=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
