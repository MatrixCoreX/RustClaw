#!/usr/bin/env python3
"""
Redact sensitive values in TOML configs and restore them later.

Usage:
  python3 scripts/redact_configs.py redact  --config-dir configs --secret-map configs_secrets.txt
  python3 scripts/redact_configs.py restore --config-dir configs --secret-map configs_secrets.txt
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
import tomllib


PLACEHOLDER_PREFIX = "__RC_SECRET_"
PROJECT_ROOT = Path(__file__).resolve().parent.parent

# Key names that are very likely sensitive.
SENSITIVE_EXACT = {
    "api_key",
    "api_secret",
    "app_secret",
    "bot_token",
    "access_token",
    "verify_token",
    "verification_token",
    "encrypt_key",
    "password",
    "passphrase",
    "private_key",
    "client_secret",
    "user_key",
    "oss_access_key_id",
    "oss_access_key_secret",
}

ASSIGN_RE = re.compile(
    r"(?P<key>[A-Za-z0-9_.-]+)\s*=\s*(?P<literal>\"(?:\\.|[^\"\\])*\"|'(?:\\.|[^'\\])*')"
)


@dataclass
class MappingEntry:
    entry_id: str
    file: str
    line: int
    key: str
    original_literal: str
    placeholder_literal: str
    original_value: str

    def to_json(self) -> str:
        return json.dumps(
            {
                "id": self.entry_id,
                "file": self.file,
                "line": self.line,
                "key": self.key,
                "original_literal": self.original_literal,
                "placeholder_literal": self.placeholder_literal,
                "original_value": self.original_value,
            },
            ensure_ascii=False,
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Redact/restore sensitive values in configs/*.toml"
    )
    parser.add_argument("mode", choices=["redact", "restore"])
    parser.add_argument("--config-dir", default="configs", help="Config directory root")
    parser.add_argument(
        "--secret-map",
        default="configs_secrets.txt",
        help="Text file to store secret mappings",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print intended changes without writing files",
    )
    return parser.parse_args()


def is_sensitive_key(key: str) -> bool:
    k = key.lower().strip()
    if k in SENSITIVE_EXACT:
        return True
    if (
        k.endswith("_key")
        or k.endswith("_token")
        or k.endswith("_secret")
        or k.endswith("_password")
        or k.endswith("_passphrase")
    ):
        return True
    return False


def should_skip_value(value: str) -> bool:
    v = value.strip()
    if not v:
        return True
    if v.startswith(PLACEHOLDER_PREFIX) and v.endswith("__"):
        return True
    if "REPLACE_ME" in v.upper():
        return True
    return False


def parse_toml_string_literal(literal: str) -> str | None:
    try:
        parsed = tomllib.loads(f"v = {literal}\n")
        value = parsed.get("v")
        return value if isinstance(value, str) else None
    except Exception:
        return None


def find_comment_start(line: str) -> int | None:
    in_single = False
    in_double = False
    escaped = False
    for idx, ch in enumerate(line):
        if escaped:
            escaped = False
            continue
        if ch == "\\" and (in_single or in_double):
            escaped = True
            continue
        if ch == "'" and not in_double:
            in_single = not in_single
            continue
        if ch == '"' and not in_single:
            in_double = not in_double
            continue
        if ch == "#" and not in_single and not in_double:
            return idx
    return None


def iter_toml_files(config_dir: Path) -> list[Path]:
    return sorted(p for p in config_dir.rglob("*.toml") if p.is_file())


def redact_file(
    path: Path, rel_path: str, start_index: int
) -> tuple[str, list[MappingEntry], int]:
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines(keepends=True)
    entries: list[MappingEntry] = []
    counter = start_index

    for i, raw_line in enumerate(lines):
        if raw_line.lstrip().startswith("#"):
            continue

        comment_at = find_comment_start(raw_line)
        if comment_at is None:
            code_part, comment_part = raw_line, ""
        else:
            code_part, comment_part = raw_line[:comment_at], raw_line[comment_at:]

        matches = list(ASSIGN_RE.finditer(code_part))
        if not matches:
            continue

        updated = code_part
        for m in reversed(matches):
            key = m.group("key")
            if not is_sensitive_key(key):
                continue
            literal = m.group("literal")
            value = parse_toml_string_literal(literal)
            if value is None or should_skip_value(value):
                continue

            entry_id = f"RC_SECRET_{counter:04d}"
            placeholder_value = f"{PLACEHOLDER_PREFIX}{counter:04d}__"
            quote = literal[0]
            placeholder_literal = f"{quote}{placeholder_value}{quote}"
            updated = (
                updated[: m.start("literal")]
                + placeholder_literal
                + updated[m.end("literal") :]
            )
            entries.append(
                MappingEntry(
                    entry_id=entry_id,
                    file=rel_path,
                    line=i + 1,
                    key=key,
                    original_literal=literal,
                    placeholder_literal=placeholder_literal,
                    original_value=value,
                )
            )
            counter += 1

        if updated != code_part:
            lines[i] = updated + comment_part

    return "".join(lines), entries, counter


def write_mapping(path: Path, entries: list[MappingEntry], dry_run: bool) -> None:
    header = [
        "# RustClaw config secret mapping",
        "# Keep this file private. Anyone with this file can restore secrets.",
        "# Format: one JSON object per line",
        "",
    ]
    body = [e.to_json() for e in entries]
    content = "\n".join(header + body).rstrip() + "\n"
    if dry_run:
        print(f"[dry-run] Would write secret map: {path} ({len(entries)} entries)")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def load_mapping(path: Path) -> list[MappingEntry]:
    if not path.exists():
        raise FileNotFoundError(f"secret map not found: {path}")
    entries: list[MappingEntry] = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        obj = json.loads(line)
        entries.append(
            MappingEntry(
                entry_id=str(obj["id"]),
                file=str(obj["file"]),
                line=int(obj.get("line", 0)),
                key=str(obj.get("key", "")),
                original_literal=str(obj["original_literal"]),
                placeholder_literal=str(obj["placeholder_literal"]),
                original_value=str(obj.get("original_value", "")),
            )
        )
    return entries


def restore_entry_in_line(raw_line: str, entry: MappingEntry) -> tuple[str, bool]:
    if raw_line.lstrip().startswith("#"):
        return raw_line, False

    comment_at = find_comment_start(raw_line)
    if comment_at is None:
        code_part, comment_part = raw_line, ""
    else:
        code_part, comment_part = raw_line[:comment_at], raw_line[comment_at:]

    matches = list(ASSIGN_RE.finditer(code_part))
    if not matches:
        return raw_line, False

    updated = code_part
    changed = False
    for m in reversed(matches):
        key = m.group("key")
        literal = m.group("literal")
        if key != entry.key or literal != entry.placeholder_literal:
            continue
        updated = (
            updated[: m.start("literal")]
            + entry.original_literal
            + updated[m.end("literal") :]
        )
        changed = True
        break

    if not changed:
        return raw_line, False
    return updated + comment_part, True


def restore_entry_in_lines(lines: list[str], entry: MappingEntry) -> bool:
    # First try the original recorded line for the exact key.
    if 1 <= entry.line <= len(lines):
        idx = entry.line - 1
        updated, changed = restore_entry_in_line(lines[idx], entry)
        if changed:
            lines[idx] = updated
            return True

    # Then search the file for a unique exact key+placeholder match.
    candidates: list[int] = []
    for idx, raw_line in enumerate(lines):
        _, changed = restore_entry_in_line(raw_line, entry)
        if changed:
            candidates.append(idx)

    if len(candidates) == 1:
        idx = candidates[0]
        updated, changed = restore_entry_in_line(lines[idx], entry)
        if changed:
            lines[idx] = updated
            return True

    # Last-resort fallback: restore by unique placeholder occurrence anywhere in file.
    joined = "".join(lines)
    if joined.count(entry.placeholder_literal) == 1:
        joined = joined.replace(entry.placeholder_literal, entry.original_literal, 1)
        lines[:] = joined.splitlines(keepends=True)
        return True

    return False


def run_redact(config_dir: Path, secret_map: Path, dry_run: bool) -> int:
    files = iter_toml_files(config_dir)
    if not files:
        print(f"No TOML files found under: {config_dir}")
        return 1

    all_entries: list[MappingEntry] = []
    changed_files = 0
    counter = 1

    for f in files:
        rel = f.relative_to(config_dir).as_posix()
        new_text, entries, counter = redact_file(f, rel, counter)
        if entries:
            changed_files += 1
            all_entries.extend(entries)
            if dry_run:
                print(f"[dry-run] Would redact {len(entries)} values in {rel}")
            else:
                f.write_text(new_text, encoding="utf-8")

    if not all_entries:
        print("No sensitive values found to redact.")
        return 0

    write_mapping(secret_map, all_entries, dry_run=dry_run)
    print(
        f"Redaction complete: {len(all_entries)} values in {changed_files} files. "
        f"Secret map: {secret_map}"
    )
    return 0


def run_restore(config_dir: Path, secret_map: Path, dry_run: bool) -> int:
    entries = load_mapping(secret_map)
    if not entries:
        print("No entries in secret map.")
        return 0

    by_file: dict[str, list[MappingEntry]] = {}
    for e in entries:
        by_file.setdefault(e.file, []).append(e)

    changed_files = 0
    restored_count = 0
    missing_count = 0

    for rel_file, file_entries in by_file.items():
        path = config_dir / rel_file
        if not path.exists():
            # Backward compatibility for old mapping files that stored cwd-relative paths.
            legacy_path = Path.cwd() / rel_file
            if legacy_path.exists():
                path = legacy_path
        if not path.exists():
            print(f"[warn] file missing, skip: {rel_file}")
            missing_count += len(file_entries)
            continue

        text = path.read_text(encoding="utf-8")
        original_text = text
        lines = text.splitlines(keepends=True)

        for e in file_entries:
            if restore_entry_in_lines(lines, e):
                restored_count += 1
            else:
                missing_count += 1
                print(
                    f"[warn] placeholder not found: {e.entry_id} ({e.file}:{e.line}, key={e.key})"
                )

        text = "".join(lines)

        if text != original_text:
            changed_files += 1
            if dry_run:
                print(f"[dry-run] Would restore values in {rel_file}")
            else:
                path.write_text(text, encoding="utf-8")

    print(
        f"Restore complete: restored={restored_count}, missing={missing_count}, files_changed={changed_files}"
    )
    return 0


def main() -> int:
    args = parse_args()
    config_dir_raw = Path(args.config_dir)
    secret_map_raw = Path(args.secret_map)

    config_dir = (
        config_dir_raw
        if config_dir_raw.is_absolute()
        else (PROJECT_ROOT / config_dir_raw)
    ).resolve()
    secret_map = (
        secret_map_raw
        if secret_map_raw.is_absolute()
        else (PROJECT_ROOT / secret_map_raw)
    ).resolve()

    if not config_dir.exists() or not config_dir.is_dir():
        print(f"config dir not found: {config_dir}", file=sys.stderr)
        return 2

    try:
        if args.mode == "redact":
            return run_redact(config_dir, secret_map, dry_run=args.dry_run)
        return run_restore(config_dir, secret_map, dry_run=args.dry_run)
    except Exception as err:
        print(f"error: {err}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
