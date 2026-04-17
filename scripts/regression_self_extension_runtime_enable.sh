#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

WORKSPACE_ROOT_OVERRIDE="${WORKSPACE_ROOT_OVERRIDE:-}"
PORT="${PORT:-}"
WAIT_SECONDS="${WAIT_SECONDS:-120}"
POLL_INTERVAL="${POLL_INTERVAL:-1}"
KEEP_WORKSPACE="${KEEP_WORKSPACE:-0}"
CLAWD_BIN="${CLAWD_BIN:-$ROOT_DIR/target/release/clawd}"

TEMP_WORKSPACE=""
CLAWD_PID=""
BASE_URL=""
USER_KEY=""
USER_ID=""
CHAT_ID=""
SKILL_NAME=""

usage() {
  cat <<'EOF'
Usage:
  bash scripts/regression_self_extension_runtime_enable.sh [--workspace-root DIR] [--port PORT] [--wait-seconds N] [--keep-workspace] [--clawd-bin PATH]

This regression creates an isolated temporary workspace, enables developer-mode
self-extension gates there, then verifies the deterministic backend chain:

  scaffold_external_skill -> validate_external_skill -> register_external_skill
  -> enable_external_skill -> admin reload-skills -> run_skill(new skill)

The script does not use provider-backed LLM calls.
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1" >&2
    exit 2
  }
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace-root)
      WORKSPACE_ROOT_OVERRIDE="${2:-}"
      shift 2
      ;;
    --port)
      PORT="${2:-}"
      shift 2
      ;;
    --wait-seconds)
      WAIT_SECONDS="${2:-}"
      shift 2
      ;;
    --keep-workspace)
      KEEP_WORKSPACE=1
      shift 1
      ;;
    --clawd-bin)
      CLAWD_BIN="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

need_cmd curl
need_cmd jq
need_cmd python3
need_cmd mktemp

pick_free_port() {
  python3 - <<'PY'
import socket

sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

make_seed_triplet() {
  python3 - <<'PY'
import time

seed = time.time_ns() % 900_000_000
base = 1_700_000_000 + seed
print(base, base + 1, seed)
PY
}

prepare_temp_workspace() {
  local workspace_root="$1"
  mkdir -p "$workspace_root"
  cp "$ROOT_DIR/Cargo.toml" "$workspace_root/Cargo.toml"
  if [[ -f "$ROOT_DIR/Cargo.lock" ]]; then
    cp "$ROOT_DIR/Cargo.lock" "$workspace_root/Cargo.lock"
  fi
  cp -R "$ROOT_DIR/configs" "$workspace_root/configs"
  cp -R "$ROOT_DIR/prompts" "$workspace_root/prompts"
  mkdir -p "$workspace_root/data" "$workspace_root/external_skills"
  ln -s "$ROOT_DIR/crates" "$workspace_root/crates"
  ln -s "$ROOT_DIR/scripts" "$workspace_root/scripts"
  ln -s "$ROOT_DIR/target" "$workspace_root/target"
}

patch_temp_config() {
  local config_path="$1"
  local port="$2"
  local sqlite_path="$3"
  python3 - "$config_path" "$port" "$sqlite_path" <<'PY'
from pathlib import Path
import re
import sys

config_path = Path(sys.argv[1])
port = sys.argv[2]
sqlite_path = sys.argv[3]
text = config_path.read_text(encoding="utf-8")

def replace_once(pattern: str, replacement: str, raw: str) -> str:
    updated, count = re.subn(pattern, replacement, raw, count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"failed to patch config pattern: {pattern}")
    return updated

text = replace_once(r'^sqlite_path\s*=\s*".*"$', f'sqlite_path = "{sqlite_path}"', text)
text = replace_once(r'^listen\s*=\s*".*"$', f'listen = "127.0.0.1:{port}"', text)
text = replace_once(r'extension_manager\s*=\s*(true|false)', 'extension_manager = true', text)
for key in (
    "enabled",
    "auto_on_capability_gap",
    "allow_execute",
    "allow_package_install",
    "allow_permanent_extension",
    "allow_runtime_enable",
):
    text = replace_once(rf'^{key}\s*=\s*(true|false)$', f'{key} = true', text)

config_path.write_text(text, encoding="utf-8")
PY
}

wait_for_health() {
  local waited=0
  while [[ "$waited" -le "$WAIT_SECONDS" ]]; do
    if curl -sS -H "X-RustClaw-Key: ${USER_KEY}" "${BASE_URL}/v1/health" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -n "$CLAWD_PID" ]] && ! kill -0 "$CLAWD_PID" >/dev/null 2>&1; then
      echo "clawd exited before health check succeeded" >&2
      return 1
    fi
    sleep "$POLL_INTERVAL"
    waited=$((waited + POLL_INTERVAL))
  done
  echo "health check timeout: ${BASE_URL}/v1/health" >&2
  return 1
}

submit_run_skill_and_wait() {
  local skill_name="$1"
  local args_json="$2"
  local submit_raw task_id final_raw status
  submit_raw="$(submit_run_skill_task "$skill_name" "$args_json")"
  task_id="$(extract_submit_task_id "$submit_raw")"
  final_raw="$(wait_task_until_terminal_with_limit "$task_id" "$WAIT_SECONDS")"
  status="$(echo "$final_raw" | jq -r '.data.status // ""')"
  if [[ "$status" != "succeeded" ]]; then
    echo "run_skill failed for ${skill_name}: ${final_raw}" >&2
    return 1
  fi
  printf '%s\n' "$final_raw"
}

reload_skills_and_wait() {
  local raw
  raw="$(
    curl -sS -X POST \
      -H "X-RustClaw-Key: ${USER_KEY}" \
      "${BASE_URL}/v1/admin/reload-skills"
  )"
  local ok
  ok="$(echo "$raw" | jq -r '.ok // false')"
  if [[ "$ok" != "true" ]]; then
    echo "reload-skills failed: $raw" >&2
    return 1
  fi
  printf '%s\n' "$raw"
}

cleanup() {
  local exit_code=$?
  if [[ -n "$CLAWD_PID" ]] && kill -0 "$CLAWD_PID" >/dev/null 2>&1; then
    kill "$CLAWD_PID" >/dev/null 2>&1 || true
    wait "$CLAWD_PID" >/dev/null 2>&1 || true
  fi
  if [[ "$KEEP_WORKSPACE" != "1" && -n "$TEMP_WORKSPACE" && -d "$TEMP_WORKSPACE" ]]; then
    rm -rf "$TEMP_WORKSPACE"
  fi
  exit "$exit_code"
}
trap cleanup EXIT

if [[ -z "$PORT" ]]; then
  PORT="$(pick_free_port)"
fi

if [[ -n "$WORKSPACE_ROOT_OVERRIDE" ]]; then
  TEMP_WORKSPACE="$WORKSPACE_ROOT_OVERRIDE"
else
  TEMP_WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/rustclaw-selfext-regression-XXXXXX")"
fi

prepare_temp_workspace "$TEMP_WORKSPACE"
patch_temp_config \
  "$TEMP_WORKSPACE/configs/config.toml" \
  "$PORT" \
  "$TEMP_WORKSPACE/data/self_extension_runtime_enable.sqlite"

read -r USER_ID CHAT_ID SEED < <(make_seed_triplet)
BASE_URL="http://127.0.0.1:${PORT}"
SKILL_NAME="smoke_selfext_${SEED}"
export BASE_URL USER_ID CHAT_ID

USER_KEY="$(
  RUSTCLAW_CONFIG_PATH="$TEMP_WORKSPACE/configs/config.toml" \
    bash "$ROOT_DIR/scripts/auth-key.sh" generate admin | awk '{print $1; exit}'
)"
export USER_KEY

if [[ ! -x "$CLAWD_BIN" ]]; then
  echo "clawd binary not found or not executable: $CLAWD_BIN" >&2
  echo "build it first, for example: cargo build -p clawd -p skill-runner -p extension-manager-skill --release" >&2
  exit 2
fi
if [[ ! -x "$ROOT_DIR/target/release/skill-runner" ]]; then
  echo "skill-runner release binary missing: $ROOT_DIR/target/release/skill-runner" >&2
  exit 2
fi
if [[ ! -x "$ROOT_DIR/target/release/extension-manager-skill" ]]; then
  echo "extension-manager release binary missing: $ROOT_DIR/target/release/extension-manager-skill" >&2
  exit 2
fi

(
  cd "$TEMP_WORKSPACE"
  WORKSPACE_ROOT="$TEMP_WORKSPACE" "$CLAWD_BIN"
) >"$TEMP_WORKSPACE/clawd.log" 2>&1 &
CLAWD_PID=$!

wait_for_health

echo "[1/6] scaffold external skill in isolated workspace"
scaffold_raw="$(submit_run_skill_and_wait "extension_manager" "$(jq -nc \
  --arg skill_name "$SKILL_NAME" \
  '{action:"scaffold_external_skill", skill_name:$skill_name, capability_summary:"Reply to ping with a short grounded success message.", actions:["ping"]}')" )"
skill_dir="$TEMP_WORKSPACE/external_skills/$SKILL_NAME"
[[ -f "$skill_dir/src/main.rs" ]] || { echo "missing scaffolded main.rs: $skill_dir/src/main.rs" >&2; exit 1; }
[[ ! -e "$ROOT_DIR/external_skills/$SKILL_NAME" ]] || { echo "unexpected repo-root pollution: $ROOT_DIR/external_skills/$SKILL_NAME" >&2; exit 1; }

echo "[2/6] validate scaffolded skill"
validate_raw="$(submit_run_skill_and_wait "extension_manager" "$(jq -nc \
  --arg skill_name "$SKILL_NAME" \
  '{action:"validate_external_skill", skill_name:$skill_name, actions:["ping"]}')" )"
echo "$validate_raw" | jq -e '.data.result_json.text | contains("cargo check ok")' >/dev/null

echo "[3/6] register external skill"
register_raw="$(submit_run_skill_and_wait "extension_manager" "$(jq -nc \
  --arg skill_name "$SKILL_NAME" \
  '{action:"register_external_skill", confirm:true, skill_name:$skill_name}')" )"
grep -q "\"external_skills/${SKILL_NAME}\"," "$TEMP_WORKSPACE/Cargo.toml"
grep -q "name = \"$SKILL_NAME\"" "$TEMP_WORKSPACE/configs/skills_registry.toml"
grep -q "${SKILL_NAME} = false" "$TEMP_WORKSPACE/configs/config.toml"

echo "[4/6] enable external skill"
enable_raw="$(submit_run_skill_and_wait "extension_manager" "$(jq -nc \
  --arg skill_name "$SKILL_NAME" \
  '{action:"enable_external_skill", confirm:true, skill_name:$skill_name}')" )"
echo "$enable_raw" | jq -e '.data.result_json.text | contains("Enabled external skill")' >/dev/null
grep -q "${SKILL_NAME} = true" "$TEMP_WORKSPACE/configs/config.toml"

echo "[5/6] reload skills"
reload_raw="$(reload_skills_and_wait)"
echo "$reload_raw" | jq -e '.ok == true' >/dev/null

echo "[6/6] run new skill"
final_raw="$(submit_run_skill_and_wait "$SKILL_NAME" '{"action":"ping"}')"
echo "$final_raw" | jq -e '.data.result_json.text == "TODO: implement ping"' >/dev/null

echo "PASS: self-extension runtime enable regression finished"
echo "workspace_root=${TEMP_WORKSPACE}"
echo "skill_name=${SKILL_NAME}"
