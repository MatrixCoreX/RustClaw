#!/usr/bin/env bash
set -euo pipefail

# One-click rollback to a target commit-ish (default: HEAD).
TARGET="${1:-HEAD}"
git reset --hard "$TARGET"
git clean -fd

echo "Rollback finished: restored to $TARGET and cleaned untracked files."
