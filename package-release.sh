#!/usr/bin/env bash
# 打包「开盒即用」发布包：仅含预编译二进制、前端构建产物(UI/dist)、配置、脚本等；
# 不含 UI 源码、不含主程序(Rust) 源码；解压即可运行，无需编译或构建。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

# 优先使用已脱敏的发布配置；若无则用 config.toml，打包时步骤 5.3 会再脱敏
if [[ -f "$SCRIPT_DIR/configs/config.release.sanitized.toml" ]]; then
  SANITIZED_CONFIG="$SCRIPT_DIR/configs/config.release.sanitized.toml"
elif [[ -f "$SCRIPT_DIR/configs/config.toml" ]]; then
  SANITIZED_CONFIG="$SCRIPT_DIR/configs/config.toml"
else
  echo "Missing config: need configs/config.toml or configs/config.release.sanitized.toml"
  exit 1
fi

echo "[1/6] Pack only (no build); discover release binaries..."
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
    cp -a "$SCRIPT_DIR/$rel" "$STAGE_PROJECT_DIR/$rel"
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
copy_if_exists "start-clawd.sh"
copy_if_exists "start-clawd-ui.sh"
copy_if_exists "start-telegramd.sh"
copy_if_exists "start-whatsappd.sh"
copy_if_exists "start-whatsapp-webd.sh"
copy_if_exists "start-wechatd.sh"
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

echo "[4.5/6] Add usage note (开盒即用)..."
cat > "$STAGE_PROJECT_DIR/使用说明.txt" <<'USAGE'
RustClaw 运行时包 — 解压即用

1) 解压后进入本目录。
2) 首次运行前请根据 configs/ 配置渠道（如 Telegram/WhatsApp）与模型等。
3) 启动方式任选其一：
   - ./start-all.sh <vendor> <model> release [channels]
     例：./start-all.sh openai gpt-4o release telegram
   - ./rustclaw -start release all --quick
   - 仅后端：./start-all-bin.sh release
4) 停止：./stop-rustclaw.sh
5) 数据与日志：data/（数据库）、logs/（运行日志）。
6) 树莓派小屏（可选）：见 pi_app/ 内脚本与 README；也可用 ./install-rustclaw-cmd.sh --pi-app 配置桌面与自启。
USAGE
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
    "start-wechatd.sh",
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
