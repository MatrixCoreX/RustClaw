#!/usr/bin/env bash
# 打包「开盒即用」发布包：仅含预编译二进制、前端构建产物(UI/dist)、配置、脚本等；
# 不含 UI 源码、不含主程序(Rust) 源码；解压即可运行，无需编译或构建。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/scripts/shell_compat.sh"
cd "$SCRIPT_DIR"

EXPLICIT_RELEASE_BIN_DIR="${RUSTCLAW_RELEASE_BIN_DIR:-}"
TRACKED_RELEASE_DIR="$SCRIPT_DIR/release-bin"
BUILD_RELEASE_DIR="${RUSTCLAW_BUILD_RELEASE_DIR:-$SCRIPT_DIR/target/release}"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

resolve_release_bin() {
  local name="$1"
  if [[ -n "$EXPLICIT_RELEASE_BIN_DIR" ]]; then
    printf '%s\n' "$EXPLICIT_RELEASE_BIN_DIR/$name"
    return
  fi
  if [[ -x "$TRACKED_RELEASE_DIR/$name" ]]; then
    printf '%s\n' "$TRACKED_RELEASE_DIR/$name"
    return
  fi
  printf '%s\n' "$BUILD_RELEASE_DIR/$name"
}

# 优先使用已脱敏的发布配置；若无则用 config.toml，打包时步骤 5.3 会再脱敏
if [[ -f "$SCRIPT_DIR/configs/config.release.sanitized.toml" ]]; then
  SANITIZED_CONFIG="$SCRIPT_DIR/configs/config.release.sanitized.toml"
elif [[ -f "$SCRIPT_DIR/configs/config.toml" ]]; then
  SANITIZED_CONFIG="$SCRIPT_DIR/configs/config.toml"
else
  echo "Missing config: need configs/config.toml or configs/config.release.sanitized.toml"
  exit 1
fi

echo "[1/6] Pack only (no build); discover runtime release binaries..."
WORKSPACE_METADATA="$(cargo metadata --no-deps --format-version 1)"
export RUSTCLAW_WORKSPACE_METADATA="$WORKSPACE_METADATA"
RUSTCLAW_ON_DEMAND_PACKAGES="$(python3 "$SCRIPT_DIR/scripts/skill_store_packages.py" --format packages)"
export RUSTCLAW_ON_DEMAND_PACKAGES

REQUIRED_BINS_RAW="$(
  python3 - <<'PY'
import json
import os

raw = os.environ.get("RUSTCLAW_WORKSPACE_METADATA", "").strip()
if not raw:
    raise SystemExit(1)
data = json.loads(raw)
workspace_members = set(data.get("workspace_members", []))
on_demand_packages = {
    value.strip()
    for value in os.environ.get("RUSTCLAW_ON_DEMAND_PACKAGES", "").splitlines()
    if value.strip()
}
bins = set()

for pkg in data.get("packages", []):
    if pkg.get("id") not in workspace_members:
        continue
    if pkg.get("name") in on_demand_packages:
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
)"
array_from_string_lines REQUIRED_BINS "$REQUIRED_BINS_RAW"

if [[ "${#REQUIRED_BINS[@]}" -eq 0 ]]; then
  echo "No workspace binaries discovered."
  exit 1
fi

for bin in "${REQUIRED_BINS[@]}"; do
  bin_path="$(resolve_release_bin "$bin")"
  if [[ ! -x "$bin_path" ]]; then
    echo "Missing release binary: $bin_path"
    exit 1
  fi
done

echo "[2/6] UI: pack existing UI/dist if present (no build)..."
if [[ -d "$SCRIPT_DIR/UI/dist" ]] && [[ -f "$SCRIPT_DIR/UI/dist/index.html" ]]; then
  echo "UI/dist found, will include in package."
else
  echo "UI/dist missing or incomplete; package will not include frontend assets."
fi

echo "[3/6] Prepare staging directory..."
STAGE_ROOT="$(mktemp -d)"
trap 'rm -rf "$STAGE_ROOT"' EXIT
STAGE_PROJECT_DIR="$STAGE_ROOT/RustClaw"
mkdir -p "$STAGE_PROJECT_DIR"

copy_if_exists() {
  local rel="$1"
  if [[ -e "$SCRIPT_DIR/$rel" ]]; then
    mkdir -p "$STAGE_PROJECT_DIR/$(dirname "$rel")"
    cp -R "$SCRIPT_DIR/$rel" "$STAGE_PROJECT_DIR/$rel"
  else
    echo "Warning: skip missing path: $rel"
  fi
}

copy_if_exists "configs"
copy_if_exists "prompts"
copy_if_exists "migrations"
copy_if_exists "scripts"
copy_if_exists "pi_app"
copy_if_exists "services/wa-web-bridge"
copy_if_exists "README.md"
copy_if_exists "rustclaw"
copy_if_exists "install-rustclaw-cmd.sh"
copy_if_exists "start-all.sh"
copy_if_exists "start-all-bin.sh"
copy_if_exists "component_start"
copy_if_exists "stop-rustclaw.sh"

if [[ -d "$SCRIPT_DIR/UI/dist" ]]; then
  mkdir -p "$STAGE_PROJECT_DIR/UI"
  cp -R "$SCRIPT_DIR/UI/dist" "$STAGE_PROJECT_DIR/UI/dist"
else
  echo "Warning: UI/dist not found, package will not include built UI assets."
fi

mkdir -p "$STAGE_PROJECT_DIR/target/release"
for bin in "${REQUIRED_BINS[@]}"; do
  cp -R "$(resolve_release_bin "$bin")" "$STAGE_PROJECT_DIR/target/release/$bin"
done

# zh: 发布包内只生成英文 USAGE.txt，避免运行时/交付物混用多语言输出。
echo "[4.5/6] Add usage note (ready to run)..."
cat > "$STAGE_PROJECT_DIR/USAGE.txt" <<'USAGE_EN'
RustClaw runtime package — ready to run

1) Extract the archive and cd into this directory.
2) Before first run, configure channels (e.g. Telegram/WhatsApp) and models under configs/.
3) Start with one of:
   - ./start-all.sh <vendor> <model> release [channels]
     e.g. ./start-all.sh openai gpt-4o release telegram
   - ./rustclaw -start release all --quick
   - Backend only: ./start-all-bin.sh release
4) Stop: ./stop-rustclaw.sh
5) Data and logs: data/ (database), logs/ (runtime logs).
6) Raspberry Pi small screen (optional): see pi_app/ scripts and README; or ./install-rustclaw-cmd.sh --pi-app for desktop shortcut and autostart.
USAGE_EN

echo "[5/6] Apply sanitized config as configs/config.toml..."
cp -R "$SANITIZED_CONFIG" "$STAGE_PROJECT_DIR/configs/config.toml"
rm -f "$STAGE_PROJECT_DIR/configs/config.release.sanitized.toml"

echo "[5.2/6] Verify required config directories in package..."
for required_dir in \
  "$STAGE_PROJECT_DIR/configs/channels" \
  "$STAGE_PROJECT_DIR/configs/i18n"; do
  if [[ ! -d "$required_dir" ]]; then
    echo "Missing required config directory in package: $required_dir"
    exit 1
  fi
done

echo "[5.3/6] Sanitize sensitive fields in packaged configs (all configs/*.toml)..."
export STAGE_PROJECT_DIR
python3 - <<'PY'
from pathlib import Path
import re
import os

stage = Path(os.environ["STAGE_PROJECT_DIR"])
configs_dir = stage / "configs"
targets = list(configs_dir.rglob("*.toml")) if configs_dir.exists() else []

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

echo "[5.5/6] Packaged scripts already use release defaults."

echo "[6/6] Create package in RustClaw_bundle and current dir..."
BUNDLE_DIR="${RUSTCLAW_BUNDLE_DIR:-$HOME/RustClaw_bundle}"
mkdir -p "$BUNDLE_DIR"
TS="$(date +%Y%m%d-%H%M%S)"
PACKAGE_BASENAME="${RUSTCLAW_PACKAGE_BASENAME:-RustClaw-runtime-release-${TS}.tar.gz}"
OUT="$BUNDLE_DIR/$PACKAGE_BASENAME"
tar -czf "$OUT" -C "$STAGE_ROOT" RustClaw
LOCAL_OUT="$SCRIPT_DIR/$(basename "$OUT")"
if [[ "${RUSTCLAW_SKIP_LOCAL_PACKAGE_COPY:-0}" != "1" ]]; then
  cp -f "$OUT" "$LOCAL_OUT"
fi

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
if [[ "${RUSTCLAW_SKIP_LOCAL_PACKAGE_COPY:-0}" != "1" ]]; then
  cleanup_old_packages "$SCRIPT_DIR" "$LOCAL_OUT"
fi

echo "Package created: $OUT"
ls -lh "$OUT"
if [[ "${RUSTCLAW_SKIP_LOCAL_PACKAGE_COPY:-0}" != "1" ]]; then
  echo "Local copy created: $LOCAL_OUT"
  ls -lh "$LOCAL_OUT"
fi
