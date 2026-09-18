"""Recover bounded API execution streams from the local trusted runner log.

The runtime's stream hashes bind these records to the persisted result. A later
finalization log may add lifecycle events, so its whole-trace hash can differ.
No user-provided log path or model response is used as execution evidence.
"""
from copy import deepcopy
import json
import os
from pathlib import Path
import re


def trace_hash(value):
    raw = json.dumps(value, ensure_ascii=False, sort_keys=True,
                     separators=(",", ":"), allow_nan=False).encode("utf-8")
    digest = 0xcbf29ce484222325
    for byte in raw:
        digest = ((digest ^ byte) * 0x100000001b3) & ((1 << 64) - 1)
    return f"fnv64:{digest:016x}"


def restore_execution_streams(final, final_path, task_id):
    trace = (final.get("data", {}).get("result_json") or {}).get("task_journal", {}).get("trace", {})
    meta = trace.get("trace_storage") or {}
    hashes = meta.get("evidence_streams")
    if not meta.get("truncated") or not isinstance(hashes, dict):
        return final, None
    names = ("step_results", "capability_results")
    detail = {"type": "execution_stream_integrity", "ok": False,
              "reason": "matching_full_trace_unavailable"}
    if any(not isinstance(hashes.get(name), str) for name in names):
        return final, detail
    # Runner layout: run/cases/date/case/final.json. Never search arbitrary
    # workspace files or trust paths inside the model's result.
    parents = Path(final_path).parents
    if len(parents) <= 3:
        return final, detail
    root = parents[3]
    logs = [root / "server.log", *sorted(root.glob("clawd_full_nl_*.log"))]
    # Live deployments do not use the isolated runner's directory layout.
    # Only the test operator may name this source; both stream digests must match.
    configured_log = os.environ.get("NL_RUNTIME_TRACE_LOG")
    if configured_log:
        logs.append(Path(configured_log))
    for log in logs:
        if not log.is_file() or log.is_symlink():
            continue
        restored, evidence = restore_from_log(final, task_id, log, hashes, names)
        if evidence is not None:
            return restored, evidence
    return final, detail


def restore_from_log(final, task_id, log, hashes, names):
    prefix = re.compile(r"^\d{4}-\d\d-\d\dT\S+\s+INFO\s+(?:task_call:\s+)?"
                        r"task_journal_summary task_id=" + re.escape(task_id)
                        + r" kind=ask phase=(?:finalize|failure)(?: |$)")
    with log.open("rb") as handle:
        while True:
            offset = handle.tell()
            raw = handle.readline()
            if not raw or not raw.endswith(b"\n"):
                break
            line = raw.decode("utf-8", errors="replace")
            if not prefix.match(line):
                continue
            start = line.find(" {", prefix.match(line).end() - 1)
            if start < 0:
                continue
            try:
                record, _ = json.JSONDecoder().raw_decode(line[start + 1:])
                streams = record.get("trace", {})
                if record.get("task_id") != task_id or any(
                    not isinstance(streams.get(name), list)
                    or trace_hash(streams[name]) != hashes[name] for name in names
                ):
                    continue
            except (ValueError, TypeError, AttributeError):
                continue
            restored = deepcopy(final)
            destination = restored["data"]["result_json"]["task_journal"]["trace"]
            for name in names:
                destination[name] = streams[name]
            return restored, {"type": "execution_stream_integrity", "ok": True,
                              "source": str(log), "offset": offset, "hashes": hashes,
                              "counts": {name: len(streams[name]) for name in names}}
    return final, None
