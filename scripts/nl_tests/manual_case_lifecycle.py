"""Map runtime lifecycle evidence to acceptance-runner polling states."""
import json
from pathlib import Path
import sys


def terminal_provider_unavailable(value):
    """Only current terminal machine evidence can classify a provider failure."""
    data = value.get("data") or {}
    if data.get("status") not in {"failed", "timeout"}:
        return False
    result = data.get("result_json") or {}
    summary = ((result.get("task_journal") or {}).get("summary") or {})
    attribution = summary.get("final_failure_attribution") or result.get("failure_attribution")
    if attribution and attribution not in {"provider_gap", "provider_error", "llm_provider_gap"}:
        return False
    lifecycle = data.get("lifecycle") or result.get("task_lifecycle") or {}
    frames = [data, result, summary]
    for parent in (data, result, summary, lifecycle):
        if not isinstance(parent, dict):
            continue
        for name in ("provider_status", "failure", "error"):
            frame = parent.get(name)
            if isinstance(frame, dict):
                frames.append(frame)
    for frame in frames:
        if frame.get("external_provider_blocked") is True:
            return True
        for name in ("error_code", "provider_blocker_status_code", "status_code"):
            code = frame.get(name)
            if isinstance(code, str) and (code == "rate_limited" or code.startswith(("provider_", "llm_provider_"))):
                return True
    return False


def poll_status(data):
    status = str(data.get("status") or "")
    lifecycle = data.get("lifecycle") or {}
    if not isinstance(lifecycle, dict) or status != "running":
        return status
    state = lifecycle.get("state")
    if state == "needs_user":
        return "needs_user"
    provider = lifecycle.get("provider_status") or {}
    control = lifecycle.get("control_request") or {}
    if (state == "waiting" and isinstance(control, dict)
            and control.get("kind") == "resume" and control.get("status") == "pending"):
        return status
    if (state == "waiting" and lifecycle.get("provider_blocker_active") is True
            and isinstance(provider, dict) and provider.get("external_provider_blocked") is True):
        return "provider_wait"
    return status


if __name__ == "__main__":
    value = json.loads(Path(sys.argv[1]).read_text())
    if len(sys.argv) == 3 and sys.argv[2] == "--terminal-provider":
        raise SystemExit(0 if terminal_provider_unavailable(value) else 1)
    print(poll_status(value.get("data") or {}))
