#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/shell_compat.sh"
configure_platform_command_path
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
PRINT_LLM_TRACE_VALUE="${PRINT_LLM_TRACE:-1}"
SELF_TEST_TRANSPORT=0
CASE_NAMES=()

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

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" "$1"
}
REPAIR_HTTP_DIR_REL="document/nl_ops_http_repair_demo"
REPAIR_HTTP_INDEX_REL="${REPAIR_HTTP_DIR_REL}/index.html"
PASS=0
FAIL=0
SKIP=0

init_llm_trace_offset() {
  local offset_file="$1"
  python3 "${ROOT_DIR}/scripts/nl_tests/print_llm_raw_trace.py" \
    --log "$TEMP_WORKSPACE/logs/model_io.log" \
    --state-file "$offset_file" \
    --init-state
}

print_new_llm_trace() {
  local task_id="$1"
  local offset_file="$2"
  [[ "${PRINT_LLM_TRACE:-1}" == "1" ]] || return 0
  python3 "${ROOT_DIR}/scripts/nl_tests/print_llm_raw_trace.py" \
    --log "$TEMP_WORKSPACE/logs/model_io.log" \
    --task-id "$task_id" \
    --state-file "$offset_file"
}

usage() {
  cat <<'EOF'
Usage:
  bash scripts/regression_long_tail_nl_flows.sh [options]

Options:
  --case-file PATH         NL case file. Default: scripts/nl_tests/cases/nl_cases_long_tail_flows.txt
  --case-name NAME         Run one named case. Repeat to select multiple cases.
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
  --self-test-transport    Validate large task payload transport without starting clawd
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
    --case-name)
      CASE_NAMES+=("${2:-}")
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
    --self-test-transport)
      SELF_TEST_TRANSPORT=1
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
  if [[ ! -e "$workspace_root/data/skill-packages" ]]; then
    ln -s "$ROOT_DIR/data/skill-packages" "$workspace_root/data/skill-packages"
  fi
  ln -s "$ROOT_DIR/crates" "$workspace_root/crates"
  ln -s "$ROOT_DIR/scripts" "$workspace_root/scripts"
  ln -s "$ROOT_DIR/target" "$workspace_root/target"
}

patch_temp_config() {
  local config_path="$1"
  local sqlite_path="$2"
  python3 - "$config_path" "$sqlite_path" <<'PY'
from pathlib import Path
import re
import sys

config_path = Path(sys.argv[1])
sqlite_path = sys.argv[2]
text = config_path.read_text(encoding="utf-8")

def replace_once(pattern: str, replacement: str, raw: str) -> str:
    updated, count = re.subn(pattern, replacement, raw, count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"failed to patch config pattern: {pattern}")
    return updated

text = replace_once(r'^sqlite_path\s*=\s*".*"$', f'sqlite_path = "{sqlite_path}"', text)
text = replace_once(
    r'^access_profile\s*=\s*".*"$',
    'access_profile = "full"',
    text,
)

config_path.write_text(text, encoding="utf-8")
PY

  grep -Fxq 'access_profile = "full"' "$config_path" || {
    echo "long-tail test config did not enable its required full tool profile" >&2
    return 1
  }
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
    "$ROOT_DIR/crates/skill-sdk"
  )
  local health_check_inputs=(
    "$ROOT_DIR/Cargo.toml"
    "$ROOT_DIR/Cargo.lock"
    "$ROOT_DIR/configs"
    "$ROOT_DIR/prompts"
    "$ROOT_DIR/crates/skills/health_check"
  )
  local kb_inputs=(
    "$ROOT_DIR/Cargo.toml"
    "$ROOT_DIR/Cargo.lock"
    "$ROOT_DIR/configs"
    "$ROOT_DIR/prompts"
    "$ROOT_DIR/crates/skills/kb"
  )
  [[ -x "$CLAWD_BIN" ]] || need_build=1
  [[ -x "$ROOT_DIR/target/release/skill-runner" ]] || need_build=1
  [[ -x "$ROOT_DIR/target/release/health-check-skill" ]] || need_build=1
  [[ -x "$ROOT_DIR/target/release/kb-skill" ]] || need_build=1
  [[ -x "$ROOT_DIR/target/release/rustclaw-skill" ]] || need_build=1
  [[ -f "$ROOT_DIR/data/skill-packages/health_check/current.json" ]] || need_build=1
  [[ -f "$ROOT_DIR/data/skill-packages/kb/current.json" ]] || need_build=1
  if [[ "$AUTO_BUILD" == "1" ]]; then
    stale="$(binary_is_stale "$CLAWD_BIN" "${clawd_inputs[@]}")"
    [[ "$stale" == "1" ]] && need_build=1
    stale="$(binary_is_stale "$ROOT_DIR/target/release/skill-runner" "${skill_runner_inputs[@]}")"
    [[ "$stale" == "1" ]] && need_build=1
    stale="$(binary_is_stale "$ROOT_DIR/target/release/health-check-skill" "${health_check_inputs[@]}")"
    [[ "$stale" == "1" ]] && need_build=1
    stale="$(binary_is_stale "$ROOT_DIR/target/release/kb-skill" "${kb_inputs[@]}")"
    [[ "$stale" == "1" ]] && need_build=1
  fi

  if [[ "$need_build" == "1" && "$AUTO_BUILD" == "1" ]]; then
    echo "building fresh binaries for long-tail NL regression"
    configure_cargo_build_environment
    (
      cd "$ROOT_DIR"
      cargo build -p clawd
      cargo build --release \
        -p skill-runner \
        -p rustclaw-skill-sdk \
        -p health-check-skill \
        -p kb-skill
      python3 scripts/project_skill_receipts.py \
        --package-root "$ROOT_DIR/data/skill-packages" \
        --skill health_check
      python3 scripts/project_skill_receipts.py \
        --package-root "$ROOT_DIR/data/skill-packages" \
        --skill kb
    )
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
  [[ -x "$ROOT_DIR/target/release/rustclaw-skill" ]] || {
    echo "skill SDK CLI missing: $ROOT_DIR/target/release/rustclaw-skill" >&2
    exit 2
  }
  "$ROOT_DIR/target/release/rustclaw-skill" receipt-verify \
    "$ROOT_DIR/data/skill-packages" health_check >/dev/null || {
    echo "verified install receipt missing for skill: health_check" >&2
    exit 2
  }
  "$ROOT_DIR/target/release/rustclaw-skill" receipt-verify \
    "$ROOT_DIR/data/skill-packages" kb >/dev/null || {
    echo "verified install receipt missing for skill: kb" >&2
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
  python3 /dev/fd/3 3<<'PY' <<<"$1"
import json
import re
import sys

obj = json.load(sys.stdin)
data = obj.get("data") or {}
result = data.get("result_json") or {}
if str(data.get("status") or "") not in {"failed", "timeout"}:
    raise SystemExit(1)

def walk(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)

def decoded_object(value):
    if not isinstance(value, str):
        return None
    value = value.strip()
    if not value.startswith("{"):
        return None
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError:
        return None
    return parsed if isinstance(parsed, dict) else None

machine_objects = list(walk(result))
journal_summary = ((result.get("task_journal") or {}).get("summary") or {})
for candidate in (
    result.get("text"),
    journal_summary.get("final_answer"),
    data.get("error_text"),
):
    parsed = decoded_object(candidate)
    if parsed is not None:
        machine_objects.extend(walk(parsed))

for item in machine_objects:
    external_blocked = item.get("external_provider_blocked") is True
    attribution = str(item.get("failure_attribution") or "").strip().lower()
    codes = {
        str(item.get(field) or "").strip().lower()
        for field in (
            "error_code",
            "provider_blocker_status_code",
            "reason_code",
            "status_code",
        )
    }
    provider_code = any(
        code == "rate_limited"
        or code.startswith("provider_")
        or code.startswith("llm_provider_")
        for code in codes
    )
    if external_blocked and (attribution == "provider_gap" or provider_code):
        raise SystemExit(0)

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
  python3 /dev/fd/3 3<<'PY' <<<"$1"
import json
import sys

obj = json.load(sys.stdin)
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
  python3 /dev/fd/3 "$2" 3<<'PY' <<<"$1"
import sys

text = sys.stdin.read()
expected = [part for part in sys.argv[1].split(";;") if part]
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

lifecycle_structured_summary() {
  python3 /dev/fd/3 "$2" 3<<'PY' <<<"$1"
import json
import re
import sys

obj = json.load(sys.stdin)
expected = [part.strip() for part in sys.argv[1].split(";;") if part.strip()]
data = obj.get("data") or {}
result = data.get("result_json") or {}
trace = ((result.get("task_journal") or {}).get("trace") or {})
steps = [step for step in trace.get("step_results") or [] if isinstance(step, dict)]

parts = []
for candidate in (data.get("error_text"), result.get("text")):
    if isinstance(candidate, str) and candidate.strip():
        parts.append(candidate.strip())
for message in result.get("messages") or []:
    if isinstance(message, str):
        parts.append(message)
    elif isinstance(message, dict) and isinstance(message.get("text"), str):
        parts.append(message["text"])

dry_run_true = []
action_refs = []
for index, step in enumerate(steps):
    for key in (
        "requested_action_ref",
        "requested_capability",
        "resolved_capability",
        "executed_skill",
        "output_excerpt",
        "error_excerpt",
    ):
        value = step.get(key)
        if isinstance(value, str) and value.strip():
            parts.append(value.strip())
            if key in {"requested_action_ref", "requested_capability", "resolved_capability"}:
                action_refs.append(value.strip())
    contract = step.get("contract") or {}
    value = contract.get("requested_action_ref")
    if isinstance(value, str) and value.strip():
        parts.append(value.strip())
        action_refs.append(value.strip())
    evidence = step.get("observed_evidence") or {}
    for item in evidence.get("items") or []:
        if not isinstance(item, dict):
            continue
        field = str(item.get("field") or "")
        excerpt = item.get("excerpt")
        if isinstance(excerpt, str):
            parts.append(excerpt)
            if field.split(".")[-1] == "dry_run" and excerpt.strip().lower() == "true":
                dry_run_true.append(f"step={index}:{field}")
        values = item.get("sample_values")
        if isinstance(values, list):
            parts.extend(str(value) for value in values)

haystack = "\n".join(parts)
missing = []
if str(data.get("status") or "") != "succeeded":
    missing.append(f"task_status={data.get('status')}")
for marker in expected:
    alternatives = [part.strip() for part in marker.split("__OR__") if part.strip()]
    if alternatives and not any(part in haystack for part in alternatives):
        missing.append(f"marker_missing={marker}")
if not steps:
    missing.append("step_results_missing")
if not action_refs:
    missing.append("capability_action_refs_missing")
if dry_run_true:
    missing.append("non_x_dry_run_observed=" + ",".join(dry_run_true))
if "preview_background_command" in haystack:
    missing.append("background_command_was_previewed")
if "kb.ingest" in expected:
    refs = [str(step.get("requested_action_ref") or "") for step in steps]
    try:
        ingest_index = refs.index("kb.ingest")
        status_index = refs.index("kb.ingest_job_status", ingest_index + 1)
        resume_index = refs.index("kb.resume_ingest", status_index + 1)
        search_index = refs.index("kb.search", resume_index + 1)
        delete_index = refs.index("kb.delete_namespace", search_index + 1)
    except ValueError:
        missing.append("kb_checkpoint_action_order_invalid")
    else:
        excerpts = [str(step.get("output_excerpt") or "") for step in steps]
        if '"complete":false' not in excerpts[ingest_index]:
            missing.append("kb_initial_checkpoint_not_incomplete")
        if not any('"complete":true' in excerpts[index] for index in range(resume_index, search_index)):
            missing.append("kb_resume_did_not_complete")
        if '"removed_documents":2' not in excerpts[delete_index]:
            missing.append("kb_cleanup_document_count_not_two")
        job_ids = set(re.findall(r"kbj-[a-zA-Z0-9_-]+", "\n".join(excerpts[ingest_index:search_index])))
        canonical_job_id = max(job_ids, key=len, default="")
        if len(canonical_job_id) < 16 or any(not canonical_job_id.startswith(value) for value in job_ids):
            missing.append(f"kb_job_identity_count={len(job_ids)}")
if missing:
    print("\n".join(missing))
    raise SystemExit(1)
print(
    f"status=succeeded; steps={len(steps)}; actions={len(action_refs)}; "
    "non_x_dry_run=0; markers=complete"
)
PY
}

lifecycle_expected_failure_summary() {
  python3 /dev/fd/3 "$2" 3<<'PY' <<<"$1"
import json
import sys

obj = json.load(sys.stdin)
expected = [part.strip() for part in sys.argv[1].split(";;") if part.strip()]
data = obj.get("data") or {}
result = data.get("result_json") or {}
serialized = json.dumps(result, ensure_ascii=False, sort_keys=True)
steps = (((result.get("task_journal") or {}).get("trace") or {}).get("step_results") or [])

def dry_run_true(value):
    if isinstance(value, dict):
        return any((key == "dry_run" and child is True) or dry_run_true(child) for key, child in value.items())
    if isinstance(value, list):
        return any(dry_run_true(child) for child in value)
    return False

missing = []
if str(data.get("status") or "") != "failed":
    missing.append(f"task_status={data.get('status')}")
for marker in expected:
    alternatives = [part.strip() for part in marker.split("__OR__") if part.strip()]
    if alternatives and not any(part in serialized for part in alternatives):
        missing.append(f"marker_missing={marker}")
if not steps:
    missing.append("step_results_missing")
if dry_run_true(result):
    missing.append("non_x_dry_run_observed")
if "preview_background_command" in serialized:
    missing.append("background_command_was_previewed")
if missing:
    print("\n".join(missing))
    raise SystemExit(1)
print(f"status=failed_expected; steps={len(steps)}; non_x_dry_run=0; terminal_contract=complete")
PY
}

wait_for_async_job_checkpoint() {
  local task_id="$1"
  local limit_seconds="$2"
  local waited=0
  while (( waited <= limit_seconds )); do
    local raw
    raw="$(query_task "$task_id")"
    if python3 /dev/fd/3 3<<'PY' <<<"$raw"
import json
import sys

obj = json.load(sys.stdin)
data = obj.get("data") or {}
if data.get("status") not in {"queued", "running"}:
    raise SystemExit(1)

def has_local_job(value):
    if isinstance(value, dict):
        job_id = value.get("job_id")
        if isinstance(job_id, str) and job_id.startswith("local_process:"):
            return True
        return any(has_local_job(child) for child in value.values())
    if isinstance(value, list):
        return any(has_local_job(child) for child in value)
    return False

raise SystemExit(0 if has_local_job(data.get("result_json") or {}) else 1)
PY
    then
      printf '%s\n' "$raw"
      return 0
    fi
    sleep "$POLL_INTERVAL_SECONDS"
    waited=$((waited + POLL_INTERVAL_SECONDS))
  done
  return 1
}

run_lifecycle_concurrent_case() {
  local round_no="$1" case_name="$2" auth_kind="$3" assertion="$4" expected="$5" prompt="$6"
  local case_dir="$LOG_DIR/cases/ask_round${round_no}_${case_name}"
  local submit_raw task_id pending_raw health_submit health_task_id health_final long_during long_final note
  mkdir -p "$case_dir"
  printf '%s\n' "$prompt" > "$case_dir/prompt.txt"
  submit_raw="$(submit_task "$prompt")"
  printf '%s\n' "$submit_raw" > "$case_dir/submit.json"
  task_id="$(extract_submit_task_id "$submit_raw")"
  if ! pending_raw="$(wait_for_async_job_checkpoint "$task_id" "$WAIT_SECONDS")"; then
    note="long task did not publish an active async checkpoint"
  else
    printf '%s\n' "$pending_raw" > "$case_dir/long_pending.json"
    health_submit="$(submit_run_skill_task "health_check" "{}")"
    printf '%s\n' "$health_submit" > "$case_dir/health_submit.json"
    health_task_id="$(extract_submit_task_id "$health_submit")"
    health_final="$(wait_task_until_terminal_with_limit "$health_task_id" 20 || true)"
    printf '%s\n' "$health_final" > "$case_dir/health_final.json"
    long_during="$(query_task "$task_id")"
    printf '%s\n' "$long_during" > "$case_dir/long_during_health_completion.json"
    long_final="$(wait_task_until_terminal_with_limit "$task_id" "$WAIT_SECONDS" || true)"
    printf '%s\n' "$long_final" > "$case_dir/final.json"
    if note="$(python3 /dev/fd/3 "$expected" "$case_dir/health_final.json" "$case_dir/long_during_health_completion.json" 3<<'PY' <<<"$long_final"
import json
import sys
from pathlib import Path

long_final = json.load(sys.stdin)
expected = [part for part in sys.argv[1].split(";;") if part]
health = json.loads(Path(sys.argv[2]).read_text())
long_during = json.loads(Path(sys.argv[3]).read_text())
long_result = (long_final.get("data") or {}).get("result_json") or {}
health_result = (health.get("data") or {}).get("result_json") or {}
long_text = json.dumps(long_result, ensure_ascii=False, sort_keys=True)
health_text = json.dumps(health_result, ensure_ascii=False, sort_keys=True)
missing = []
if (long_final.get("data") or {}).get("status") != "succeeded": missing.append("long_task_not_succeeded")
if (health.get("data") or {}).get("status") != "succeeded": missing.append("health_task_not_succeeded")
if (long_during.get("data") or {}).get("status") not in {"queued", "running"}: missing.append("long_task_not_active_when_health_finished")
if "system_health" not in health_text: missing.append("health_check_result_missing")
for marker in expected:
    if marker not in long_text: missing.append(f"long_marker_missing={marker}")
if '"dry_run": true' in long_text or '"dry_run": true' in health_text: missing.append("non_x_dry_run_observed")
if missing:
    print("\n".join(missing)); raise SystemExit(1)
print("long_active_during_health=1; health_status=succeeded; long_status=succeeded; non_x_dry_run=0")
PY
)"; then
      echo "[PASS] ${case_name} (${note})"
      PASS=$((PASS + 1)); append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "pass" "$note"
      return
    fi
  fi
  echo "[FAIL] ${case_name}: ${note}"
  FAIL=$((FAIL + 1)); append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "$note"
}

run_lifecycle_cancel_case() {
  local round_no="$1" case_name="$2" auth_kind="$3" assertion="$4" expected="$5" prompt="$6"
  local case_dir="$LOG_DIR/cases/ask_round${round_no}_${case_name}"
  local submit_raw task_id pending_raw cancel_first cancel_second final_raw stable_raw note
  local -a auth_args=()
  mkdir -p "$case_dir"
  printf '%s\n' "$prompt" > "$case_dir/prompt.txt"
  submit_raw="$(submit_task "$prompt")"; printf '%s\n' "$submit_raw" > "$case_dir/submit.json"
  task_id="$(extract_submit_task_id "$submit_raw")"
  if ! pending_raw="$(wait_for_async_job_checkpoint "$task_id" "$WAIT_SECONDS")"; then
    note="long task did not publish an active async checkpoint"
  else
    printf '%s\n' "$pending_raw" > "$case_dir/pending.json"
    array_from_command_lines auth_args curl_auth_args
    cancel_first="$(curl -sS -X POST "${BASE_URL}/v1/tasks/cancel-by-task-id" -H "Content-Type: application/json" "${auth_args[@]}" -d "{\"task_id\":\"${task_id}\"}")"
    printf '%s\n' "$cancel_first" > "$case_dir/cancel_first.json"
    final_raw="$(wait_task_until_terminal_with_limit "$task_id" 30 || true)"
    printf '%s\n' "$final_raw" > "$case_dir/final.json"
    cancel_second="$(curl -sS -X POST "${BASE_URL}/v1/tasks/cancel-by-task-id" -H "Content-Type: application/json" "${auth_args[@]}" -d "{\"task_id\":\"${task_id}\"}")"
    printf '%s\n' "$cancel_second" > "$case_dir/cancel_second.json"
    sleep 2
    stable_raw="$(query_task "$task_id")"; printf '%s\n' "$stable_raw" > "$case_dir/stable_terminal.json"
    if note="$(python3 - "$case_dir/pending.json" "$case_dir/cancel_first.json" "$case_dir/cancel_second.json" "$case_dir/final.json" "$case_dir/stable_terminal.json" "$expected" <<'PY'
import json
import os
import signal
import sys
from pathlib import Path

pending, first, second, final, stable = [json.loads(Path(p).read_text()) for p in sys.argv[1:6]]
expected = [part for part in sys.argv[6].split(";;") if part]
serialized = json.dumps(pending, ensure_ascii=False) + json.dumps(final, ensure_ascii=False)
missing = []
if not first.get("ok") or ((first.get("data") or {}).get("status") != "task_cancelled"): missing.append("first_cancel_failed")
if not second.get("ok") or ((second.get("data") or {}).get("status") != "task_already_cancelled"): missing.append("second_cancel_not_idempotent")
if (final.get("data") or {}).get("status") != "canceled": missing.append("task_not_canceled")
if (stable.get("data") or {}).get("status") != "canceled": missing.append("canceled_status_not_stable")
for marker in expected:
    if marker not in serialized: missing.append(f"marker_missing={marker}")
if '"dry_run": true' in serialized: missing.append("non_x_dry_run_observed")
cancel_result = (((final.get("data") or {}).get("result_json") or {}).get("cancel_adapter_result") or {})
if cancel_result.get("adapter_kind") != "local_process_poll": missing.append("local_cancel_adapter_missing")
if cancel_result.get("status") != "accepted": missing.append("local_cancel_not_accepted")
if not isinstance(cancel_result.get("pid"), int): missing.append("local_cancel_pid_missing")

def collect_pids(value):
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "pid" and isinstance(child, int): yield child
            yield from collect_pids(child)
    elif isinstance(value, list):
        for child in value: yield from collect_pids(child)
alive = []
for pid in set(collect_pids(cancel_result)):
    try: os.kill(pid, 0)
    except ProcessLookupError: pass
    except PermissionError: alive.append(pid)
    else: alive.append(pid)
if alive: missing.append("process_still_alive=" + ",".join(map(str, alive)))
if missing:
    print("\n".join(missing)); raise SystemExit(1)
print("first_cancel=applied; second_cancel=idempotent; terminal=canceled; process_gone=1; non_x_dry_run=0")
PY
)"; then
      echo "[PASS] ${case_name} (${note})"
      PASS=$((PASS + 1)); append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "pass" "$note"
      return
    fi
  fi
  echo "[FAIL] ${case_name}: ${note}"
  FAIL=$((FAIL + 1)); append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "$note"
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
  python3 /dev/fd/3 "$2" 3<<'PY' <<<"$1"
import json
import sys

obj = json.load(sys.stdin)
expected = sys.argv[1]
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

def step_text_parts(step):
    parts = []
    for key in ("output_excerpt", "error_excerpt"):
        value = step.get(key)
        if isinstance(value, str) and value.strip():
            parts.append(value)
    evidence = step.get("observed_evidence") or {}
    for item in evidence.get("items") or []:
        if not isinstance(item, dict):
            continue
        for key in ("excerpt", "sample_values"):
            value = item.get(key)
            if isinstance(value, str) and value.strip():
                parts.append(value)
            elif isinstance(value, list):
                parts.extend(str(entry) for entry in value if str(entry).strip())
    return parts

def trace_has_expected_part(part: str) -> bool:
    alternatives = [alt.strip() for alt in part.split("__OR__") if alt.strip()]
    if not alternatives:
        return True
    if any(alt in visible_text for alt in alternatives):
        return True
    for step in trace.get("step_results") or []:
        if str(step.get("status") or "").lower() != "ok":
            continue
        haystack = "\n".join(step_text_parts(step))
        if any(alt in haystack for alt in alternatives):
            return True
    return False

step_results = [
    step
    for step in (trace.get("step_results") or [])
    if isinstance(step, dict) and str(step.get("status") or "").lower() == "ok"
]

def action_refs(step):
    refs = []
    for key in (
        "requested_action_ref",
        "requested_capability",
        "resolved_capability",
        "sanitized_args_summary",
    ):
        value = step.get(key)
        if isinstance(value, str) and value.strip():
            refs.append(value.strip().lower())
    return refs

def is_http_observation(step):
    refs = set(action_refs(step))
    if refs & {"http.get", "http_basic.get"}:
        return True
    if "system.run_command" not in refs:
        return False
    evidence_text = "\n".join(step_text_parts(step)).lower()
    return "curl " in evidence_text and "127.0.0.1:" in evidence_text

mutation_action_refs = {
    "fs_basic.write_text",
    "fs_basic.append_text",
    "fs_basic.make_dir",
    "fs_basic.remove_path",
    "fs_basic.rename_path",
    "fs_basic.copy_path",
    "write_file",
    "make_dir",
    "remove_file",
    "workspace.apply_patch",
    "workspace.replace_text",
    "workspace.write_text",
}

def is_structured_mutation(step):
    if isinstance(step.get("structured_workspace_mutation"), dict):
        return True
    return bool(set(action_refs(step)) & mutation_action_refs)

http_indexes = [
    idx for idx, step in enumerate(step_results) if is_http_observation(step)
]
mutation_indexes = [
    idx for idx, step in enumerate(step_results) if is_structured_mutation(step)
]
first_http_index = http_indexes[0] if http_indexes else None
repair_mutation_index = next(
    (
        idx
        for idx in mutation_indexes
        if first_http_index is not None and idx > first_http_index
    ),
    None,
)

coding_workflow = (
    ((result.get("task_journal") or {}).get("summary") or {}).get("coding_workflow")
    or trace.get("coding_workflow")
    or {}
)
changed_file_count = int(coding_workflow.get("changed_file_count") or 0)
repair_mutation = repair_mutation_index is not None
if not repair_mutation and first_http_index is not None and changed_file_count > 0:
    repair_mutation = True

if repair_mutation_index is not None:
    post_repair_validation = any(idx > repair_mutation_index for idx in http_indexes)
else:
    post_repair_validation = repair_mutation and len(http_indexes) >= 2
repair_round = len(rounds) >= 2 and repair_mutation and post_repair_validation

status = str(data.get("status") or "")
missing = []
if status != "succeeded":
    missing.append(f"status={status}")
for part in expected_parts:
    if not trace_has_expected_part(part):
        missing.append(f"trace_marker_missing={part}")
if not repair_round:
    missing.append("repair_round_missing")
if not repair_mutation:
    missing.append("repair_mutation_missing")
if not post_repair_validation:
    missing.append("post_repair_validation_missing")
if any(
    '"dry_run": true' in json.dumps(step, ensure_ascii=False, sort_keys=True)
    or '"preview": true' in json.dumps(step, ensure_ascii=False, sort_keys=True)
    for step in step_results
):
    missing.append("non_x_dry_run_observed")
if missing:
    print("\n".join(missing))
    raise SystemExit(1)
print(
    f"status={status}; rounds={len(rounds)}; repair_round=true; repair_mutation=true"
)
PY
}

ops_http_validation_summary() {
  python3 /dev/fd/3 "$2" 3<<'PY' <<<"$1"
import json
import sys

obj = json.load(sys.stdin)
expected_parts = [part.strip() for part in sys.argv[1].split(";;") if part.strip()]
data = obj.get("data") or {}
result = data.get("result_json") or {}
trace = ((result.get("task_journal") or {}).get("trace") or {})
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

def step_text_parts(step):
    parts = []
    for key in ("output_excerpt", "error_excerpt"):
        value = step.get(key)
        if isinstance(value, str) and value.strip():
            parts.append(value)
    evidence = step.get("observed_evidence") or {}
    for item in evidence.get("items") or []:
        if not isinstance(item, dict):
            continue
        for key in ("excerpt", "sample_values"):
            value = item.get(key)
            if isinstance(value, str) and value.strip():
                parts.append(value)
            elif isinstance(value, list):
                parts.extend(str(entry) for entry in value if str(entry).strip())
    return parts

status = str(data.get("status") or "")
missing = []
if status != "succeeded":
    missing.append(f"status={status}")
for part in expected_parts:
    alternatives = [alt.strip() for alt in part.split("__OR__") if alt.strip()]
    if not alternatives:
        continue
    found = False
    if any(alt in visible_text for alt in alternatives):
        found = True
    for step in trace.get("step_results") or []:
        if found:
            break
        if str(step.get("status") or "").lower() != "ok":
            continue
        haystack = "\n".join(step_text_parts(step))
        if any(alt in haystack for alt in alternatives):
            found = True
            break
    if not found:
        missing.append(f"trace_marker_missing={part}")
if missing:
    print("\n".join(missing))
    raise SystemExit(1)
print(f"status={status}; validation_marker_observed=true")
PY
}

health_check_structured_summary() {
  python3 /dev/fd/3 "$2" 3<<'PY' <<<"$1"
import json
import sys

obj = json.load(sys.stdin)
expected_paths = [part.strip() for part in sys.argv[1].split(";;") if part.strip()]
data = obj.get("data") or {}
result = data.get("result_json") or {}
trace = ((result.get("task_journal") or {}).get("trace") or {})

def collect_paths(value, prefix="", out=None):
    if out is None:
        out = set()
    if isinstance(value, dict):
        for key, child in value.items():
            child_path = f"{prefix}.{key}" if prefix else str(key)
            out.add(child_path)
            collect_paths(child, child_path, out)
    elif isinstance(value, list):
        for idx, child in enumerate(value):
            child_path = f"{prefix}[{idx}]"
            out.add(child_path)
            collect_paths(child, child_path, out)
    return out

def parse_json_text(text):
    if not isinstance(text, str) or not text.strip():
        return None
    try:
        return json.loads(text)
    except Exception:
        return None

observed_fields = set()
health_step_count = 0
for step in trace.get("step_results") or []:
    skill = str(
        step.get("executed_skill")
        or step.get("skill")
        or step.get("resolved_tool_or_skill")
        or ""
    )
    if skill != "health_check" or str(step.get("status") or "").lower() != "ok":
        continue
    health_step_count += 1
    evidence = step.get("observed_evidence") or {}
    for item in evidence.get("items") or []:
        if isinstance(item, dict) and isinstance(item.get("field"), str):
            observed_fields.add(item["field"])
    for candidate in (step.get("output_excerpt"), step.get("output")):
        value = parse_json_text(candidate)
        if value is None:
            continue
        observed_fields.update(collect_paths(value))
        extra = value.get("extra") if isinstance(value, dict) else None
        if isinstance(extra, dict):
            observed_fields.update(collect_paths(extra))
        text_value = value.get("text") if isinstance(value, dict) else None
        nested = parse_json_text(text_value)
        if nested is not None:
            observed_fields.update(collect_paths(nested))

status = str(data.get("status") or "")
missing = []
if status != "succeeded":
    missing.append(f"status={status}")
if health_step_count < 1:
    missing.append("health_check_step_missing")
for path in expected_paths:
    accepted = {path, f"extra.{path}"}
    if not (accepted & observed_fields):
        missing.append(f"field_missing={path}")
if missing:
    print("\n".join(missing))
    raise SystemExit(1)
print(
    f"status={status}; health_check_steps={health_step_count}; fields={','.join(expected_paths)}"
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
  local outcome_kind="$5"
  local note="$6"
  python3 - "$LOG_DIR/summary.jsonl" "$round_no" "$case_name" "$auth_kind" "$assertion" "$outcome_kind" "$note" <<'PY'
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
  python3 - "$case_file" "${CASE_NAMES[@]}" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
selected_names = set(sys.argv[2:])
matched_names = set()
for idx, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    parts = [part.strip() for part in line.split("|", 4)]
    if len(parts) != 5:
        raise SystemExit(f"invalid case format on line {idx}: {raw}")
    name, auth, assertion, expected, prompt = parts
    if selected_names and name not in selected_names:
        continue
    matched_names.add(name)
    print(f"{idx}\x1f{name}\x1f{auth}\x1f{assertion}\x1f{expected}\x1f{prompt}")

missing_names = sorted(selected_names - matched_names)
if missing_names:
    raise SystemExit(f"unknown case name(s): {','.join(missing_names)}")
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

  if [[ "$assertion" == "lifecycle_concurrent" ]]; then
    run_lifecycle_concurrent_case "$round_no" "$case_name" "$auth_kind" "$assertion" "$expected" "$prompt"
    return
  elif [[ "$assertion" == "lifecycle_cancel" ]]; then
    run_lifecycle_cancel_case "$round_no" "$case_name" "$auth_kind" "$assertion" "$expected" "$prompt"
    return
  fi

  if [[ "$assertion" == "ops_http" ]]; then
    kill_process_on_port "$HTTP_PORT"
  elif [[ "$assertion" == "ops_http_repair" ]]; then
    prime_broken_http_repair_demo_server "$TEMP_WORKSPACE" "$round_no" "$case_name"
  fi

  echo "[ask][round ${round_no}] ${case_name}"
  echo "[PROMPT]"
  printf '%s\n' "$prompt"
  local submit_raw task_id final_raw status visible_text missing
  submit_raw="$(submit_task "$prompt")"
  task_id="$(extract_submit_task_id "$submit_raw")"
  final_raw="$(wait_task_until_terminal_with_limit "$task_id" "$WAIT_SECONDS")"
  print_new_llm_trace "$task_id" "$LOG_DIR/llm_trace.offset"
  write_case_artifacts "ask" "$round_no" "$case_name" "$prompt" "$submit_raw" "$final_raw"
  status="$(extract_task_status "$final_raw")"

  if result_provider_unavailable "$final_raw"; then
    echo "[SKIP] ${case_name}: provider unavailable"
    SKIP=$((SKIP + 1))
    append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "skip" "provider unavailable"
    return
  fi

  visible_text="$(extract_visible_text "$final_raw")"
  echo "[REPLY]"
  printf '%s\n' "${visible_text:-<empty>}"
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
    health_check_structured)
      if missing="$(health_check_structured_summary "$final_raw" "$expected" 2>&1)"; then
        echo "[PASS] ${case_name} (${missing})"
        PASS=$((PASS + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "pass" "$missing"
      else
        echo "[FAIL] ${case_name}: ${missing}"
        FAIL=$((FAIL + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "$missing"
      fi
      ;;
    ops_http)
      if missing="$(ops_http_validation_summary "$final_raw" "$expected" 2>&1)"; then
        echo "[PASS] ${case_name} (${missing})"
        PASS=$((PASS + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "pass" "$missing"
      else
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
    lifecycle_structured)
      if missing="$(lifecycle_structured_summary "$final_raw" "$expected" 2>&1)"; then
        echo "[PASS] ${case_name} (${missing})"
        PASS=$((PASS + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "pass" "$missing"
      else
        echo "[FAIL] ${case_name}: ${missing}"
        FAIL=$((FAIL + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "$missing"
      fi
      ;;
    lifecycle_expected_failure)
      if missing="$(lifecycle_expected_failure_summary "$final_raw" "$expected" 2>&1)"; then
        echo "[PASS] ${case_name} (${missing})"
        PASS=$((PASS + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "pass" "$missing"
      else
        echo "[FAIL] ${case_name}: ${missing}"
        FAIL=$((FAIL + 1))
        append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "$missing"
      fi
      ;;
    *)
      echo "unsupported assertion kind in case ${case_name}: ${assertion}" >&2
      FAIL=$((FAIL + 1))
      append_summary "$round_no" "$case_name" "$auth_kind" "$assertion" "fail" "bad assertion kind"
      ;;
  esac
}

run_transport_self_test() {
  local payload
  local visible
  local lifecycle_payload
  local lifecycle_summary
  local expected_failure_payload
  local expected_failure_summary
  local dry_run_payload
  payload="$(
    python3 - <<'PY'
import json

print(json.dumps({
    "data": {
        "status": "failed",
        "error_text": "rate limit",
        "result_json": {
            "text": "X" * 300_000,
            "messages": [],
        },
    },
}))
PY
  )"
  visible="$(extract_visible_text "$payload")"
  [[ "${#visible}" -gt 300000 ]]
  result_provider_unavailable "$payload"
  missing_substrings "$visible" "rate limit;;$(printf 'X%.0s' {1..128})"
  lifecycle_payload="$({
    python3 - <<'PY'
import json

print(json.dumps({
    "data": {
        "status": "succeeded",
        "result_json": {
            "text": "RUSTCLAW_LONG_COMMAND_COMPLETE",
            "task_journal": {
                "trace": {
                    "step_results": [{
                        "requested_action_ref": "system.run_command",
                        "resolved_capability": "local_process_poll",
                        "observed_evidence": {
                            "items": [{
                                "field": "extra.output",
                                "excerpt": "RUSTCLAW_HEARTBEAT_7",
                            }],
                        },
                    }],
                },
            },
        },
    },
}))
PY
  })"
  lifecycle_summary="$(
    lifecycle_structured_summary \
      "$lifecycle_payload" \
      "system.run_command;;local_process_poll;;RUSTCLAW_HEARTBEAT_7"
  )"
  missing_substrings "$lifecycle_summary" "status=succeeded;;non_x_dry_run=0;;markers=complete"

  expected_failure_payload="$({
    python3 - <<'PY'
import json

print(json.dumps({
    "data": {
        "status": "failed",
        "result_json": {
            "error_code": "local_process_runtime_timeout",
            "exit_code": 124,
            "task_journal": {
                "trace": {
                    "step_results": [{
                        "requested_action_ref": "system.run_command",
                        "resolved_capability": "local_process_poll",
                    }],
                },
            },
        },
    },
}))
PY
  })"
  expected_failure_summary="$(
    lifecycle_expected_failure_summary \
      "$expected_failure_payload" \
      "system.run_command;;local_process_poll;;local_process_runtime_timeout;;124"
  )"
  missing_substrings "$expected_failure_summary" "status=failed_expected;;non_x_dry_run=0;;terminal_contract=complete"

  dry_run_payload="$({
    python3 - <<'PY'
import json

print(json.dumps({
    "data": {
        "status": "succeeded",
        "result_json": {
            "task_journal": {
                "trace": {
                    "step_results": [{
                        "requested_action_ref": "system.run_command",
                        "observed_evidence": {
                            "items": [{
                                "field": "extra.dry_run",
                                "excerpt": "true",
                            }],
                        },
                    }],
                },
            },
        },
    },
}))
PY
  })"
  if lifecycle_structured_summary "$dry_run_payload" "system.run_command" >/dev/null 2>&1; then
    echo "lifecycle assertion accepted a non-X dry-run result" >&2
    return 1
  fi
  echo "LONG_TAIL_RUNNER_TRANSPORT_SELF_TEST ok payload_bytes=${#payload} lifecycle_assertions=ok"
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
  echo "log_dir_ref=$(path_ref "${LOG_DIR}")"
  if [[ "$KEEP_WORKSPACE" == "1" && -n "$TEMP_WORKSPACE" ]]; then
    echo "workspace_root_ref=$(path_ref "${TEMP_WORKSPACE}")"
  fi
  exit "$exit_code"
}

if [[ "$SELF_TEST_TRANSPORT" == "1" ]]; then
  run_transport_self_test
  exit 0
fi

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
  RUSTCLAW_INTERNAL_LISTEN="127.0.0.1:${PORT}" \
    WORKSPACE_ROOT="$TEMP_WORKSPACE" "$CLAWD_BIN"
) >"$LOG_DIR/clawd.log" 2>&1 &
CLAWD_PID=$!

wait_for_health
init_llm_trace_offset "$LOG_DIR/llm_trace.offset"

printf 'workspace_root=%s\nbase_url=%s\nhttp_port=%s\nhttp_dir=%s\nhttp_marker=%s\nhttp_repair_port=%s\nhttp_repair_dir=%s\nhttp_repair_marker=%s\nhttp_repair_bad_marker=%s\ntemporary_admin_auth=generated\ntemporary_user_auth=generated\nrounds=%s\ncase_file=%s\n' \
  "$TEMP_WORKSPACE" "$BASE_URL" "$HTTP_PORT" "$HTTP_DIR_REL" "$HTTP_MARKER" "$HTTP_REPAIR_PORT" "$REPAIR_HTTP_DIR_REL" "$REPAIR_HTTP_MARKER" "$REPAIR_HTTP_BAD_MARKER" "$ROUNDS" "$CASE_FILE" > "$LOG_DIR/meta.txt"

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
