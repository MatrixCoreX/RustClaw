#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"
export LC_ALL=C
export LANG=C

CASE_FILE="${CASE_FILE:-${ROOT_DIR}/scripts/nl_tests/cases/nl_cases_long_tail_flows.txt}"
WORKSPACE_ROOT_OVERRIDE="${WORKSPACE_ROOT_OVERRIDE:-}"
PORT="${PORT:-}"
HTTP_PORT="${HTTP_PORT:-}"
HTTP_REPAIR_PORT="${HTTP_REPAIR_PORT:-}"
WAIT_SECONDS="${WAIT_SECONDS:-180}"
POLL_INTERVAL="${POLL_INTERVAL:-1}"
ROUNDS="${ROUNDS:-1}"
KEEP_WORKSPACE="${KEEP_WORKSPACE:-0}"
CLAWD_BIN="${CLAWD_BIN:-}"
RUNTIME_ENV_FILE="${RUNTIME_ENV_FILE:-/home/guagua/runtime_env_filled.sh}"
AUTO_BUILD="${AUTO_BUILD:-1}"
LOG_DIR="${LOG_DIR:-}"

TEMP_WORKSPACE=""
CLAWD_PID=""
BASE_URL=""
ADMIN_USER_KEY=""
REGULAR_USER_KEY=""
BASE_ID_SEED=""
HTTP_MARKER="ops-demo-ok"
HTTP_DIR_REL="document/nl_ops_http_demo"
HTTP_INDEX_REL="${HTTP_DIR_REL}/index.html"
REPAIR_HTTP_MARKER="ops-repair-ok"
REPAIR_HTTP_BAD_MARKER="ops-repair-bad"
REPAIR_HTTP_DIR_REL="document/nl_ops_http_repair_demo"
REPAIR_HTTP_INDEX_REL="${REPAIR_HTTP_DIR_REL}/index.html"
PASS=0
FAIL=0
SKIP=0

usage() {
  cat <<'EOF'
Usage:
  bash scripts/regression_long_tail_nl_flows.sh [options]

Options:
  --case-file PATH         NL case file. Default: scripts/nl_tests/cases/nl_cases_long_tail_flows.txt
  --workspace-root DIR     Reuse a temp workspace instead of mktemp
  --log-dir DIR            Preserve logs under this directory
  --port PORT              clawd listen port
  --http-port PORT         Temporary local HTTP demo port
  --http-repair-port PORT  Temporary local HTTP repair demo port
  --wait-seconds N         Max wait per task (default: 180)
  --rounds N               Repeat NL ask cases N rounds (default: 1)
  --keep-workspace         Do not remove temp workspace on exit
  --clawd-bin PATH         clawd binary path
  --runtime-env-file PATH  Shell file with provider env vars
  --auto-build             Build missing binaries automatically
  -h, --help               Show this help

Stages:
  1. Start an isolated temp workspace
  2. Run NL ask checks for health_check OS-only summaries
  3. Run NL ask checks for ops_closed_loop HTTP start-and-validate flows

Artifacts:
  scripts/nl_suite_logs/long_tail_flows/<timestamp>/
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
    --http-port)
      HTTP_PORT="${2:-}"
      shift 2
      ;;
    --http-repair-port)
      HTTP_REPAIR_PORT="${2:-}"
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
need_cmd lsof
need_cmd mktemp
need_cmd python3

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
  LOG_DIR="${ROOT_DIR}/scripts/nl_suite_logs/long_tail_flows/$(date +%Y%m%d_%H%M%S)"
fi
if [[ "$LOG_DIR" != /* ]]; then
  LOG_DIR="${ROOT_DIR}/${LOG_DIR}"
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
print(1_950_000_000 + seed)
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
  mkdir -p "$workspace_root/data" "$workspace_root/document" "$workspace_root/external_skills"
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

text = replace_once(r'^listen\s*=\s*".*"$', f'listen = "127.0.0.1:{port}"', text)
text = replace_once(r'^sqlite_path\s*=\s*".*"$', f'sqlite_path = "{sqlite_path}"', text)
text = replace_once(r'^auto_on_capability_gap\s*=\s*(true|false)$', 'auto_on_capability_gap = false', text)
text = replace_once(r'^allow_execute\s*=\s*(true|false)$', 'allow_execute = false', text)
text = replace_once(r'^allow_package_install\s*=\s*(true|false)$', 'allow_package_install = false', text)
text = replace_once(r'^allow_permanent_extension\s*=\s*(true|false)$', 'allow_permanent_extension = false', text)
text = replace_once(r'^allow_runtime_enable\s*=\s*(true|false)$', 'allow_runtime_enable = false', text)

config_path.write_text(text, encoding="utf-8")
PY
}

prepare_http_demo() {
  local workspace_root="$1"
  mkdir -p "$workspace_root/$HTTP_DIR_REL"
  printf '%s\n' "$HTTP_MARKER" > "$workspace_root/$HTTP_INDEX_REL"
}

prepare_http_repair_demo() {
  local workspace_root="$1"
  mkdir -p "$workspace_root/$REPAIR_HTTP_DIR_REL"
  printf '%s\n' "$REPAIR_HTTP_BAD_MARKER" > "$workspace_root/$REPAIR_HTTP_INDEX_REL"
}

binary_is_stale() {
  python3 - "$@" <<'PY'
from pathlib import Path
import sys

binary = Path(sys.argv[1])
roots = [Path(arg) for arg in sys.argv[2:]]
if not binary.exists():
    print("1")
    raise SystemExit(0)

try:
    binary_mtime = binary.stat().st_mtime
except OSError:
    print("1")
    raise SystemExit(0)

latest_source_mtime = 0.0
for root in roots:
    if not root.exists():
        continue
    candidates = [root] if root.is_file() else root.rglob("*")
    for path in candidates:
        try:
            if not path.is_file():
                continue
            latest_source_mtime = max(latest_source_mtime, path.stat().st_mtime)
        except OSError:
            continue

print("1" if latest_source_mtime > binary_mtime else "0")
PY
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
  local stale=0
  local clawd_inputs=(
    "$ROOT_DIR/Cargo.toml"
    "$ROOT_DIR/Cargo.lock"
    "$ROOT_DIR/configs"
    "$ROOT_DIR/prompts"
    "$ROOT_DIR/crates/clawd"
    "$ROOT_DIR/crates/claw-core"
  )
  local skill_runner_inputs=(
    "$ROOT_DIR/Cargo.toml"
    "$ROOT_DIR/Cargo.lock"
    "$ROOT_DIR/crates/skill-runner"
  )
  local health_check_inputs=(
    "$ROOT_DIR/Cargo.toml"
    "$ROOT_DIR/Cargo.lock"
    "$ROOT_DIR/configs"
    "$ROOT_DIR/prompts"
    "$ROOT_DIR/crates/skills/health_check"
  )
  [[ -x "$CLAWD_BIN" ]] || need_build=1
  [[ -x "$ROOT_DIR/target/release/skill-runner" ]] || need_build=1
  [[ -x "$ROOT_DIR/target/release/health-check-skill" ]] || need_build=1
  if [[ "$AUTO_BUILD" == "1" ]]; then
    stale="$(binary_is_stale "$CLAWD_BIN" "${clawd_inputs[@]}")"
    [[ "$stale" == "1" ]] && need_build=1
    stale="$(binary_is_stale "$ROOT_DIR/target/release/skill-runner" "${skill_runner_inputs[@]}")"
    [[ "$stale" == "1" ]] && need_build=1
    stale="$(binary_is_stale "$ROOT_DIR/target/release/health-check-skill" "${health_check_inputs[@]}")"
    [[ "$stale" == "1" ]] && need_build=1
  fi

  if [[ "$need_build" == "1" && "$AUTO_BUILD" == "1" ]]; then
    echo "building fresh binaries for long-tail NL regression"
    (cd "$ROOT_DIR" && cargo build -p clawd && cargo build --release -p skill-runner -p health-check-skill)
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
  [[ -x "$ROOT_DIR/target/release/health-check-skill" ]] || {
    echo "health-check-skill release binary missing: $ROOT_DIR/target/release/health-check-skill" >&2
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

wait_for_http_server() {
  local url="$1"
  local waited=0
  local max_wait=15
  while [[ "$waited" -le "$max_wait" ]]; do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
    waited=$((waited + 1))
  done
  echo "http server timeout: ${url}" >&2
  return 1
}

kill_process_on_port() {
  local port="$1"
  local pids
  pids="$(lsof -ti "tcp:${port}" 2>/dev/null || true)"
  if [[ -n "$pids" ]]; then
    kill $pids >/dev/null 2>&1 || true
    sleep 1
    local survivors
    survivors="$(lsof -ti "tcp:${port}" 2>/dev/null || true)"
    if [[ -n "$survivors" ]]; then
      kill -9 $survivors >/dev/null 2>&1 || true
    fi
  fi
}

start_http_server_from_dir() {
  local dir="$1"
  local port="$2"
  local log_file="$3"
  (
    cd "$dir"
    python3 -m http.server "$port" --bind 127.0.0.1 >"$log_file" 2>&1 &
  )
  wait_for_http_server "http://127.0.0.1:${port}/"
}

prime_broken_http_repair_demo_server() {
  local workspace_root="$1"
  local round_no="$2"
  local case_name="$3"
  local repair_dir="$workspace_root/$REPAIR_HTTP_DIR_REL"
  local seed_log="$LOG_DIR/http_repair_seed_round${round_no}_${case_name}.log"
  printf '%s\n' "$REPAIR_HTTP_BAD_MARKER" > "$repair_dir/index.html"
  kill_process_on_port "$HTTP_REPAIR_PORT"
  start_http_server_from_dir "$repair_dir" "$HTTP_REPAIR_PORT" "$seed_log"
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
missing = []
for needle in expected:
    alternatives = [alt.strip() for alt in needle.split("__OR__") if alt.strip()]
    if alternatives and not any(alt in text for alt in alternatives):
        missing.append(needle)
if missing:
    print("\n".join(missing))
    raise SystemExit(1)
PY
}

expand_template() {
  python3 - \
    "$1" \
    "$HTTP_PORT" \
    "$HTTP_MARKER" \
    "$HTTP_DIR_REL" \
    "$HTTP_REPAIR_PORT" \
    "$REPAIR_HTTP_MARKER" \
    "$REPAIR_HTTP_DIR_REL" \
    "$REPAIR_HTTP_BAD_MARKER" <<'PY'
import sys

text = sys.argv[1]
replacements = {
    "{{HTTP_PORT}}": sys.argv[2],
    "{{HTTP_MARKER}}": sys.argv[3],
    "{{HTTP_DIR}}": sys.argv[4],
    "{{REPAIR_HTTP_PORT}}": sys.argv[5],
    "{{REPAIR_HTTP_MARKER}}": sys.argv[6],
    "{{REPAIR_HTTP_DIR}}": sys.argv[7],
    "{{REPAIR_HTTP_BAD_MARKER}}": sys.argv[8],
}
for key, value in replacements.items():
    text = text.replace(key, value)
print(text)
PY
}

ops_http_repair_summary() {
  python3 - "$1" "$2" <<'PY'
import json
import sys

obj = json.loads(sys.argv[1])
expected = sys.argv[2]
expected_parts = [part.strip() for part in expected.split(";;") if part.strip()]
data = obj.get("data") or {}
result = data.get("result_json") or {}
messages = result.get("messages") or []
visible_parts = []
for candidate in (data.get("error_text"), result.get("text")):
    if isinstance(candidate, str) and candidate.strip():
        visible_parts.append(candidate.strip())
for item in messages:
    if isinstance(item, str) and item.strip():
        visible_parts.append(item.strip())
    elif isinstance(item, dict):
        text = item.get("text")
        if isinstance(text, str) and text.strip():
            visible_parts.append(text.strip())
visible_text = "\n".join(visible_parts)

trace = ((result.get("task_journal") or {}).get("trace") or {})
rounds = trace.get("rounds") or []

def is_mutating_run_cmd(command: str) -> bool:
    lower = command.lower()
    markers = [
        ">",
        " tee ",
        "sed -i",
        "python -c",
        "python3 -c",
        "perl -0pi",
        "perl -pi",
        "printf ",
        "echo ",
        "cat <<",
        "cp ",
        "mv ",
    ]
    return any(marker in lower for marker in markers)

repair_round = False
repair_mutation = False
for idx, round_entry in enumerate(rounds):
    plan = round_entry.get("plan_result") or {}
    goal = str(plan.get("goal") or "")
    plan_kind = str(plan.get("plan_kind") or "")
    in_repair = "current_phase=repair" in goal or plan_kind == "Repair"
    in_apply_after_first_check = idx > 0 and "current_phase=apply" in goal
    treat_as_repair_round = in_repair or in_apply_after_first_check
    if treat_as_repair_round:
        repair_round = True
    for step in plan.get("steps") or []:
        skill = str(step.get("skill") or "")
        args = step.get("args") or {}
        if not treat_as_repair_round:
            continue
        if skill in {"write_file", "make_dir", "remove_file"}:
            repair_mutation = True
            break
        if skill == "service_control":
            action = str(args.get("action") or "").strip().lower()
            if action in {"start", "stop", "restart", "reload", "enable", "disable"}:
                repair_mutation = True
                break
        if skill == "run_cmd":
            command = str(args.get("command") or "")
            if is_mutating_run_cmd(command):
                repair_mutation = True
                break
    if repair_mutation:
        break

status = str(data.get("status") or "")
missing = []
if status != "succeeded":
    missing.append(f"status={status}")
for part in expected_parts:
    alternatives = [alt.strip() for alt in part.split("__OR__") if alt.strip()]
    if alternatives and not any(alt in visible_text for alt in alternatives):
        missing.append(f"visible_text_missing={part}")
if not repair_round:
    missing.append("repair_round_missing")
if not repair_mutation:
    missing.append("repair_mutation_missing")
if missing:
    print("\n".join(missing))
    raise SystemExit(1)
print(
    f"status={status}; rounds={len(rounds)}; repair_round=true; repair_mutation=true"
)
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
  local round_no="$1"
  local case_name="$2"
  local auth_kind="$3"
  local assertion="$4"
  local result_kind="$5"
  local note="$6"
  python3 - "$LOG_DIR/summary.jsonl" "$round_no" "$case_name" "$auth_kind" "$assertion" "$result_kind" "$note" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
obj = {
    "round": int(sys.argv[2]),
    "case_name": sys.argv[3],
    "auth": sys.argv[4],
    "assertion": sys.argv[5],
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
    parts = [part.strip() for part in line.split("|", 4)]
    if len(parts) != 5:
        raise SystemExit(f"invalid case format on line {idx}: {raw}")
    name, auth, assertion, expected, prompt = parts
    print(f"{idx}\x1f{name}\x1f{auth}\x1f{assertion}\x1f{expected}\x1f{prompt}")
PY
}

extract_task_status() {
  printf '%s\n' "$1" | jq -r '.data.status // ""'
}

run_nl_case() {
  local round_no="$1"
  local ordinal="$2"
  local case_name="$3"
  local auth_kind="$4"
  local assertion="$5"
  local expected_template="$6"
  local prompt_template="$7"

  local expected prompt
  expected="$(expand_template "$expected_template")"
  prompt="$(expand_template "$prompt_template")"

  case "$auth_kind" in
    admin) USER_KEY="$ADMIN_USER_KEY" ;;
    user) USER_KEY="$REGULAR_USER_KEY" ;;
    *)
      echo "unsupported auth kind in case ${case_name}: ${auth_kind}" >&2
      FAIL=$((FAIL + 1))
      append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "bad auth kind"
      return
      ;;
  esac
  read -r USER_ID CHAT_ID < <(case_user_ids "$round_no" "$ordinal")
  export USER_KEY USER_ID CHAT_ID

  if [[ "$assertion" == "ops_http" ]]; then
    kill_process_on_port "$HTTP_PORT"
  elif [[ "$assertion" == "ops_http_repair" ]]; then
    prime_broken_http_repair_demo_server "$TEMP_WORKSPACE" "$round_no" "$case_name"
  fi

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
    append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "skip" "provider unavailable"
    return
  fi

  visible_text="$(extract_visible_text "$final_raw")"
  case "$assertion" in
    text)
      if missing="$(missing_substrings "$visible_text" "$expected" 2>&1)"; then
        echo "[PASS] ${case_name} (status=${status})"
        PASS=$((PASS + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "pass" "status=${status}"
      else
        echo "[FAIL] ${case_name}: missing -> ${missing}"
        FAIL=$((FAIL + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "$missing"
      fi
      ;;
    ops_http)
      if [[ "$status" == "succeeded" ]] && missing="$(missing_substrings "$visible_text" "$expected" 2>&1)"; then
        echo "[PASS] ${case_name} (status=${status})"
        PASS=$((PASS + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "pass" "status=${status}"
      else
        if [[ "$status" != "succeeded" ]]; then
          missing="status=${status}"
        fi
        echo "[FAIL] ${case_name}: ${missing}"
        FAIL=$((FAIL + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "$missing"
      fi
      kill_process_on_port "$HTTP_PORT"
      ;;
    ops_http_repair)
      if missing="$(ops_http_repair_summary "$final_raw" "$expected" 2>&1)"; then
        echo "[PASS] ${case_name} (${missing})"
        PASS=$((PASS + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "pass" "$missing"
      else
        echo "[FAIL] ${case_name}: ${missing}"
        FAIL=$((FAIL + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "$missing"
      fi
      kill_process_on_port "$HTTP_REPAIR_PORT"
      ;;
    *)
      echo "unsupported assertion kind in case ${case_name}: ${assertion}" >&2
      FAIL=$((FAIL + 1))
      append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "bad assertion kind"
      ;;
  esac
}

cleanup() {
  local exit_code=$?
  if [[ -n "${HTTP_PORT:-}" ]]; then
    kill_process_on_port "$HTTP_PORT" || true
  fi
  if [[ -n "${HTTP_REPAIR_PORT:-}" ]]; then
    kill_process_on_port "$HTTP_REPAIR_PORT" || true
  fi
  if [[ -n "$CLAWD_PID" ]] && kill -0 "$CLAWD_PID" >/dev/null 2>&1; then
    kill "$CLAWD_PID" >/dev/null 2>&1 || true
    wait "$CLAWD_PID" >/dev/null 2>&1 || true
  fi
  if [[ "$KEEP_WORKSPACE" != "1" && -n "$TEMP_WORKSPACE" && -d "$TEMP_WORKSPACE" ]]; then
    rm -rf "$TEMP_WORKSPACE"
  fi
  echo "log_dir=${LOG_DIR}"
  if [[ "$KEEP_WORKSPACE" == "1" && -n "$TEMP_WORKSPACE" ]]; then
    echo "workspace_root=${TEMP_WORKSPACE}"
  fi
  exit "$exit_code"
}
trap cleanup EXIT

ensure_binaries

if [[ -z "$PORT" ]]; then
  PORT="$(pick_free_port)"
fi
if [[ -z "$HTTP_PORT" ]]; then
  HTTP_PORT="$(pick_free_port)"
fi
if [[ -z "$HTTP_REPAIR_PORT" ]]; then
  HTTP_REPAIR_PORT="$(pick_free_port)"
fi
BASE_ID_SEED="$(make_base_seed)"

if [[ -n "$WORKSPACE_ROOT_OVERRIDE" ]]; then
  TEMP_WORKSPACE="$WORKSPACE_ROOT_OVERRIDE"
else
  TEMP_WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/rustclaw-long-tail-nl-XXXXXX")"
fi

prepare_temp_workspace "$TEMP_WORKSPACE"
patch_temp_config \
  "$TEMP_WORKSPACE/configs/config.toml" \
  "$PORT" \
  "$TEMP_WORKSPACE/data/long_tail_nl.sqlite"
prepare_http_demo "$TEMP_WORKSPACE"
prepare_http_repair_demo "$TEMP_WORKSPACE"

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

printf 'workspace_root=%s\nbase_url=%s\nhttp_port=%s\nhttp_dir=%s\nhttp_marker=%s\nhttp_repair_port=%s\nhttp_repair_dir=%s\nhttp_repair_marker=%s\nhttp_repair_bad_marker=%s\nadmin_key=%s\nuser_key=%s\nrounds=%s\ncase_file=%s\n' \
  "$TEMP_WORKSPACE" "$BASE_URL" "$HTTP_PORT" "$HTTP_DIR_REL" "$HTTP_MARKER" "$HTTP_REPAIR_PORT" "$REPAIR_HTTP_DIR_REL" "$REPAIR_HTTP_MARKER" "$REPAIR_HTTP_BAD_MARKER" "$ADMIN_USER_KEY" "$REGULAR_USER_KEY" "$ROUNDS" "$CASE_FILE" > "$LOG_DIR/meta.txt"

for round_no in $(seq 1 "$ROUNDS"); do
  ordinal=0
  while IFS=$'\x1f' read -r _ case_name auth_kind assertion expected prompt; do
    ordinal=$((ordinal + 1))
    run_nl_case "$round_no" "$ordinal" "$case_name" "$auth_kind" "$assertion" "$expected" "$prompt"
  done < <(load_case_rows "$CASE_FILE")
done

echo
echo "Summary: pass=${PASS} fail=${FAIL} skip=${SKIP}"
[[ "$FAIL" -eq 0 ]]
