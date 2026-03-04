#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="$SCRIPT_DIR/rustclaw"
DEFAULT_INSTALL_DIR="/usr/local/bin"
USER_INSTALL_DIR="${HOME}/.local/bin"
FORCE_BUILD=0
SKIP_BUILD=0
USE_USER_DIR=0
INSTALL_DIR="$DEFAULT_INSTALL_DIR"

usage() {
  cat <<'EOF'
Usage:
  bash install-rustclaw-cmd.sh [options]

Options:
  --force-build    Force rebuild before install
  --skip-build     Skip build check/build step
  --user           Install to ~/.local/bin (no sudo)
  --dir <path>     Install to custom directory
  -h, --help       Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force-build)
      FORCE_BUILD=1
      ;;
    --skip-build)
      SKIP_BUILD=1
      ;;
    --user)
      USE_USER_DIR=1
      INSTALL_DIR="$USER_INSTALL_DIR"
      ;;
    --dir)
      shift
      if [[ $# -lt 1 ]]; then
        echo "Missing value for --dir"
        exit 1
      fi
      INSTALL_DIR="$1"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      usage
      exit 1
      ;;
  esac
  shift
done

LINK_PATH="$INSTALL_DIR/rustclaw"

ensure_build() {
  local force="$1"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found. Please install Rust toolchain first."
    exit 1
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 not found."
    exit 1
  fi

  local need_build
  need_build="$(
python3 - "$SCRIPT_DIR" "$force" <<'PY'
import json
import os
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
mode = sys.argv[2].strip().lower()
release_dir = root / "target" / "release"

def latest_source_mtime(base: Path) -> float:
    latest = 0.0
    tracked_ext = {".rs", ".toml", ".lock"}
    tracked_names = {"Cargo.toml", "Cargo.lock"}
    for current, dirs, files in os.walk(base):
        p = Path(current)
        # Skip heavy/non-build folders.
        if any(seg in {"target", ".git", "node_modules"} for seg in p.parts):
            continue
        for name in files:
            fp = p / name
            if fp.name in tracked_names or fp.suffix in tracked_ext:
                try:
                    latest = max(latest, fp.stat().st_mtime)
                except OSError:
                    pass
    return latest

if mode == "--force-build":
    print("1")
    raise SystemExit(0)

metadata_raw = subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"],
    cwd=str(root),
    text=True,
)
meta = json.loads(metadata_raw)
workspace_members = set(meta.get("workspace_members", []))
bins = set()
for pkg in meta.get("packages", []):
    if pkg.get("id") not in workspace_members:
        continue
    for target in pkg.get("targets", []):
        if "bin" in (target.get("kind", []) or []):
            name = (target.get("name") or "").strip()
            if name:
                bins.add(name)

if not bins:
    print("1")
    raise SystemExit(0)

latest_src = latest_source_mtime(root)
if latest_src <= 0:
    print("1")
    raise SystemExit(0)

oldest_bin = None
for name in sorted(bins):
    bp = release_dir / name
    if not bp.exists():
        print("1")
        raise SystemExit(0)
    try:
        m = bp.stat().st_mtime
    except OSError:
        print("1")
        raise SystemExit(0)
    oldest_bin = m if oldest_bin is None else min(oldest_bin, m)

if oldest_bin is None or oldest_bin < latest_src:
    print("1")
else:
    print("0")
PY
  )"

  if [[ "$need_build" == "1" ]]; then
    echo "Release binaries are missing or outdated. Building release..."
    "$SCRIPT_DIR/build-all.sh" release
  else
    echo "Release binaries are up-to-date. Skip build."
  fi
}

if [[ ! -f "$TARGET" ]]; then
  echo "Missing launcher script: $TARGET"
  exit 1
fi

if [[ "$SKIP_BUILD" == "0" ]]; then
  if [[ "$FORCE_BUILD" == "1" ]]; then
    ensure_build "--force-build"
  else
    ensure_build ""
  fi
else
  echo "Skip build check/build step by --skip-build."
fi

chmod +x "$TARGET"

install_without_sudo() {
  mkdir -p "$INSTALL_DIR"
  rm -f "$LINK_PATH"
  ln -s "$TARGET" "$LINK_PATH"
}

install_with_sudo() {
  sudo mkdir -p "$INSTALL_DIR"
  sudo rm -f "$LINK_PATH"
  sudo ln -s "$TARGET" "$LINK_PATH"
}

if [[ "$USE_USER_DIR" == "1" ]]; then
  install_without_sudo
elif [[ -w "$INSTALL_DIR" ]]; then
  install_without_sudo
elif command -v sudo >/dev/null 2>&1; then
  echo "Installing launcher to $LINK_PATH (sudo required)..."
  install_with_sudo
else
  echo "No write permission to $INSTALL_DIR and sudo is unavailable."
  echo "Falling back to user install path: $USER_INSTALL_DIR"
  INSTALL_DIR="$USER_INSTALL_DIR"
  LINK_PATH="$INSTALL_DIR/rustclaw"
  install_without_sudo
fi

echo "Installed: $LINK_PATH -> $TARGET"
if [[ "$LINK_PATH" == "$USER_INSTALL_DIR/rustclaw" ]]; then
  case ":$PATH:" in
    *":$USER_INSTALL_DIR:"*) ;;
    *)
      echo "Note: $USER_INSTALL_DIR is not in PATH."
      echo "Add this to your shell profile:"
      echo "  export PATH=\"$USER_INSTALL_DIR:\$PATH\""
      ;;
  esac
fi
echo "Try:"
echo "  rustclaw -status"
echo "  rustclaw -start release"
echo "  rustclaw -stop"
echo
echo "Tip:"
echo "  bash install-rustclaw-cmd.sh --force-build"
