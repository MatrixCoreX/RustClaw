#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -P "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
RUNTIME_ENV_FILE="${RUNTIME_ENV_FILE:-${APP_RUNTIME_ENV_SCRIPT:-$HOME/runtime_env_filled.sh}}"
CONFIG_PATH="${APP_CONFIG_PATH:-${ROOT_DIR}/configs/config.toml}"
PID_FILE="${ROOT_DIR}/.pids/clawd.pid"
LOG_FILE="${APP_CLAWD_LOG_FILE:-${ROOT_DIR}/logs/clawd.run.log}"
STARTUP_WAIT_SECONDS="${APP_CLAWD_STARTUP_WAIT_SECONDS:-30}"

cd "${ROOT_DIR}"
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/version_info.sh"
print_app_version "${ROOT_DIR}"
mkdir -p "$(dirname "${PID_FILE}")"
mkdir -p "$(dirname "${LOG_FILE}")"

if [[ -f "${RUNTIME_ENV_FILE}" ]]; then
  # shellcheck source=/dev/null
  source "${RUNTIME_ENV_FILE}"
fi
# Match the normal multi-component startup path so a standalone clawd restart
# preserves the platform command PATH and the selected modern Python runtime.
# shellcheck source=/dev/null
source "${ROOT_DIR}/scripts/shell_compat.sh"
configure_platform_command_path
configure_python3_with_tomllib

CLAWD_BIN="${ROOT_DIR}/target/release/clawd"
if [[ ! -x "${CLAWD_BIN}" ]]; then
  echo "clawd binary missing: ${CLAWD_BIN}" >&2
  exit 1
fi

LISTEN_ADDR="${APP_INTERNAL_LISTEN:-127.0.0.1:8787}"
PORT="${LISTEN_ADDR##*:}"

if [[ ! "${STARTUP_WAIT_SECONDS}" =~ ^[1-9][0-9]*$ ]]; then
  echo "APP_CLAWD_STARTUP_WAIT_SECONDS must be a positive integer" >&2
  exit 2
fi

clawd_pids() {
  pgrep -f "^${CLAWD_BIN}([[:space:]]|$)" 2>/dev/null || true
}

port_is_listening() {
  if command -v ss >/dev/null 2>&1; then
    ss -lnt | awk '{print $4}' | grep -Eq "[:.]${PORT}$"
  elif command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"${PORT}" -sTCP:LISTEN >/dev/null 2>&1
  else
    return 1
  fi
}

while IFS= read -r existing_pid; do
  [[ -n "${existing_pid}" ]] && kill "${existing_pid}"
done < <(clawd_pids)

for _ in $(seq 1 50); do
  if ! port_is_listening; then
    break
  fi
  sleep 0.2
done

if port_is_listening; then
  echo "port ${PORT} is still in use after stopping ${CLAWD_BIN}" >&2
  exit 1
fi

if command -v setsid >/dev/null 2>&1; then
  setsid "${CLAWD_BIN}" --config "${CONFIG_PATH}" >"${LOG_FILE}" 2>&1 </dev/null &
  started_pid=$!
else
  nohup "${CLAWD_BIN}" --config "${CONFIG_PATH}" >"${LOG_FILE}" 2>&1 </dev/null &
  started_pid=$!
fi

for _ in $(seq 1 "$((STARTUP_WAIT_SECONDS * 5))"); do
  if ! kill -0 "${started_pid}" 2>/dev/null; then
    echo "clawd exited before becoming ready" >&2
    echo "--- ${LOG_FILE} ---" >&2
    tail -n 80 "${LOG_FILE}" >&2 || true
    exit 1
  fi
  if port_is_listening; then
    break
  fi
  sleep 0.2
done

if ! port_is_listening; then
  echo "clawd did not listen on port ${PORT} within ${STARTUP_WAIT_SECONDS}s" >&2
  echo "--- ${LOG_FILE} ---" >&2
  tail -n 80 "${LOG_FILE}" >&2 || true
  exit 1
fi

printf '%s\n' "${started_pid}" > "${PID_FILE}"

cat "${PID_FILE}"
echo '---'
pgrep -af "^${CLAWD_BIN}([[:space:]]|$)"
echo '---'
if command -v ss >/dev/null 2>&1; then
  ss -lntp | grep -E "${PORT}|clawd"
else
  lsof -nP -iTCP:"${PORT}" -sTCP:LISTEN
fi
