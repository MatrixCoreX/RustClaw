#!/usr/bin/env python3
"""Clean RustClaw development artifacts that are safe to regenerate.

Default mode is a dry run. Pass --apply to delete.
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def byte_size(path: Path) -> int:
    if not path.exists():
        return 0
    if path.is_file() or path.is_symlink():
        try:
            return path.lstat().st_size
        except OSError:
            return 0
    total = 0
    for item in path.rglob("*"):
        try:
            total += item.lstat().st_size
        except OSError:
            continue
    return total


def human_size(size: int) -> str:
    value = float(size)
    for unit in ("B", "K", "M", "G", "T"):
        if value < 1024 or unit == "T":
            if unit == "B":
                return f"{int(value)}{unit}"
            return f"{value:.1f}{unit}"
        value /= 1024
    return f"{size}B"


def contents(path: Path) -> list[Path]:
    if not path.is_dir():
        return []
    return sorted(path.iterdir())


def historical_model_io_logs() -> list[Path]:
    logs = ROOT / "logs"
    if not logs.is_dir():
        return []
    return sorted(logs.glob("model_io.log.*"))


def node_modules_dirs() -> list[Path]:
    candidates = [
        ROOT / "node_modules",
        ROOT / "UI" / "node_modules",
        ROOT / "services" / "wa-web-bridge" / "node_modules",
        ROOT / "pi_app" / "node_modules",
    ]
    return [path for path in candidates if path.exists()]


def default_targets() -> list[Path]:
    targets = [ROOT / "target"]
    targets.extend(contents(ROOT / "scripts" / "nl_suite_logs"))
    targets.extend(contents(ROOT / "tmp"))
    targets.extend(contents(ROOT / "logs" / "nl_tests"))
    targets.extend(historical_model_io_logs())
    return targets


def delete_path(path: Path) -> None:
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    else:
        path.unlink(missing_ok=True)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="delete selected artifacts")
    parser.add_argument(
        "--include-node-modules",
        action="store_true",
        help="also delete node_modules directories; reinstall with npm install when needed",
    )
    parser.add_argument(
        "--include-current-logs",
        action="store_true",
        help="also delete current logs/*.log files, not only rotated model_io logs",
    )
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    targets = default_targets()
    if args.include_node_modules:
        targets.extend(node_modules_dirs())
    if args.include_current_logs:
        targets.extend(sorted((ROOT / "logs").glob("*.log")))

    existing: list[Path] = []
    seen: set[Path] = set()
    for target in targets:
        resolved = target.resolve()
        if resolved in seen or not target.exists():
            continue
        seen.add(resolved)
        existing.append(target)

    total = 0
    for target in existing:
        size = byte_size(target)
        total += size
        rel = target.relative_to(ROOT)
        print(f"{human_size(size):>8}  {rel}")
    mode = "APPLY" if args.apply else "DRY_RUN"
    print(f"DEV_ARTIFACT_CLEAN {mode} targets={len(existing)} total={human_size(total)}")

    if not args.apply:
        return 0
    for target in existing:
        delete_path(target)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
