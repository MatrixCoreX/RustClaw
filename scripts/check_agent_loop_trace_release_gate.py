#!/usr/bin/env python3
"""Release/deletion gate for agent-loop trace replay evidence.

This is the preferred name for the current gate. The older
`check_route_delta_release_gate.py` script remains as a compatibility wrapper
around the same historical route-delta fields.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path
from types import ModuleType


def load_compat_gate() -> ModuleType:
    path = Path(__file__).with_name("check_route_delta_release_gate.py")
    spec = importlib.util.spec_from_file_location("agent_loop_trace_gate_compat", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load compatibility release gate: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> int:
    return int(load_compat_gate().main())


if __name__ == "__main__":
    raise SystemExit(main())
