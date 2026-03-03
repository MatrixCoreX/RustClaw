#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

SANITIZED_CONFIG="$SCRIPT_DIR/configs/config.release.sanitized.toml"
if [[ ! -f "$SANITIZED_CONFIG" ]]; then
  echo "Missing sanitized config: $SANITIZED_CONFIG"
  echo "Please create configs/config.release.sanitized.toml first."
  exit 1
fi

echo "[1/6] Build workspace in release profile..."
cargo build --workspace --release

echo "[2/6] Discover and verify release binaries..."
WORKSPACE_METADATA="$(cargo metadata --no-deps --format-version 1)"
export RUSTCLAW_WORKSPACE_METADATA="$WORKSPACE_METADATA"

mapfile -t REQUIRED_BINS < <(
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
        kinds = target.get("kind", [])
        if "bin" in kinds:
            name = (target.get("name") or "").strip()
            if name:
                bins.add(name)

for name in sorted(bins):
    print(name)
PY
)

if [[ "${#REQUIRED_BINS[@]}" -eq 0 ]]; then
  echo "No workspace binaries discovered."
  exit 1
fi

for bin in "${REQUIRED_BINS[@]}"; do
  if [[ ! -x "$SCRIPT_DIR/target/release/$bin" ]]; then
    echo "Missing release binary: target/release/$bin"
    exit 1
  fi
done

echo "[3/6] Check UI build freshness..."
UI_DIR="$SCRIPT_DIR/UI"
if [[ -d "$UI_DIR" ]]; then
  if ! command -v npm >/dev/null 2>&1; then
    echo "npm is required to build UI, but not found."
    exit 1
  fi
  if [[ ! -f "$UI_DIR/package.json" ]]; then
    echo "UI package.json not found: $UI_DIR/package.json"
    exit 1
  fi
  UI_BUILD_REASON="$(
python3 - <<'PY'
import os
from pathlib import Path

ui = Path("UI")
dist = ui / "dist"

if not ui.exists():
    print("no_ui")
    raise SystemExit(0)
if not dist.exists() or not (dist / "index.html").exists():
    print("missing_dist")
    raise SystemExit(0)

scan_paths = [
    ui / "src",
    ui / "public",
    ui / "index.html",
    ui / "package.json",
    ui / "package-lock.json",
    ui / "vite.config.ts",
    ui / "vite.config.js",
    ui / "tsconfig.json",
]

def latest_mtime(paths):
    latest = 0.0
    for p in paths:
        if not p.exists():
            continue
        if p.is_file():
            latest = max(latest, p.stat().st_mtime)
            continue
        for root, _, files in os.walk(p):
            for name in files:
                fp = Path(root) / name
                try:
                    latest = max(latest, fp.stat().st_mtime)
                except OSError:
                    pass
    return latest

src_latest = latest_mtime(scan_paths)
dist_latest = latest_mtime([dist])
if src_latest > dist_latest:
    print("stale_dist")
else:
    print("up_to_date")
PY
)"
  if [[ "$UI_BUILD_REASON" == "missing_dist" || "$UI_BUILD_REASON" == "stale_dist" ]]; then
    echo "UI build required: $UI_BUILD_REASON"
    if [[ ! -d "$UI_DIR/node_modules" ]]; then
      echo "Installing UI dependencies..."
      (cd "$UI_DIR" && npm install)
    fi
    echo "Building UI assets..."
    (cd "$UI_DIR" && npm run build)
  else
    echo "UI assets are up-to-date."
  fi
else
  echo "UI directory not found, skip UI build."
fi

echo "[4/6] Prepare staging directory..."
STAGE_ROOT="$(mktemp -d)"
trap 'rm -rf "$STAGE_ROOT"' EXIT
STAGE_PROJECT_DIR="$STAGE_ROOT/RustClaw"
mkdir -p "$STAGE_PROJECT_DIR"

copy_if_exists() {
  local rel="$1"
  if [[ -e "$SCRIPT_DIR/$rel" ]]; then
    mkdir -p "$STAGE_PROJECT_DIR/$(dirname "$rel")"
    cp -a "$SCRIPT_DIR/$rel" "$STAGE_PROJECT_DIR/$rel"
  else
    echo "Warning: skip missing path: $rel"
  fi
}

copy_if_exists "configs"
copy_if_exists "prompts"
copy_if_exists "migrations"
copy_if_exists "scripts"
copy_if_exists "services/wa-web-bridge"
copy_if_exists "README.md"
copy_if_exists "start-all.sh"
copy_if_exists "start-all-bin.sh"
copy_if_exists "start-clawd.sh"
copy_if_exists "start-clawd-ui.sh"
copy_if_exists "start-telegramd.sh"
copy_if_exists "start-whatsappd.sh"
copy_if_exists "start-whatsapp-webd.sh"
copy_if_exists "start-future-adapters.sh"
copy_if_exists "stop-rustclaw.sh"

if [[ -d "$SCRIPT_DIR/UI/dist" ]]; then
  mkdir -p "$STAGE_PROJECT_DIR/UI"
  cp -a "$SCRIPT_DIR/UI/dist" "$STAGE_PROJECT_DIR/UI/dist"
else
  echo "Warning: UI/dist not found, package will not include built UI assets."
fi

mkdir -p "$STAGE_PROJECT_DIR/target/release"
for bin in "${REQUIRED_BINS[@]}"; do
  cp -a "$SCRIPT_DIR/target/release/$bin" "$STAGE_PROJECT_DIR/target/release/$bin"
done

echo "[5/6] Apply sanitized config as configs/config.toml..."
cp -a "$SANITIZED_CONFIG" "$STAGE_PROJECT_DIR/configs/config.toml"
rm -f "$STAGE_PROJECT_DIR/configs/config.release.sanitized.toml"

echo "[5.5/6] Force packaged scripts to release defaults..."
export STAGE_PROJECT_DIR
python3 - <<'PY'
from pathlib import Path

root = Path("tmp-not-used")
del root
stage = Path(__import__("os").environ["STAGE_PROJECT_DIR"])

script_names = [
    "start-all.sh",
    "start-all-bin.sh",
    "start-clawd.sh",
    "start-clawd-ui.sh",
    "start-telegramd.sh",
    "start-whatsappd.sh",
    "start-whatsapp-webd.sh",
]

for name in script_names:
    p = stage / name
    if not p.exists():
        continue
    s = p.read_text(encoding="utf-8")
    s = s.replace("RUSTCLAW_START_PROFILE:-debug", "RUSTCLAW_START_PROFILE:-release")
    s = s.replace('PROFILE="${1:-debug}"', 'PROFILE="${1:-release}"')
    s = s.replace('PROFILE="${3:-${RUSTCLAW_START_PROFILE:-debug}}"', 'PROFILE="${3:-${RUSTCLAW_START_PROFILE:-release}}"')
    p.write_text(s, encoding="utf-8")
PY

echo "[6/6] Create package in RustClaw_bundle and current dir..."
BUNDLE_DIR="$HOME/RustClaw_bundle"
mkdir -p "$BUNDLE_DIR"
TS="$(date +%Y%m%d-%H%M%S)"
OUT="$BUNDLE_DIR/RustClaw-runtime-release-${TS}.tar.gz"
tar -czf "$OUT" -C "$STAGE_ROOT" RustClaw
LOCAL_OUT="$SCRIPT_DIR/$(basename "$OUT")"
cp -f "$OUT" "$LOCAL_OUT"

cleanup_old_packages() {
  local dir="$1"
  local keep_file="$2"
  local pattern="$dir/RustClaw-runtime-release-*.tar.gz"
  shopt -s nullglob
  local files=( $pattern )
  shopt -u nullglob
  for f in "${files[@]}"; do
    if [[ "$f" != "$keep_file" ]]; then
      rm -f "$f"
      echo "Removed old package: $f"
    fi
  done
}

echo "[6.5/6] Remove older release packages..."
cleanup_old_packages "$BUNDLE_DIR" "$OUT"
cleanup_old_packages "$SCRIPT_DIR" "$LOCAL_OUT"

echo "Package created: $OUT"
ls -lh "$OUT"
echo "Local copy created: $LOCAL_OUT"
ls -lh "$LOCAL_OUT"
