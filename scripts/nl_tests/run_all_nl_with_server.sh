#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

REQUESTED_BASE_URL="${BASE_URL:-}"
BASE_URL=""
CLAWD_BIN="${ROOT_DIR}/target/release/clawd"
SOURCE_CONFIG="${ROOT_DIR}/configs/config.toml"
RUNTIME_ENV_FILE="${ROOT_DIR}/../runtime_env_filled.sh"
WAIT_SECONDS="600"
POLL_SECONDS="1"
PROVIDER_RETRIES="0"
PROMPT_REPLY_ONLY=1
LOG_DIR="/tmp"
START_TIMEOUT_SECONDS="80"
REUSE_SERVER=0
BUILD_RELEASE=0
EXTRA_SUITE_ARGS=()
SUITE_SELECTION=(--category all)
USER_KEY_VALUE="${USER_KEY:-${RUSTCLAW_USER_KEY:-}}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/nl_tests/run_all_nl_with_server.sh [options] [-- extra run_suite args...]

What it does:
  1. Sources runtime_env_filled.sh when present.
  2. By default creates an isolated config, task DB, audit DB, random local
     port, and non-delivering UI-channel test server.
  3. Runs the selected NL suite/category; the default is category all.
  4. Prints log paths, prompt count, and rate-limit/unavailable count.
  5. Stops only the server process started by this script.

Options:
  --base-url URL          isolated clawd URL. Default: random 127.0.0.1 port
  --clawd-bin PATH        clawd binary. Default: target/release/clawd
  --source-config PATH    source config copied into the isolated runtime.
                          Default: configs/config.toml
  --suite NAME            run one named suite instead of category all
  --category NAME         run one suite category instead of category all
  --runtime-env PATH      runtime env file. Default: ../runtime_env_filled.sh
  --no-runtime-env        do not source any runtime env file
  --wait-seconds N        max wait seconds per NL case. Default: 600
  --poll-seconds N        polling interval seconds. Default: 1
  --provider-retries N    provider retry count passed to run_suite. Default: 0
  --log-dir PATH          log output directory. Default: /tmp
  --start-timeout N       health wait timeout. Default: 80
  --no-prompt-reply-only  show full run_suite output instead of only prompt/reply
  --reuse-server          explicitly reuse an existing server and its databases
  --no-reuse-server       retained compatibility spelling for the safe default
  --build-release         run cargo build -p clawd --release before starting
  -h, --help              show this help

Examples:
  bash scripts/nl_tests/run_all_nl_with_server.sh
  bash scripts/nl_tests/run_all_nl_with_server.sh --build-release
  bash scripts/nl_tests/run_all_nl_with_server.sh --suite client_like_continuous -- --case-limit 20
  bash scripts/nl_tests/run_all_nl_with_server.sh -- --no-llm-trace
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base-url)
      REQUESTED_BASE_URL="$2"
      shift 2
      ;;
    --clawd-bin)
      CLAWD_BIN="$2"
      shift 2
      ;;
    --source-config)
      SOURCE_CONFIG="$2"
      shift 2
      ;;
    --suite)
      SUITE_SELECTION=("$2")
      shift 2
      ;;
    --category)
      SUITE_SELECTION=(--category "$2")
      shift 2
      ;;
    --runtime-env)
      RUNTIME_ENV_FILE="$2"
      shift 2
      ;;
    --no-runtime-env)
      RUNTIME_ENV_FILE=""
      shift
      ;;
    --wait-seconds)
      WAIT_SECONDS="$2"
      shift 2
      ;;
    --poll-seconds)
      POLL_SECONDS="$2"
      shift 2
      ;;
    --provider-retries)
      PROVIDER_RETRIES="$2"
      shift 2
      ;;
    --log-dir)
      LOG_DIR="$2"
      shift 2
      ;;
    --start-timeout)
      START_TIMEOUT_SECONDS="$2"
      shift 2
      ;;
    --no-prompt-reply-only)
      PROMPT_REPLY_ONLY=0
      shift
      ;;
    --no-reuse-server)
      REUSE_SERVER=0
      shift
      ;;
    --reuse-server)
      REUSE_SERVER=1
      shift
      ;;
    --build-release)
      BUILD_RELEASE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      EXTRA_SUITE_ARGS+=("$@")
      break
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

resolve_user_key() {
  if [[ -n "${USER_KEY_VALUE:-}" ]]; then
    return 0
  fi
  if [[ -x "${ROOT_DIR}/scripts/auth-key.sh" ]]; then
    USER_KEY_VALUE="$("${ROOT_DIR}/scripts/auth-key.sh" list | awk '$2 == "admin" && $3 == "enabled" { print $1; exit }')"
  fi
}

curl_health() {
  local -a auth_args=()
  if [[ -n "${USER_KEY_VALUE:-}" ]]; then
    auth_args=(-H "X-RustClaw-Key: ${USER_KEY_VALUE}")
  fi
  curl -sS "${auth_args[@]}" "${health_url}" >/dev/null
}

started_pid=""
ISOLATION_ROOT=""

cleanup() {
  if [[ -n "${started_pid}" ]]; then
    kill "${started_pid}" >/dev/null 2>&1 || true
    wait "${started_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${ISOLATION_ROOT}" && -d "${ISOLATION_ROOT}" ]]; then
    rm -rf "${ISOLATION_ROOT}"
  fi
}
trap cleanup EXIT

cd "${ROOT_DIR}"
mkdir -p "${LOG_DIR}"

if [[ "${REUSE_SERVER}" -eq 0 ]]; then
  BASE_URL="${REQUESTED_BASE_URL}"
  if [[ -z "${BASE_URL}" ]]; then
    isolated_port="$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"
    BASE_URL="http://127.0.0.1:${isolated_port}"
  fi
  isolated_listen="$(python3 - "${BASE_URL}" <<'PY'
import sys
from urllib.parse import urlparse
parsed = urlparse(sys.argv[1])
if parsed.scheme != "http" or parsed.hostname not in {"127.0.0.1", "localhost"} or not parsed.port:
    raise SystemExit("isolated base URL must be http://127.0.0.1:<port> or http://localhost:<port>")
print(f"127.0.0.1:{parsed.port}")
PY
)"
  ISOLATION_ROOT="$(mktemp -d "${LOG_DIR%/}/rustclaw-nl-isolated-XXXXXX")"
  ISOLATED_CONFIG="${ISOLATION_ROOT}/config.toml"
  ISOLATED_DB="${ISOLATION_ROOT}/tasks.sqlite"
  ISOLATED_AUDIT_DB="${ISOLATION_ROOT}/audit.sqlite"
  python3 "${SCRIPT_DIR}/create_isolated_config.py" \
    --source "${SOURCE_CONFIG}" \
    --output "${ISOLATED_CONFIG}" \
    --sqlite-path "${ISOLATED_DB}" \
    --audit-sqlite-path "${ISOLATED_AUDIT_DB}" \
    --listen "${isolated_listen}"
  echo "server_mode=isolated"
  echo "base_url=${BASE_URL}"
  echo "config_identity=isolated/config.toml"
  echo "task_db_identity=isolated/tasks.sqlite"
  echo "audit_db_identity=isolated/audit.sqlite"
else
  BASE_URL="${REQUESTED_BASE_URL:-http://127.0.0.1:8787}"
  echo "server_mode=explicit_reuse"
  echo "base_url=${BASE_URL}"
fi
SELECTED_BASE_URL="${BASE_URL}"

if [[ -n "${RUNTIME_ENV_FILE}" && -f "${RUNTIME_ENV_FILE}" ]]; then
  # shellcheck disable=SC1090
  source "${RUNTIME_ENV_FILE}"
  echo "runtime_env=${RUNTIME_ENV_FILE}"
elif [[ -n "${RUNTIME_ENV_FILE}" ]]; then
  echo "runtime_env=missing:${RUNTIME_ENV_FILE}"
fi
BASE_URL="${SELECTED_BASE_URL}"
if [[ "${REUSE_SERVER}" -eq 0 ]]; then
  export RUSTCLAW_CONFIG_PATH="${ISOLATED_CONFIG}"
  export RUSTCLAW_DB_PATH="${ISOLATED_DB}"
  export CLIENT_LIKE_CHANNEL="ui"
fi
health_url="${BASE_URL%/}/v1/health"
USER_KEY_VALUE="${USER_KEY_VALUE:-${USER_KEY:-${RUSTCLAW_USER_KEY:-}}}"
resolve_user_key
if [[ -n "${USER_KEY_VALUE:-}" ]]; then
  export USER_KEY="${USER_KEY_VALUE}"
  export RUSTCLAW_USER_KEY="${RUSTCLAW_USER_KEY:-${USER_KEY_VALUE}}"
  echo "auth_key=resolved"
else
  echo "auth_key=missing"
fi

if [[ "${BUILD_RELEASE}" -eq 1 ]]; then
  cargo build -p clawd --release
fi

if curl_health >/dev/null 2>&1; then
  if [[ "${REUSE_SERVER}" -ne 1 ]]; then
    echo "A healthy clawd server is already running at isolated URL ${BASE_URL}" >&2
    exit 2
  fi
  echo "clawd_health=ok existing_server=${BASE_URL}"
else
  if [[ "${REUSE_SERVER}" -eq 1 ]]; then
    echo "No healthy clawd server is available for explicit reuse at ${BASE_URL}" >&2
    exit 2
  fi
  if [[ ! -x "${CLAWD_BIN}" ]]; then
    echo "clawd binary not found or not executable: ${CLAWD_BIN}" >&2
    echo "Run: cargo build -p clawd --release" >&2
    exit 2
  fi
  stamp="$(date +%Y%m%d_%H%M%S)"
  SERVER_LOG="${LOG_DIR%/}/clawd_full_nl_${stamp}.log"
  "${CLAWD_BIN}" --config "${ISOLATED_CONFIG}" >"${SERVER_LOG}" 2>&1 &
  started_pid=$!
  echo "server_log=${SERVER_LOG}"
  echo "server_pid=${started_pid}"

  for second in $(seq 1 "${START_TIMEOUT_SECONDS}"); do
    if curl_health >/dev/null 2>&1; then
      echo "clawd_health=ok after ${second}s"
      break
    fi
    sleep 1
    if ! kill -0 "${started_pid}" >/dev/null 2>&1; then
      echo "clawd exited before health" >&2
      tail -n 80 "${SERVER_LOG}" >&2 || true
      exit 1
    fi
    if [[ "${second}" = "${START_TIMEOUT_SECONDS}" ]]; then
      echo "clawd health timeout after ${START_TIMEOUT_SECONDS}s" >&2
      tail -n 80 "${SERVER_LOG}" >&2 || true
      exit 1
    fi
  done
fi

stamp="$(date +%Y%m%d_%H%M%S)"
SUITE_LOG="${LOG_DIR%/}/rustclaw_full_nl_${stamp}.out"

suite_cmd=(
  bash "${SCRIPT_DIR}/run_suite.sh"
  "${SUITE_SELECTION[@]}"
  --base-url "${BASE_URL}"
  --wait-seconds "${WAIT_SECONDS}"
  --poll-seconds "${POLL_SECONDS}"
  --provider-retries "${PROVIDER_RETRIES}"
)
if [[ "${PROMPT_REPLY_ONLY}" -eq 1 ]]; then
  suite_cmd+=(--prompt-reply-only)
fi
if [[ "${#EXTRA_SUITE_ARGS[@]}" -gt 0 ]]; then
  suite_cmd+=("${EXTRA_SUITE_ARGS[@]}")
fi

echo "suite_log=${SUITE_LOG}"
echo "suite_cmd=${suite_cmd[*]}"

set +e
"${suite_cmd[@]}" | tee "${SUITE_LOG}"
suite_status=${PIPESTATUS[0]}
set -e

prompt_count="$(grep -Ec '^(\[PROMPT\]|PROMPT:)' "${SUITE_LOG}" 2>/dev/null || true)"
rate_limit_count="$(grep -Ec 'Rate limit|rate_limit|usage limit|限流|模型暂时不可用' "${SUITE_LOG}" 2>/dev/null || true)"

echo "NL_SUITE_STATUS=${suite_status}"
echo "PROMPT_COUNT=${prompt_count}"
echo "RATE_LIMIT_OR_UNAVAILABLE_COUNT=${rate_limit_count}"
if [[ -n "${started_pid}" ]]; then
  echo "server_log=${SERVER_LOG}"
else
  echo "server_log=<reused existing server>"
fi
echo "suite_log=${SUITE_LOG}"

exit "${suite_status}"
