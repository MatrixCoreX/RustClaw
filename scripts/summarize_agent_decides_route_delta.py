#!/usr/bin/env python3
"""Compatibility entrypoint for historical route-delta summaries.

Use ``summarize_agent_loop_trace_replay.py`` for current agent-loop trace
review. This module keeps the old file name importable for historical scripts.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path
from types import ModuleType
from typing import Any

_IMPL: ModuleType | None = None


def load_impl() -> ModuleType:
    global _IMPL
    if _IMPL is not None:
        return _IMPL
    path = Path(__file__).with_name("agent_loop_trace_replay_summary_impl.py")
    spec = importlib.util.spec_from_file_location("agent_loop_trace_replay_summary_impl", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load agent-loop trace replay implementation: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    _IMPL = module
    return module


def __getattr__(name: str) -> Any:
    return getattr(load_impl(), name)


def main() -> int:
    return int(load_impl().main())


if __name__ == "__main__":
    raise SystemExit(main())
