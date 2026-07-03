#!/usr/bin/env python3
"""Release/deletion gate for agent-loop trace replay evidence.

This is the preferred name for the current gate. The older
`check_route_delta_release_gate.py` script remains as a compatibility entrypoint
around the same historical route-delta fields.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path
from types import ModuleType


def load_impl() -> ModuleType:
    path = Path(__file__).with_name("agent_loop_trace_release_gate_impl.py")
    spec = importlib.util.spec_from_file_location("agent_loop_trace_release_gate_impl", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load agent-loop trace release gate implementation: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> int:
    return int(load_impl().main())


if __name__ == "__main__":
    raise SystemExit(main())
