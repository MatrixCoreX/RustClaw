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

echo "[1/6] Check whether release build is required..."
BUILD_REQUIRED="$(
python3 - <<'PY'
import json
import os
import subprocess
from pathlib import Path

root = Path(".").resolve()
release_dir = root / "target" / "release"

def latest_source_mtime(base: Path) -> float:
    latest = 0.0
    tracked_ext = {".rs", ".toml", ".lock"}
    tracked_names = {"Cargo.toml", "Cargo.lock"}
    for current, dirs, files in os.walk(base):
        p = Path(current)
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

meta_raw = subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"],
    cwd=str(root),
    text=True,
)
meta = json.loads(meta_raw)
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

if [[ "$BUILD_REQUIRED" == "1" ]]; then
  echo "Release binaries missing or outdated; building workspace..."
  cargo build --workspace --release
else
  echo "Release binaries are up-to-date; skip rebuild."
fi

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
copy_if_exists "rustclaw"
copy_if_exists "install-rustclaw-cmd.sh"
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

echo "[5.2/6] Verify required config directories in package..."
for required_dir in \
  "$STAGE_PROJECT_DIR/configs/channels" \
  "$STAGE_PROJECT_DIR/configs/i18n" \
  "$STAGE_PROJECT_DIR/configs/command_intent"; do
  if [[ ! -d "$required_dir" ]]; then
    echo "Missing required config directory in package: $required_dir"
    exit 1
  fi
done

echo "[5.3/6] Sanitize sensitive fields in packaged configs..."
export STAGE_PROJECT_DIR
python3 - <<'PY'
from pathlib import Path
import re
import os

stage = Path(os.environ["STAGE_PROJECT_DIR"])

targets = [
    stage / "configs" / "config.toml",
    stage / "configs" / "channels" / "telegram.toml",
    stage / "configs" / "crypto.toml",
]

rules = [
    # Telegram bot token
    (re.compile(r'^(\s*bot_token\s*=\s*).*$'), r'\1"REDACTED_TELEGRAM_BOT_TOKEN"'),
    # fields containing bot
    (re.compile(r'^(\s*[A-Za-z0-9_.-]*bot[A-Za-z0-9_.-]*\s*=\s*).*$',
                flags=re.IGNORECASE), r'\1"REDACTED_BOT"'),
    # fields containing id (numeric replacement to keep type)
    (re.compile(r'^(\s*[A-Za-z0-9_.-]*id[A-Za-z0-9_.-]*\s*=\s*).*$',
                flags=re.IGNORECASE), r'\g<1>0'),
    # admins list
    (re.compile(r'^(\s*admins\s*=\s*).*$'), r'\1[]'),
    # exchange/API secrets
    (re.compile(r'^(\s*api_key\s*=\s*).*$'), r'\1"REDACTED_API_KEY"'),
    (re.compile(r'^(\s*api_secret\s*=\s*).*$'), r'\1"REDACTED_API_SECRET"'),
    (re.compile(r'^(\s*passphrase\s*=\s*).*$'), r'\1"REDACTED_PASSPHRASE"'),
]

for fp in targets:
    if not fp.exists():
        continue
    lines = fp.read_text(encoding="utf-8").splitlines()
    out = []
    for line in lines:
        replaced = line
        for pat, repl in rules:
            if pat.match(replaced):
                replaced = pat.sub(repl, replaced)
        out.append(replaced)
    fp.write_text("\n".join(out) + "\n", encoding="utf-8")
PY

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
