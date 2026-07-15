#!/usr/bin/env python3
"""Render portable path references for logs and test artifacts."""

from __future__ import annotations

import argparse
import sys
import tempfile
from pathlib import Path, PurePosixPath


def portable_path_ref(
    value: str,
    *,
    root: Path,
    anchor: Path | None = None,
    anchor_name: str = "run_dir",
) -> str:
    raw = (value or "").strip()
    if not raw:
        return "external_path"

    try:
        root = root.resolve()
        resolved_anchor = anchor.resolve() if anchor is not None else None
        candidate = Path(raw).resolve()
    except OSError:
        return "external_path"

    if resolved_anchor is not None:
        try:
            rel = candidate.relative_to(resolved_anchor)
            return anchor_name if str(rel) == "." else f"{anchor_name}/{rel.as_posix()}"
        except ValueError:
            pass

    try:
        return candidate.relative_to(root).as_posix()
    except ValueError:
        pass

    if not raw.startswith("/") and "\\" not in raw:
        rel = PurePosixPath(raw)
        if rel.parts and all(part not in {"", ".", ".."} for part in rel.parts):
            return rel.as_posix()

    return "external_path"


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / "repo"
        run_dir = root / "scripts" / "nl_suite_logs" / "manual" / "20260715_000000"
        run_dir.mkdir(parents=True)
        (root / "configs").mkdir()
        checks = [
            (portable_path_ref(str(run_dir), root=root, anchor=run_dir), "run_dir"),
            (portable_path_ref(str(run_dir / "run.log"), root=root, anchor=run_dir), "run_dir/run.log"),
            (portable_path_ref(str(root / "configs" / "config.toml"), root=root, anchor=run_dir), "configs/config.toml"),
            (portable_path_ref("relative/report.json", root=root, anchor=run_dir), "relative/report.json"),
            (portable_path_ref(str(Path(tmp) / "outside.log"), root=root, anchor=run_dir), "external_path"),
            (portable_path_ref("", root=root, anchor=run_dir), "external_path"),
        ]
        bad = [(actual, expected) for actual, expected in checks if actual != expected]
        if bad:
            print({"bad": bad}, file=sys.stderr)
            return 1
    print("PATH_REF_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("value", nargs="?")
    parser.add_argument("--root", default=".")
    parser.add_argument("--anchor")
    parser.add_argument("--anchor-name", default="run_dir")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test()
    if args.value is None:
        parser.error("value is required unless --self-test is used")

    print(
        portable_path_ref(
            args.value,
            root=Path(args.root),
            anchor=Path(args.anchor) if args.anchor else None,
            anchor_name=args.anchor_name,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
