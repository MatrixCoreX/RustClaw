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
CLAWD_BIN="${CLAWD_BIN:-}"
RUNTIME_ENV_FILE="${RUNTIME_ENV_FILE:-/home/guagua/runtime_env_filled.sh}"

TEMP_WORKSPACE=""
CLAWD_PID=""
BASE_URL=""
USER_KEY=""
USER_ID=""
CHAT_ID=""
CASE_INDEX=0
CASE_TOTAL=2

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" "$1"
}

usage() {
  cat <<'EOF'
Usage:
  bash scripts/regression_self_extension_nl_handoff.sh [--workspace-root DIR] [--port PORT] [--wait-seconds N] [--keep-workspace] [--clawd-bin PATH] [--runtime-env-file PATH]

This regression starts an isolated clawd instance with self_extension enabled
but permanent materialization disabled, then submits one natural-language ask
that explicitly requests a new reusable skill instead of existing skills.

Expected when provider is available:
  - each task succeeds
  - task_journal.summary.route_result.self_extension.mode == permanent_extension
  - trigger == explicit_user_request
  - English prompt returns the English permanent-plan reply
  - Chinese prompt returns the Chinese permanent-plan reply

If the upstream provider is unavailable, the script exits 2 (skip) instead of
reporting a product regression.
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
    --runtime-env-file)
      RUNTIME_ENV_FILE="${2:-}"
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
base = 1_800_000_000 + seed
print(base, base + 1)
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
text = replace_once(r'^enabled\s*=\s*(true|false)$', 'enabled = true', text)
text = replace_once(r'^auto_on_capability_gap\s*=\s*(true|false)$', 'auto_on_capability_gap = false', text)
text = replace_once(r'^allow_execute\s*=\s*(true|false)$', 'allow_execute = false', text)
text = replace_once(r'^allow_package_install\s*=\s*(true|false)$', 'allow_package_install = false', text)
text = replace_once(r'^allow_permanent_extension\s*=\s*(true|false)$', 'allow_permanent_extension = false', text)
text = replace_once(r'^allow_runtime_enable\s*=\s*(true|false)$', 'allow_runtime_enable = false', text)

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

result_provider_unavailable() {
  python3 - "$1" <<'PY'
import json
import re
import sys

obj = json.loads(sys.argv[1])
data = obj.get("data") or {}
result = data.get("result_json") or {}
messages = result.get("messages") or []
parts = [
    str(data.get("error_text") or ""),
    str(result.get("text") or ""),
]
for item in messages:
    if isinstance(item, dict):
        parts.append(str(item.get("text") or ""))
    elif isinstance(item, str):
        parts.append(item)
text = "\n".join(part.strip().lower() for part in parts if str(part).strip())
markers = [
    "当前大模型服务暂时不可用",
    "selected model is at capacity",
    "usage limit exceeded",
    "rate limit",
    "rate_limit",
    "too many requests",
    "http 429",
    "http 529",
    "529 overloaded",
    "missing choices[0].message.content",
]
provider_like = any(marker in text for marker in markers)
provider_like = provider_like or (
    "provider=vendor-" in text
    and (
        re.search(r"http 5\d\d", text) is not None
        or '"type":"server_error"' in text
        or "unknown error, 520" in text
    )
)
raise SystemExit(0 if provider_like else 1)
PY
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
  TEMP_WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/rustclaw-selfext-nl-XXXXXX")"
fi

prepare_temp_workspace "$TEMP_WORKSPACE"
patch_temp_config \
  "$TEMP_WORKSPACE/configs/config.toml" \
  "$PORT" \
  "$TEMP_WORKSPACE/data/self_extension_nl_handoff.sqlite"

read -r USER_ID CHAT_ID < <(make_seed_triplet)
BASE_URL="http://127.0.0.1:${PORT}"
export BASE_URL USER_ID CHAT_ID

USER_KEY="$(
  RUSTCLAW_CONFIG_PATH="$TEMP_WORKSPACE/configs/config.toml" \
    bash "$ROOT_DIR/scripts/auth-key.sh" generate admin | awk '{print $1; exit}'
)"
export USER_KEY

if [[ -z "$CLAWD_BIN" ]]; then
  if [[ -x "$ROOT_DIR/target/debug/clawd" ]]; then
    CLAWD_BIN="$ROOT_DIR/target/debug/clawd"
  else
    CLAWD_BIN="$ROOT_DIR/target/release/clawd"
  fi
fi

if [[ ! -x "$CLAWD_BIN" ]]; then
  echo "clawd binary not found or not executable: $CLAWD_BIN" >&2
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
  if [[ -f "$RUNTIME_ENV_FILE" ]]; then
    # shellcheck source=/dev/null
    source "$RUNTIME_ENV_FILE"
  fi
  WORKSPACE_ROOT="$TEMP_WORKSPACE" "$CLAWD_BIN"
) >"$TEMP_WORKSPACE/clawd.log" 2>&1 &
CLAWD_PID=$!

wait_for_health

run_nl_handoff_case() {
  local case_name="$1"
  local prompt="$2"
  local expected_substring="$3"

  CASE_INDEX=$((CASE_INDEX + 1))
  echo "[${CASE_INDEX}/${CASE_TOTAL}] submit ${case_name}"
  local submit_raw
  submit_raw="$(submit_task "$prompt")"
  local task_id
  task_id="$(extract_submit_task_id "$submit_raw")"

  echo "    wait for terminal result and verify ${case_name}"
  local final_raw
  final_raw="$(wait_task_until_terminal_with_limit "$task_id" "$WAIT_SECONDS")"
  local status
  status="$(echo "$final_raw" | jq -r '.data.status // ""')"
  if [[ "$status" != "succeeded" ]]; then
    if result_provider_unavailable "$final_raw"; then
      echo "provider unavailable during natural-language self-extension handoff (${case_name}); skip regression"
      exit 2
    fi
    echo "unexpected task status for ${case_name}: $status" >&2
    echo "$final_raw" >&2
    exit 1
  fi

  if result_provider_unavailable "$final_raw"; then
    echo "provider unavailable during natural-language self-extension handoff (${case_name}); skip regression"
    exit 2
  fi

  echo "$final_raw" | jq -e '
    .data.result_json.task_journal.summary.route_result.self_extension.mode == "permanent_extension"
  ' >/dev/null
  echo "$final_raw" | jq -e '
    .data.result_json.task_journal.summary.route_result.self_extension.trigger == "explicit_user_request"
  ' >/dev/null
  echo "$final_raw" | jq -e --arg expected "$expected_substring" '
    (.data.result_json.text // "") | contains($expected)
  ' >/dev/null

  echo "    PASS: ${case_name} task_id=${task_id}"
}

PROMPT_EN="Do not use any existing skill. Instead, plan a brand-new reusable skill for this capability: when called with action ping, reply with a short success message. Do not execute or enable anything yet; just prepare the reusable skill plan."
PROMPT_ZH="不要使用任何现有技能。请先为这个能力规划一个全新的可复用技能：调用 action ping 时，返回一句简短的成功提示。先不要执行，也不要启用，只生成可复用技能方案。"

run_nl_handoff_case "english_prompt" "$PROMPT_EN" "external-skill scaffold plan"
run_nl_handoff_case "chinese_prompt" "$PROMPT_ZH" "外部技能脚手架方案"

echo "PASS: self-extension natural-language handoff regression finished"
echo "workspace_root_ref=$(path_ref "${TEMP_WORKSPACE}")"
