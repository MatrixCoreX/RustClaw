#!/usr/bin/env python3
"""Compatibility entrypoint for the historical route-delta release gate.

Use ``check_agent_loop_trace_release_gate.py`` for current agent-loop trace
release/deletion checks. This module keeps the old command importable.
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
    path = Path(__file__).with_name("agent_loop_trace_release_gate_impl.py")
    spec = importlib.util.spec_from_file_location("agent_loop_trace_release_gate_impl", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load agent-loop trace release gate implementation: {path}")
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
