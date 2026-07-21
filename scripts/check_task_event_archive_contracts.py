#!/usr/bin/env python3
"""Guard durable task-event archive, replay, snapshot, and retention contracts."""

from __future__ import annotations

import argparse
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS: dict[str, tuple[str, ...]] = {
    "migrations/001_init.sql": (
        "CREATE TABLE IF NOT EXISTS task_event_archive",
        "previous_event_hash",
        "payload_schema_version",
        "CREATE TABLE IF NOT EXISTS task_event_snapshots",
        "source_event_count",
        "snapshot_hash",
    ),
    "crates/clawd/src/task_event_transport.rs": (
        "payload_schema_version",
        "previous_event_hash",
        "backfill_hot_suffix",
        "task_event_archive::insert_event",
        'replay_source: "archive"',
        "archive_recovered",
    ),
    "crates/clawd/src/task_event_archive.rs": (
        "ARCHIVE_SNAPSHOT_INTERVAL",
        "persist_snapshot",
        "source_event_range",
        "task_event_redaction_v1",
        "delete_orphaned_records",
    ),
    "crates/clawd/src/http/task_events.rs": (
        "archive_replay_control_event",
        "has_unread_persisted_events",
        "task_event_archive_replay_failed",
        '"replay_mode": "archive_recovery"',
    ),
    "crates/clawcli/src/replay.rs": (
        "read_task_event_snapshot",
        "replay_bundle_json_with_archived_events",
        '"task_event_archive"',
        '"raw": redact_value(raw)',
    ),
    "crates/clawd/src/worker/runtime_support/background_workers.rs": (
        "task_event_archive::delete_orphaned_records",
    ),
    "crates/clawd/src/task_event_transport_tests.rs": (
        "bounded_hot_retention_recovers_older_cursor_from_archive",
        "archive_records_hash_chain_payload_version_and_terminal_snapshot",
        "irrecoverable_archive_gap_returns_structured_expired_cursor_state",
    ),
    "crates/clawcli/src/replay_tests.rs": (
        "replay_bundle_prefers_versioned_archived_events",
    ),
}

FORBIDDEN_TOKENS: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/task_event_archive.rs": (
        "user_text",
        "Regex::",
        "raw_llm_request",
        "raw_llm_response",
    ),
    "crates/clawd/src/task_event_transport.rs": (
        '"replay_mode": "retained_suffix"',
    ),
}


def scan_texts(texts: dict[str, str | None]) -> list[str]:
    findings: list[str] = []
    for rel_path, tokens in REQUIRED_TOKENS.items():
        text = texts.get(rel_path)
        if text is None:
            findings.append(f"missing_or_unreadable:{rel_path}")
            continue
        for token in tokens:
            if token not in text:
                findings.append(f"missing_token:{rel_path}:{token}")
    for rel_path, tokens in FORBIDDEN_TOKENS.items():
        text = texts.get(rel_path)
        if text is None:
            findings.append(f"missing_or_unreadable:{rel_path}")
            continue
        for token in tokens:
            if token in text:
                findings.append(f"forbidden_token:{rel_path}:{token}")
    return findings


def read_repo_texts() -> dict[str, str | None]:
    paths = set(REQUIRED_TOKENS) | set(FORBIDDEN_TOKENS)
    output: dict[str, str | None] = {}
    for rel_path in paths:
        try:
            output[rel_path] = (ROOT / rel_path).read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeDecodeError):
            output[rel_path] = None
    return output


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens)
        for rel_path, tokens in REQUIRED_TOKENS.items()
    }
    for rel_path in FORBIDDEN_TOKENS:
        texts.setdefault(rel_path, "")
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    assert not scan_texts(good)

    missing = dict(good)
    missing["crates/clawd/src/task_event_archive.rs"] = "persist_snapshot"
    assert any("source_event_range" in item for item in scan_texts(missing))

    regressed = dict(good)
    regressed["crates/clawd/src/task_event_transport.rs"] += (
        '\n"replay_mode": "retained_suffix"'
    )
    assert any("forbidden_token" in item for item in scan_texts(regressed))
    print("TASK_EVENT_ARCHIVE_CONTRACT_SELF_TEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"TASK_EVENT_ARCHIVE_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print("TASK_EVENT_ARCHIVE_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
