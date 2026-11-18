#!/usr/bin/env python3
"""Scan staged configuration blobs, never unstaged contents or raw secret output.

This is a lightweight configuration guard, not a full source/publication scan.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shlex
import stat
import subprocess
import sys


MAX_CONFIG_BYTES = 16 * 1024 * 1024
OBJECT_ID = re.compile(rb"(?:[0-9a-f]{40}|[0-9a-f]{64})")
ASSIGNMENT = re.compile(
    r'''(?i)(?<![\w-])["']?(api_key|apikey|access_token|refresh_token|token|secret|password|app_secret)["']?\s*[:=]\s*(?:"([^"\r\n]*)"|'([^'\r\n]*)'|([^\s#,\r\n]+))'''
)
SAFE_VALUE = re.compile(r"(?:REPLACE_ME(?:_[A-Za-z0-9_]*)?|your_[A-Za-z0-9_]+_here|\$\{[A-Z_][A-Z0-9_]*\}|\$[A-Z_][A-Z0-9_]*|\*+)")
KEY_PREFIX = re.compile(r"\b(?:sk|tp|ak)-[A-Za-z0-9][A-Za-z0-9_-]{15,}\b")
LONG_VALUE = re.compile(r"[A-Za-z0-9_-]{24,}")
PRIVATE_KEY = re.compile(r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----")


class GuardError(Exception):
    pass


def git(root, *args):
    result = subprocess.run(["git", "--no-pager", "-C", os.fspath(root), *args],
                            capture_output=True, check=False)
    if result.returncode:
        raise GuardError("git_object_or_index_read_failed")
    return result.stdout


def config_path(path):
    parts = Path(path).parts
    name = Path(path).name
    return (bool(parts) and parts[0] == "configs") or name.endswith(
        (".toml", ".yaml", ".yml", ".env")) or name.startswith(".env.")


def findings(content):
    if len(content) > MAX_CONFIG_BYTES:
        raise GuardError("configuration_too_large")
    try:
        text = content.decode("utf-8")
    except UnicodeDecodeError:
        raise GuardError("configuration_not_utf8") from None
    for number, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith(("#", "//")):
            continue
        if PRIVATE_KEY.search(line):
            yield number, "private_key_material"
            continue
        if KEY_PREFIX.search(line):
            yield number, "credential_prefix"
            continue
        for match in ASSIGNMENT.finditer(line):
            value = next(value for value in match.groups()[1:] if value is not None)
            if value and not SAFE_VALUE.fullmatch(value) and LONG_VALUE.fullmatch(value):
                yield number, "credential_literal"
                break


def staged_blobs(root):
    # Raw records pin immutable object IDs; filenames containing newlines are data.
    raw = git(root, "diff", "--cached", "--raw", "--no-abbrev", "--no-renames",
              "--no-ext-diff", "--no-textconv", "--diff-filter=ACMT", "-z")
    fields = raw.split(b"\0")
    if fields[-1] != b"" or (len(fields) - 1) % 2:
        raise GuardError("index_records_invalid")
    for offset in range(0, len(fields) - 1, 2):
        header = fields[offset].split()
        if (len(header) != 5 or not header[0].startswith(b":")
                or header[4] not in (b"A", b"C", b"M", b"T")
                or not OBJECT_ID.fullmatch(header[3])):
            raise GuardError("index_records_invalid")
        path = os.fsdecode(fields[offset + 1])
        if not config_path(path):
            continue
        if header[1] not in (b"100644", b"100755"):
            raise GuardError("configuration_not_regular_blob")
        oid = header[3].decode("ascii")
        size = git(root, "cat-file", "-s", oid).strip()
        if not size.isdigit() or int(size) > MAX_CONFIG_BYTES:
            raise GuardError("configuration_too_large")
        yield path, git(root, "cat-file", "blob", oid)


def explicit_files(paths):
    for supplied in paths:
        path = Path(supplied)
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise GuardError("explicit_path_is_symlink")
        if stat.S_ISDIR(metadata.st_mode):
            for base, directories, names in os.walk(path, followlinks=False):
                directories[:] = [name for name in directories if name != ".git"]
                for name in names:
                    child = Path(base) / name
                    if config_path(child):
                        yield from explicit_files([child])
        elif config_path(path):
            if not stat.S_ISREG(metadata.st_mode):
                raise GuardError("configuration_not_regular_file")
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            with os.fdopen(fd, "rb") as source:
                if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
                    raise GuardError("configuration_not_regular_file")
                yield os.fspath(path), source.read(MAX_CONFIG_BYTES + 1)


def install_hook(root):
    location = os.fsdecode(git(root, "rev-parse", "--git-path", "hooks/pre-commit").removesuffix(b"\n"))
    path = Path(location)
    if not path.is_absolute():
        path = root / path
    path.parent.mkdir(parents=True, exist_ok=True)
    entry = Path(__file__).resolve().with_name("check-secrets.sh")
    try:
        with path.open("x", encoding="utf-8") as hook:
            hook.write(f"#!/usr/bin/env bash\nset -eu\nexec {shlex.quote(str(entry))} --staged\n")
    except FileExistsError:
        raise GuardError("existing_hook_not_replaced") from None
    path.chmod(0o755)
    print("SECRET_GUARD hook_installed")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--staged", action="store_true", help="scan index blobs (default)")
    modes.add_argument("--install", action="store_true", help="install without replacing an existing hook")
    parser.add_argument("paths", nargs="*", help="explicit worktree configuration files/directories")
    args = parser.parse_args(argv)
    if args.paths and (args.staged or args.install):
        parser.error("paths cannot be combined with --staged or --install")
    try:
        if args.paths:
            selected = explicit_files(args.paths)
        else:
            root = Path(os.fsdecode(git(Path.cwd(), "rev-parse", "--show-toplevel").removesuffix(b"\n")))
            if args.install:
                install_hook(root)
                return 0
            if git(root, "ls-files", "--unmerged", "-z"):
                raise GuardError("unmerged_index")
            selected = staged_blobs(root)
        count = 0
        failures = 0
        for name, content in selected:
            count += 1
            for number, rule in findings(content):
                print(f"SECRET: {json.dumps(name, ensure_ascii=True)}:{number}: {rule}")
                failures += 1
        print(f"SECRET_GUARD files={count} findings={failures}")
        return 1 if failures else 0
    except (GuardError, OSError, subprocess.SubprocessError) as error:
        reason = str(error) if isinstance(error, GuardError) else "configuration_or_process_read_failed"
        print(f"SECRET_GUARD refused: {reason}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
