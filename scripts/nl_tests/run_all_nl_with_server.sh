#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/shell_compat.sh"
configure_platform_command_path

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
USER_KEY_VALUE="${USER_KEY:-${APP_USER_KEY:-}}"
INSTALL_ON_DEMAND_SKILLS=()
INSTALLED_ON_DEMAND_SKILLS=()

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
  --install-on-demand-skill NAME
                          install one registry on-demand skill through the
                          isolated Skill Store HTTP API before NL execution;
                          repeat for multiple skills. Reuse-server mode is rejected.
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
    --install-on-demand-skill)
      INSTALL_ON_DEMAND_SKILLS+=("$2")
      shift 2
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

for numeric_value in "$WAIT_SECONDS" "$POLL_SECONDS" "$START_TIMEOUT_SECONDS"; do
  if ! [[ "$numeric_value" =~ ^[1-9][0-9]*$ ]]; then
    echo "wait, poll, and start timeout values must be positive integers" >&2
    exit 2
  fi
done

if [[ "${BUILD_RELEASE}" -eq 1 || "${#INSTALL_ON_DEMAND_SKILLS[@]}" -gt 0 ]]; then
  configure_cargo_build_environment
fi

resolve_user_key() {
  if [[ -n "${USER_KEY_VALUE:-}" ]]; then
    return 0
  fi
  if [[ -x "${ROOT_DIR}/scripts/auth-key.sh" ]]; then
    local auth_output
    auth_output="$("${ROOT_DIR}/scripts/auth-key.sh" list 2>/dev/null || true)"
    USER_KEY_VALUE="$(awk '$2 == "admin" && $3 == "enabled" { print $1; exit }' <<<"${auth_output}")"
  fi
}

curl_health() {
  if [[ -z "${USER_KEY_VALUE:-}" ]]; then
    return 1
  fi
  local -a auth_args=()
  auth_args=(-H "X-Agent-Key: ${USER_KEY_VALUE}")
  curl -fsS --max-time 5 "${auth_args[@]}" "${health_url}" >/dev/null
}

started_pid=""
suite_pid=""
ISOLATION_ROOT=""
ISOLATED_WORKSPACE=""

prepare_isolated_workspace() {
  local name skill_name
  ISOLATED_WORKSPACE="${ISOLATION_ROOT}/workspace"
  mkdir -p \
    "${ISOLATED_WORKSPACE}/data/skill-packages" \
    "${ISOLATED_WORKSPACE}/data/skills" \
    "${ISOLATED_WORKSPACE}/logs" \
    "${ISOLATED_WORKSPACE}/optional_skills" \
    "${ISOLATED_WORKSPACE}/tmp" \
    "${ISOLATED_WORKSPACE}/.pids" \
    "${ISOLATED_WORKSPACE}/.agent-runtime"

  # Runtime path policy deliberately rejects workspace symlink escapes. Copy
  # only Git-visible source and fixtures so normal NL file operations exercise
  # real paths inside the isolated root without copying caches or model data.
  python3 "${SCRIPT_DIR}/materialize_isolated_workspace.py" \
    --source "${ROOT_DIR}" \
    --destination "${ISOLATED_WORKSPACE}" \
    --include-root docs

  # Build output and Git object storage are immutable inputs for this harness.
  # They are not exposed as ordinary workspace files to capability tests.
  for name in target .git; do
    if [[ -e "${ROOT_DIR}/${name}" && ! -e "${ISOLATED_WORKSPACE}/${name}" ]]; then
      ln -s "${ROOT_DIR}/${name}" "${ISOLATED_WORKSPACE}/${name}"
    fi
  done

  # Admission canonicalizes package manifests and rejects symlink escapes.
  # Optional source was materialized above, while build output still reuses the
  # shared target directory.
  for skill_name in "${INSTALL_ON_DEMAND_SKILLS[@]}"; do
    if [[ ! "${skill_name}" =~ ^[a-z0-9_]+$ ]] \
      || [[ ! -d "${ROOT_DIR}/optional_skills/${skill_name}" ]]; then
      echo "Invalid or missing on-demand skill source: ${skill_name}" >&2
      return 2
    fi
  done
}

skill_store_response_ok() {
  local skill_name="$1"
  local operation="$2"
  python3 -c '
import json, sys
payload = json.load(sys.stdin)
if payload.get("ok") is not True:
    raise SystemExit(f"skill={sys.argv[1]} operation={sys.argv[2]} response_not_ok={payload}")
' "$skill_name" "$operation"
}

wait_for_skill_store_operation() {
  local skill_name="$1"
  local operation="$2"
  local accepted_response="$3"
  local operation_id status response elapsed=0
  operation_id="$(jq -er '.data.operation.operation_id' <<<"$accepted_response")" || {
    echo "skill=${skill_name} operation=${operation} missing durable operation id" >&2
    echo "$accepted_response" >&2
    return 1
  }
  while (( elapsed <= WAIT_SECONDS )); do
    response="$(curl -fsS \
      -H "X-Agent-Key: ${USER_KEY_VALUE}" \
      "${BASE_URL%/}/v1/skills/store/operations/${operation_id}")" || return 1
    status="$(jq -r '.data.operation.status // ""' <<<"$response")"
    case "$status" in
      success)
        echo "skill_store_${operation}=ok skill=${skill_name} operation_id=${operation_id}"
        return 0
        ;;
      failure|cancelled)
        echo "skill_store_${operation}=failed skill=${skill_name} operation_id=${operation_id} status=${status}" >&2
        jq -c '.data.operation.failure // .error // .' <<<"$response" >&2
        return 1
        ;;
      queued|running|cancelling) ;;
      *)
        echo "skill_store_${operation}=invalid_status skill=${skill_name} operation_id=${operation_id} status=${status}" >&2
        return 1
        ;;
    esac
    sleep "$POLL_SECONDS"
    elapsed=$((elapsed + POLL_SECONDS))
  done
  echo "skill_store_${operation}=timeout skill=${skill_name} operation_id=${operation_id}" >&2
  return 1
}

project_proactive_skill_receipts() {
  local sdk_cli="${ROOT_DIR}/target/release/skillctl"
  if [[ ! -x "$sdk_cli" ]]; then
    echo "skill receipt CLI not found: ${sdk_cli}" >&2
    echo "Run: ./build-all.sh no-ui" >&2
    return 1
  fi
  python3 "${ROOT_DIR}/scripts/project_skill_receipts.py" \
    --target host \
    --scope proactive \
    --binary-dir "${ROOT_DIR}/target/release" \
    --sdk-cli "$sdk_cli" \
    --package-root "${ISOLATED_WORKSPACE}/data/skill-packages"
}

install_on_demand_skill() {
  local skill_name="$1"
  if [[ ! "$skill_name" =~ ^[a-z0-9_]+$ ]]; then
    echo "Invalid on-demand skill name: ${skill_name}" >&2
    return 2
  fi
  local response
  response="$(curl -fsS \
    -H "X-Agent-Key: ${USER_KEY_VALUE}" \
    -H "Content-Type: application/json" \
    --data "{\"skill_name\":\"${skill_name}\"}" \
    "${BASE_URL%/}/v1/skills/store/install")"
  if ! skill_store_response_ok "$skill_name" install <<<"$response"; then
    echo "$response" >&2
    return 1
  fi
  wait_for_skill_store_operation "$skill_name" install "$response" || return 1
  INSTALLED_ON_DEMAND_SKILLS+=("$skill_name")
}

remove_installed_on_demand_skills() {
  if [[ -z "${USER_KEY_VALUE:-}" || -z "${BASE_URL:-}" ]]; then
    return 0
  fi
  local index skill_name response
  for ((index=${#INSTALLED_ON_DEMAND_SKILLS[@]} - 1; index >= 0; index--)); do
    skill_name="${INSTALLED_ON_DEMAND_SKILLS[$index]}"
    response="$(curl -fsS \
      -H "X-Agent-Key: ${USER_KEY_VALUE}" \
      -H "Content-Type: application/json" \
      --data "{\"skill_name\":\"${skill_name}\",\"preserve_config\":true,\"preserve_data\":true}" \
      "${BASE_URL%/}/v1/skills/store/remove" 2>/dev/null || true)"
    if [[ -n "$response" ]] \
      && skill_store_response_ok "$skill_name" remove <<<"$response" \
      && wait_for_skill_store_operation "$skill_name" remove "$response"; then
      echo "skill_store_remove_preservation=ok skill=${skill_name} config_preserved=true data_preserved=true"
    else
      echo "skill_store_remove=failed skill=${skill_name}" >&2
    fi
  done
  INSTALLED_ON_DEMAND_SKILLS=()
}

cleanup() {
  if [[ -n "${suite_pid}" ]]; then
    kill "${suite_pid}" >/dev/null 2>&1 || true
    wait "${suite_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${started_pid}" ]] && kill -0 "${started_pid}" >/dev/null 2>&1; then
    remove_installed_on_demand_skills || true
  fi
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
  # Admission overlays are intentionally restricted to the workspace. Keep
  # the complete isolated runtime under the ignored workspace tmp directory
  # so the harness cannot accidentally share production skill state or ask
  # clawd to weaken its path boundary.
  mkdir -p "${ROOT_DIR}/tmp"
  ISOLATION_ROOT="$(mktemp -d "${ROOT_DIR}/tmp/agent-runtime-nl-isolated-XXXXXX")"
  ISOLATED_CONFIG="${ISOLATION_ROOT}/config.toml"
  ISOLATED_DB="${ISOLATION_ROOT}/tasks.sqlite"
  ISOLATED_AUDIT_DB="${ISOLATION_ROOT}/audit.sqlite"
  prepare_isolated_workspace
  python3 "${SCRIPT_DIR}/create_isolated_config.py" \
    --source "${SOURCE_CONFIG}" \
    --output "${ISOLATED_CONFIG}" \
    --sqlite-path "${ISOLATED_DB}" \
    --audit-sqlite-path "${ISOLATED_AUDIT_DB}" \
    --skill-data-root "${ISOLATED_WORKSPACE}/data/skills"
  echo "server_mode=isolated"
  echo "base_url=${BASE_URL}"
  echo "config_identity=isolated/config.toml"
  echo "task_db_identity=isolated/tasks.sqlite"
  echo "audit_db_identity=isolated/audit.sqlite"
  echo "skill_package_identity=isolated/data/skill-packages"
else
  if [[ "${#INSTALL_ON_DEMAND_SKILLS[@]}" -gt 0 ]]; then
    echo "--install-on-demand-skill requires the default isolated server mode" >&2
    exit 2
  fi
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
  export APP_CONFIG_PATH="${ISOLATED_CONFIG}"
  export APP_DB_PATH="${ISOLATED_DB}"
  export APP_INTERNAL_LISTEN="${isolated_listen}"
  export CLIENT_LIKE_CHANNEL="ui"
  export NL_MODEL_IO_LOG="${ISOLATED_WORKSPACE}/logs/model_io.log"
  # An isolated database gets its own generated admin key. Never reuse a key
  # inherited from the developer's normal runtime against that database.
  USER_KEY_VALUE=""
fi
health_url="${BASE_URL%/}/v1/health"
if [[ "${REUSE_SERVER}" -eq 1 ]]; then
  USER_KEY_VALUE="${USER_KEY_VALUE:-${USER_KEY:-${APP_USER_KEY:-}}}"
fi
resolve_user_key
if [[ -n "${USER_KEY_VALUE:-}" ]]; then
  export USER_KEY="${USER_KEY_VALUE}"
  export APP_USER_KEY="${APP_USER_KEY:-${USER_KEY_VALUE}}"
  echo "auth_key=resolved"
else
  echo "auth_key=missing"
fi

if [[ "${BUILD_RELEASE}" -eq 1 ]]; then
  cargo build -p clawd --release
fi

# Direct NL-suite startup bypasses start-all.sh, so project the same verified
# proactive receipts before exercising on-demand packages. This performs no
# compilation and never projects an on-demand Skill Store entry.
if [[ "${REUSE_SERVER}" -eq 0 ]]; then
  project_proactive_skill_receipts
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
  (
    cd "${ISOLATED_WORKSPACE}"
    exec "${CLAWD_BIN}" --config "${ISOLATED_CONFIG}"
  ) >"${SERVER_LOG}" 2>&1 &
  started_pid=$!
  echo "server_log=${SERVER_LOG}"
  echo "server_pid=${started_pid}"

  for second in $(seq 1 "${START_TIMEOUT_SECONDS}"); do
    if [[ -z "${USER_KEY_VALUE:-}" ]]; then
      resolve_user_key
      if [[ -n "${USER_KEY_VALUE:-}" ]]; then
        export USER_KEY="${USER_KEY_VALUE}"
        export APP_USER_KEY="${USER_KEY_VALUE}"
        echo "auth_key=resolved_from_isolated_db"
      fi
    fi
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

if [[ "${#INSTALL_ON_DEMAND_SKILLS[@]}" -gt 0 ]]; then
  for skill_name in "${INSTALL_ON_DEMAND_SKILLS[@]}"; do
    install_on_demand_skill "$skill_name"
  done
fi

stamp="$(date +%Y%m%d_%H%M%S)"
SUITE_LOG="${LOG_DIR%/}/agent_full_nl_${stamp}.out"

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
"${suite_cmd[@]}" > >(tee "${SUITE_LOG}") 2>&1 &
suite_pid=$!
server_exit_status=""
while kill -0 "${suite_pid}" >/dev/null 2>&1; do
  if [[ -n "${started_pid}" ]] && ! kill -0 "${started_pid}" >/dev/null 2>&1; then
    wait "${started_pid}"
    server_exit_status=$?
    started_pid=""
    echo "clawd exited while NL suite was running (status=${server_exit_status})" >&2
    kill "${suite_pid}" >/dev/null 2>&1 || true
    break
  fi
  sleep 1
done
wait "${suite_pid}"
suite_status=$?
suite_pid=""
if [[ -n "${server_exit_status}" ]]; then
  suite_status=1
fi
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

remove_installed_on_demand_skills

exit "${suite_status}"
