#!/usr/bin/env python3
"""Agent-loop trace replay summary entrypoint.

This is the preferred name for summarizing historical route-delta attribution
and current agent-loop trace evidence. The older
`summarize_agent_decides_route_delta.py` module remains as the compatibility
implementation so existing logs and scripts keep working.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path
from types import ModuleType


def load_compat_summarizer() -> ModuleType:
    path = Path(__file__).with_name("summarize_agent_decides_route_delta.py")
    spec = importlib.util.spec_from_file_location("agent_loop_trace_replay_compat", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load compatibility summarizer: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> int:
    return int(load_compat_summarizer().main())


if __name__ == "__main__":
    raise SystemExit(main())
