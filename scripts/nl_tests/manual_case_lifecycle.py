"""Map runtime lifecycle evidence to acceptance-runner polling states."""
import json
from pathlib import Path
import sys


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
    print(poll_status(json.loads(Path(sys.argv[1]).read_text()).get("data") or {}))
