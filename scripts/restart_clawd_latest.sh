#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="/home/guagua/rustclaw"
RUNTIME_ENV_FILE="${RUNTIME_ENV_FILE:-$HOME/runtime_env_filled.sh}"
CONFIG_PATH="${RUSTCLAW_CONFIG_PATH:-${ROOT_DIR}/configs/config.toml}"
PID_FILE="${ROOT_DIR}/.pids/clawd.pid"
LOG_FILE="${TMPDIR:-/tmp}/clawd.out"

cd "${ROOT_DIR}"
mkdir -p "$(dirname "${PID_FILE}")"

if [[ -f "${RUNTIME_ENV_FILE}" ]]; then
  # shellcheck source=/dev/null
  source "${RUNTIME_ENV_FILE}"
fi

CLAWD_BIN="${ROOT_DIR}/target/release/clawd"
if [[ ! -x "${CLAWD_BIN}" ]]; then
  echo "clawd binary missing: ${CLAWD_BIN}" >&2
  exit 1
fi

LISTEN_ADDR="$(
python3 - "${CONFIG_PATH}" <<'PY'
import sys
import tomllib
from pathlib import Path

cfg = tomllib.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
listen = str(cfg.get("server", {}).get("listen", "127.0.0.1:8787"))
print(listen)
PY
)"
PORT="${LISTEN_ADDR##*:}"

pkill -f 'target/release/clawd|cargo run -p clawd' || true

for _ in $(seq 1 50); do
  if ! ss -lnt | awk '{print $4}' | grep -Eq "[:.]${PORT}\$"; then
    break
  fi
  sleep 0.2
done

setsid "${CLAWD_BIN}" --config "${CONFIG_PATH}" >"${LOG_FILE}" 2>&1 </dev/null &

sleep 2

if ! pgrep -n -f "^${CLAWD_BIN}\$|${CLAWD_BIN}" > "${PID_FILE}"; then
  echo "failed to restart clawd" >&2
  echo "--- ${LOG_FILE} ---" >&2
  tail -n 80 "${LOG_FILE}" >&2 || true
  exit 1
fi

cat "${PID_FILE}"
echo '---'
pgrep -af "^${CLAWD_BIN}\$|${CLAWD_BIN}"
echo '---'
ss -lntp | rg "${PORT}|clawd"
