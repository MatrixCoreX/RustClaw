#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/shell_compat.sh"
SOURCE_DIR="${1:-${RUSTCLAW_RELEASE_SOURCE:-$ROOT_DIR/target/release}}"
DEST_DIR="${RUSTCLAW_RELEASE_BIN_DIR:-$ROOT_DIR/release-bin}"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found; cannot discover workspace binaries."
  exit 1
fi

if [[ ! -d "$SOURCE_DIR" ]]; then
  echo "Release source directory not found: $SOURCE_DIR"
  exit 1
fi

WORKSPACE_METADATA="$(cargo metadata --no-deps --format-version 1)"
export RUSTCLAW_WORKSPACE_METADATA="$WORKSPACE_METADATA"

array_from_command_lines REQUIRED_BINS \
  python3 - <<'PY'
import json
import os

raw = os.environ.get("RUSTCLAW_WORKSPACE_METADATA", "").strip()
if not raw:
    raise SystemExit(1)

data = json.loads(raw)
workspace_members = set(data.get("workspace_members", []))
bins = set()

for pkg in data.get("packages", []):
    if pkg.get("id") not in workspace_members:
        continue
    for target in pkg.get("targets", []):
        if "bin" in (target.get("kind", []) or []):
            name = (target.get("name") or "").strip()
            if name:
                bins.add(name)

for name in sorted(bins):
    print(name)
PY

if [[ "${#REQUIRED_BINS[@]}" -eq 0 ]]; then
  echo "No workspace binaries discovered."
  exit 1
fi

mkdir -p "$DEST_DIR"

for bin in "${REQUIRED_BINS[@]}"; do
  src="$SOURCE_DIR/$bin"
  if [[ ! -x "$src" ]]; then
    echo "Missing executable: $src"
    exit 1
  fi
done

for existing in "$DEST_DIR"/*; do
  [[ -e "$existing" ]] || continue
  name="$(basename "$existing")"
  keep=0
  for bin in "${REQUIRED_BINS[@]}"; do
    if [[ "$name" == "$bin" ]]; then
      keep=1
      break
    fi
  done
  if [[ "$keep" == "0" ]]; then
    rm -f "$existing"
  fi
done

for bin in "${REQUIRED_BINS[@]}"; do
  src="$SOURCE_DIR/$bin"
  dst="$DEST_DIR/$bin"
  cp -f "$src" "$dst"
  chmod +x "$dst"
  echo "Synced $bin"
done

echo "Release executables are ready in: $DEST_DIR"
