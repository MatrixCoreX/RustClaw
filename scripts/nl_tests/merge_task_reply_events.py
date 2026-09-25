#!/usr/bin/env python3
"""Project durable nonterminal reply events into an NL task snapshot.

The task query intentionally keeps a resumable task in ``running`` state. UI
clients receive clarifications and side replies from the task SSE stream, so
the acceptance harness must combine both public interfaces before asserting
on user-visible output.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def parse_sse_data(raw: str) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    for line in raw.splitlines():
        if not line.startswith("data:"):
            continue
        payload = line.removeprefix("data:").strip()
        if not payload:
            continue
        try:
            value = json.loads(payload)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            events.append(value)
    return events


def project_reply(task_snapshot: dict[str, Any], events: list[dict[str, Any]]) -> bool:
    data = task_snapshot.get("data")
    if not isinstance(data, dict):
        return False
    lifecycle = data.get("lifecycle")
    reply_id = lifecycle.get("reply_id") if isinstance(lifecycle, dict) else None

    candidates: list[dict[str, Any]] = []
    for event in events:
        if event.get("event_kind") != "conversation_reply_item":
            continue
        payload = event.get("payload")
        if not isinstance(payload, dict):
            continue
        if reply_id and payload.get("reply_id") != reply_id:
            continue
        if not str(payload.get("text") or "").strip():
            continue
        candidates.append(event)
    if not candidates:
        return False

    selected = max(candidates, key=lambda event: int(event.get("seq") or 0))
    payload = selected["payload"]
    result = data.get("result_json")
    if not isinstance(result, dict):
        result = {}
        data["result_json"] = result
    text = str(payload["text"]).strip()
    if not str(result.get("text") or "").strip():
        result["text"] = text
    messages = result.get("messages")
    if not isinstance(messages, list):
        messages = []
        result["messages"] = messages
    if not any(isinstance(item, dict) and item.get("reply_id") == payload.get("reply_id") for item in messages):
        messages.append(
            {
                "text": text,
                "reply_id": payload.get("reply_id"),
                "relation": payload.get("relation"),
                "lifecycle_stage": payload.get("lifecycle_stage"),
            }
        )
    return True


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--task-json", type=Path, required=True)
    parser.add_argument("--events-file", type=Path, required=True)
    args = parser.parse_args()

    snapshot = json.loads(args.task_json.read_text(encoding="utf-8"))
    events = parse_sse_data(args.events_file.read_text(encoding="utf-8"))
    if project_reply(snapshot, events):
        args.task_json.write_text(
            json.dumps(snapshot, ensure_ascii=False, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
