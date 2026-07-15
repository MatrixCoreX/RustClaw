#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"
export LC_ALL=C
export LANG=C

CASE_FILE="${CASE_FILE:-${ROOT_DIR}/scripts/nl_tests/cases/nl_cases_sensitive_flows.txt}"
WORKSPACE_ROOT_OVERRIDE="${WORKSPACE_ROOT_OVERRIDE:-}"
PORT="${PORT:-}"
WAIT_SECONDS="${WAIT_SECONDS:-120}"
POLL_INTERVAL="${POLL_INTERVAL:-1}"
ROUNDS="${ROUNDS:-2}"
KEEP_WORKSPACE="${KEEP_WORKSPACE:-0}"
CLAWD_BIN="${CLAWD_BIN:-}"
RUNTIME_ENV_FILE="${RUNTIME_ENV_FILE:-/home/guagua/runtime_env_filled.sh}"
AUTO_BUILD="${AUTO_BUILD:-0}"
LOG_DIR="${LOG_DIR:-}"

TEMP_WORKSPACE=""
CLAWD_PID=""
BASE_URL=""
ADMIN_USER_KEY=""
REGULAR_USER_KEY=""
CONFIG_BASELINE_SHA=""
BASE_ID_SEED=""
PASS=0
FAIL=0
SKIP=0

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" "$1"
}

usage() {
  cat <<'EOF'
Usage:
  bash scripts/regression_sensitive_nl_flows.sh [options]

Options:
  --case-file PATH         NL case file. Default: scripts/nl_tests/cases/nl_cases_sensitive_flows.txt
  --workspace-root DIR     Reuse a temp workspace instead of mktemp
  --log-dir DIR            Preserve logs under this directory
  --port PORT              clawd listen port
  --wait-seconds N         Max wait per task (default: 120)
  --rounds N               Repeat NL ask cases N rounds (default: 2)
  --keep-workspace         Do not remove temp workspace on exit
  --clawd-bin PATH         clawd binary path
  --runtime-env-file PATH  Shell file with provider env vars
  --auto-build             Build missing binaries automatically
  -h, --help               Show this help

Stages:
  1. Deterministic direct run_skill checks for config guard and crypto unbound prompts
  2. Multi-round NL ask checks for config mutation guard, crypto unbound hints, and self-extension handoff

Artifacts:
  scripts/nl_suite_logs/sensitive_flows/<timestamp>/
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
    --case-file)
      CASE_FILE="${2:-}"
      shift 2
      ;;
    --workspace-root)
      WORKSPACE_ROOT_OVERRIDE="${2:-}"
      shift 2
      ;;
    --log-dir)
      LOG_DIR="${2:-}"
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
    --rounds)
      ROUNDS="${2:-}"
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
    --auto-build)
      AUTO_BUILD=1
      shift 1
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
need_cmd mktemp
need_cmd python3
need_cmd shasum

[[ -f "$CASE_FILE" ]] || {
  echo "Case file not found: $CASE_FILE" >&2
  exit 2
}

if ! [[ "$WAIT_SECONDS" =~ ^[0-9]+$ ]] || [[ "$WAIT_SECONDS" -le 0 ]]; then
  echo "--wait-seconds must be a positive integer" >&2
  exit 2
fi
if ! [[ "$ROUNDS" =~ ^[0-9]+$ ]] || [[ "$ROUNDS" -le 0 ]]; then
  echo "--rounds must be a positive integer" >&2
  exit 2
fi

if [[ -z "$LOG_DIR" ]]; then
  LOG_DIR="${ROOT_DIR}/scripts/nl_suite_logs/sensitive_flows/$(date +%Y%m%d_%H%M%S)"
fi
mkdir -p "$LOG_DIR"
exec > >(tee -a "$LOG_DIR/run.log") 2>&1

pick_free_port() {
  python3 - <<'PY'
import socket

sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

make_base_seed() {
  python3 - <<'PY'
import time

seed = time.time_ns() % 900_000_000
print(1_900_000_000 + seed)
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

config_sha() {
  shasum -a 256 "$TEMP_WORKSPACE/configs/config.toml" | awk '{print $1}'
}

assert_temp_config_unchanged() {
  local case_name="$1"
  local current_sha
  current_sha="$(config_sha)"
  if [[ "$current_sha" != "$CONFIG_BASELINE_SHA" ]]; then
    echo "[FAIL] ${case_name} mutated temp configs/config.toml" >&2
    FAIL=$((FAIL + 1))
    return 1
  fi
  return 0
}

ensure_binaries() {
  if [[ -z "$CLAWD_BIN" ]]; then
    if [[ -x "$ROOT_DIR/target/debug/clawd" ]]; then
      CLAWD_BIN="$ROOT_DIR/target/debug/clawd"
    else
      CLAWD_BIN="$ROOT_DIR/target/release/clawd"
    fi
  fi

  local need_build=0
  [[ -x "$CLAWD_BIN" ]] || need_build=1
  [[ -x "$ROOT_DIR/target/release/skill-runner" ]] || need_build=1
  [[ -x "$ROOT_DIR/target/release/crypto-skill" ]] || need_build=1
  [[ -x "$ROOT_DIR/target/release/extension-manager-skill" ]] || need_build=1

  if [[ "$need_build" == "1" && "$AUTO_BUILD" == "1" ]]; then
    (cd "$ROOT_DIR" && cargo build -p clawd && cargo build --release -p skill-runner -p crypto-skill -p extension-manager-skill)
    if [[ -x "$ROOT_DIR/target/debug/clawd" ]]; then
      CLAWD_BIN="$ROOT_DIR/target/debug/clawd"
    fi
  fi

  [[ -x "$CLAWD_BIN" ]] || {
    echo "clawd binary not found or not executable: $CLAWD_BIN" >&2
    exit 2
  }
  [[ -x "$ROOT_DIR/target/release/skill-runner" ]] || {
    echo "skill-runner release binary missing: $ROOT_DIR/target/release/skill-runner" >&2
    exit 2
  }
  [[ -x "$ROOT_DIR/target/release/crypto-skill" ]] || {
    echo "crypto-skill release binary missing: $ROOT_DIR/target/release/crypto-skill" >&2
    exit 2
  }
  [[ -x "$ROOT_DIR/target/release/extension-manager-skill" ]] || {
    echo "extension-manager release binary missing: $ROOT_DIR/target/release/extension-manager-skill" >&2
    exit 2
  }
}

wait_for_health() {
  local waited=0
  while [[ "$waited" -le "$WAIT_SECONDS" ]]; do
    if curl -sS -H "X-RustClaw-Key: ${ADMIN_USER_KEY}" "${BASE_URL}/v1/health" >/dev/null 2>&1; then
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

extract_visible_text() {
  python3 - "$1" <<'PY'
import json
import sys

obj = json.loads(sys.argv[1])
data = obj.get("data") or {}
result = data.get("result_json") or {}
messages = result.get("messages") or []
parts = []
for candidate in (data.get("error_text"), result.get("text")):
    if isinstance(candidate, str) and candidate.strip():
        parts.append(candidate.strip())
for item in messages:
    if isinstance(item, str) and item.strip():
        parts.append(item.strip())
    elif isinstance(item, dict):
        text = item.get("text")
        if isinstance(text, str) and text.strip():
            parts.append(text.strip())
print("\n".join(parts))
PY
}

missing_substrings() {
  python3 - "$1" "$2" <<'PY'
import sys

text = sys.argv[1]
expected = [part for part in sys.argv[2].split(";;") if part]
missing = [needle for needle in expected if needle not in text]
if missing:
    print("\n".join(missing))
    raise SystemExit(1)
PY
}

write_case_artifacts() {
  local stage="$1"
  local round_no="$2"
  local case_name="$3"
  local prompt="$4"
  local submit_raw="$5"
  local final_raw="$6"
  local case_dir="$LOG_DIR/cases/${stage}_round${round_no}_${case_name}"
  mkdir -p "$case_dir"
  printf '%s\n' "$prompt" > "$case_dir/prompt.txt"
  printf '%s\n' "$submit_raw" > "$case_dir/submit.json"
  printf '%s\n' "$final_raw" > "$case_dir/final.json"
}

append_summary() {
  local stage="$1"
  local round_no="$2"
  local case_name="$3"
  local auth_kind="$4"
  local result_kind="$5"
  local note="$6"
  python3 - "$LOG_DIR/summary.jsonl" "$stage" "$round_no" "$case_name" "$auth_kind" "$result_kind" "$note" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
obj = {
    "stage": sys.argv[2],
    "round": int(sys.argv[3]),
    "case_name": sys.argv[4],
    "auth": sys.argv[5],
    "result": sys.argv[6],
    "note": sys.argv[7],
}
path.parent.mkdir(parents=True, exist_ok=True)
with path.open("a", encoding="utf-8") as fh:
    fh.write(json.dumps(obj, ensure_ascii=False) + "\n")
PY
}

case_user_ids() {
  local round_no="$1"
  local ordinal="$2"
  python3 - "$BASE_ID_SEED" "$round_no" "$ordinal" <<'PY'
import sys

base = int(sys.argv[1])
round_no = int(sys.argv[2])
ordinal = int(sys.argv[3])
offset = round_no * 1000 + ordinal
print(base + offset, base + offset + 1000000)
PY
}

load_case_rows() {
  local case_file="$1"
  python3 - "$case_file" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
for idx, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = [part.strip() for part in line.split("|", 3)]
    if len(parts) != 4:
        raise SystemExit(f"invalid case format on line {idx}: {raw}")
    name, auth, expected, prompt = parts
    print(f"{idx}\x1f{name}\x1f{auth}\x1f{expected}\x1f{prompt}")
PY
}

extract_task_status() {
  printf '%s\n' "$1" | jq -r '.data.status // ""'
}

check_self_extension_route() {
  local raw="$1"
  printf '%s\n' "$raw" | jq -e '
    .data.result_json.task_journal.summary.route_result.self_extension.mode == "permanent_extension"
    and .data.result_json.task_journal.summary.route_result.self_extension.trigger == "explicit_user_request"
  ' >/dev/null
}

run_direct_case() {
  local case_name="$1"
  local args_json="$2"
  local expected="$3"
  local skill_name="$4"

  USER_KEY="$REGULAR_USER_KEY"
  read -r USER_ID CHAT_ID < <(case_user_ids 0 "$((PASS + FAIL + SKIP + 1))")
  export USER_KEY USER_ID CHAT_ID

  echo "[direct] ${case_name}"
  local submit_raw task_id final_raw visible_text missing
  submit_raw="$(submit_run_skill_task "$skill_name" "$args_json")"
  task_id="$(extract_submit_task_id "$submit_raw")"
  final_raw="$(wait_task_until_terminal_with_limit "$task_id" "$WAIT_SECONDS")"
  write_case_artifacts "direct" 0 "$case_name" "$args_json" "$submit_raw" "$final_raw"
  visible_text="$(extract_visible_text "$final_raw")"
  if missing="$(missing_substrings "$visible_text" "$expected" 2>&1)"; then
    echo "[PASS] ${case_name}"
    PASS=$((PASS + 1))
    append_summary "direct" 0 "$case_name" "user" "pass" ""
  else
    echo "[FAIL] ${case_name}: missing -> ${missing}"
    FAIL=$((FAIL + 1))
    append_summary "direct" 0 "$case_name" "user" "fail" "$missing"
  fi

  if [[ "$case_name" == config_guard_* ]]; then
    assert_temp_config_unchanged "$case_name" || true
  fi
}

run_nl_case() {
  local round_no="$1"
  local ordinal="$2"
  local case_name="$3"
  local auth_kind="$4"
  local expected="$5"
  local prompt="$6"

  case "$auth_kind" in
    admin) USER_KEY="$ADMIN_USER_KEY" ;;
    user) USER_KEY="$REGULAR_USER_KEY" ;;
    *)
      echo "unsupported auth kind in case ${case_name}: ${auth_kind}" >&2
      FAIL=$((FAIL + 1))
      append_summary "ask" "$round_no" "$case_name" "$auth_kind" "fail" "bad auth kind"
      return
      ;;
  esac
  read -r USER_ID CHAT_ID < <(case_user_ids "$round_no" "$ordinal")
  export USER_KEY USER_ID CHAT_ID

  echo "[ask][round ${round_no}] ${case_name}"
  local submit_raw task_id final_raw status visible_text missing
  submit_raw="$(submit_task "$prompt")"
  task_id="$(extract_submit_task_id "$submit_raw")"
  final_raw="$(wait_task_until_terminal_with_limit "$task_id" "$WAIT_SECONDS")"
  write_case_artifacts "ask" "$round_no" "$case_name" "$prompt" "$submit_raw" "$final_raw"
  status="$(extract_task_status "$final_raw")"

  if result_provider_unavailable "$final_raw"; then
    echo "[SKIP] ${case_name}: provider unavailable"
    SKIP=$((SKIP + 1))
    append_summary "ask" "$round_no" "$case_name" "$auth_kind" "skip" "provider unavailable"
    return
  fi

  visible_text="$(extract_visible_text "$final_raw")"
  if missing="$(missing_substrings "$visible_text" "$expected" 2>&1)"; then
    if [[ "$case_name" == self_extension_* ]] && ! check_self_extension_route "$final_raw"; then
      echo "[FAIL] ${case_name}: self_extension route_result mismatch"
      FAIL=$((FAIL + 1))
      append_summary "ask" "$round_no" "$case_name" "$auth_kind" "fail" "self_extension route_result mismatch"
      return
    fi
    echo "[PASS] ${case_name} (status=${status})"
    PASS=$((PASS + 1))
    append_summary "ask" "$round_no" "$case_name" "$auth_kind" "pass" "status=${status}"
  else
    echo "[FAIL] ${case_name}: missing -> ${missing}"
    FAIL=$((FAIL + 1))
    append_summary "ask" "$round_no" "$case_name" "$auth_kind" "fail" "$missing"
  fi

  if [[ "$case_name" == config_guard_* ]]; then
    assert_temp_config_unchanged "$case_name" || true
  fi
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
  echo "log_dir_ref=$(path_ref "${LOG_DIR}")"
  if [[ "$KEEP_WORKSPACE" == "1" && -n "$TEMP_WORKSPACE" ]]; then
    echo "workspace_root_ref=$(path_ref "${TEMP_WORKSPACE}")"
  fi
  exit "$exit_code"
}
trap cleanup EXIT

ensure_binaries

if [[ -z "$PORT" ]]; then
  PORT="$(pick_free_port)"
fi
BASE_ID_SEED="$(make_base_seed)"

if [[ -n "$WORKSPACE_ROOT_OVERRIDE" ]]; then
  TEMP_WORKSPACE="$WORKSPACE_ROOT_OVERRIDE"
else
  TEMP_WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/rustclaw-sensitive-nl-XXXXXX")"
fi

prepare_temp_workspace "$TEMP_WORKSPACE"
patch_temp_config \
  "$TEMP_WORKSPACE/configs/config.toml" \
  "$PORT" \
  "$TEMP_WORKSPACE/data/sensitive_nl.sqlite"
CONFIG_BASELINE_SHA="$(config_sha)"

BASE_URL="http://127.0.0.1:${PORT}"
export BASE_URL

ADMIN_USER_KEY="$(
  RUSTCLAW_CONFIG_PATH="$TEMP_WORKSPACE/configs/config.toml" \
    bash "$ROOT_DIR/scripts/auth-key.sh" generate admin | awk '{print $1; exit}'
)"
REGULAR_USER_KEY="$(
  RUSTCLAW_CONFIG_PATH="$TEMP_WORKSPACE/configs/config.toml" \
    bash "$ROOT_DIR/scripts/auth-key.sh" generate user | awk '{print $1; exit}'
)"

(
  cd "$TEMP_WORKSPACE"
  if [[ -f "$RUNTIME_ENV_FILE" ]]; then
    # shellcheck source=/dev/null
    source "$RUNTIME_ENV_FILE"
  fi
  WORKSPACE_ROOT="$TEMP_WORKSPACE" "$CLAWD_BIN"
) >"$LOG_DIR/clawd.log" 2>&1 &
CLAWD_PID=$!

wait_for_health

printf 'workspace_root=%s\nbase_url=%s\nadmin_key=%s\nuser_key=%s\nrounds=%s\ncase_file=%s\n' \
  "$TEMP_WORKSPACE" "$BASE_URL" "$ADMIN_USER_KEY" "$REGULAR_USER_KEY" "$ROUNDS" "$CASE_FILE" > "$LOG_DIR/meta.txt"

echo "== Stage 1: local direct checks =="
run_direct_case \
  "config_guard_direct_en" \
  '{"path":"configs/config.toml","content":"forbidden","request_text":"Please modify configs/config.toml and apply the change directly."}' \
  'Web admin console' \
  "write_file"
run_direct_case \
  "config_guard_direct_zh" \
  '{"path":"configs/config.toml","content":"forbidden","request_text":"把 configs/config.toml 直接改掉。"}' \
  'Web 管理端' \
  "write_file"
run_direct_case \
  "crypto_binance_direct_unbound" \
  '{"action":"positions","exchange":"binance"}' \
  '当前 key 还没有绑定 Binance API;;/cryptoapi set binance' \
  "crypto"
run_direct_case \
  "crypto_okx_direct_unbound" \
  '{"action":"positions","exchange":"okx"}' \
  '当前 key 还没有绑定 OKX API;;/cryptoapi set okx' \
  "crypto"

echo "== Stage 2: NL ask checks =="
for round_no in $(seq 1 "$ROUNDS"); do
  ordinal=0
  while IFS=$'\x1f' read -r _line_no case_name auth_kind expected prompt; do
    ordinal=$((ordinal + 1))
    run_nl_case "$round_no" "$ordinal" "$case_name" "$auth_kind" "$expected" "$prompt"
  done < <(load_case_rows "$CASE_FILE")
done

echo "== Summary =="
echo "PASS=${PASS}"
echo "FAIL=${FAIL}"
echo "SKIP=${SKIP}"

if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
if [[ "$SKIP" -gt 0 ]]; then
  exit 2
fi
exit 0
