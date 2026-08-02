#!/usr/bin/env python3
"""Read the WhatsApp Web enable state from its canonical split config."""

from __future__ import annotations

import argparse
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib


def whatsapp_web_enabled(path: Path) -> bool:
    if not path.is_file():
        return False
    with path.open("rb") as handle:
        config = tomllib.load(handle)
    section = config.get("whatsapp_web", {}) if isinstance(config, dict) else {}
    if not isinstance(section, dict):
        raise ValueError("whatsapp_web must be a TOML table")
    enabled = section.get("enabled", False)
    if not isinstance(enabled, bool):
        raise ValueError("whatsapp_web.enabled must be boolean")
    return enabled


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("config", type=Path)
    args = parser.parse_args()
    print("1" if whatsapp_web_enabled(args.config) else "0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
