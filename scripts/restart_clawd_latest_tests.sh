#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_ROOT="$(mktemp -d)"
MOCK_BIN="$TEST_ROOT/bin"
LISTEN_MARKER="$TEST_ROOT/listening"
PROCESS_MARKER="$TEST_ROOT/process.pid"

cleanup() {
  if [[ -f "$PROCESS_MARKER" ]]; then
    kill "$(cat "$PROCESS_MARKER")" >/dev/null 2>&1 || true
  fi
  find "$TEST_ROOT" -type f -delete 2>/dev/null || true
  find "$TEST_ROOT" -depth -type d -empty -delete 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p \
  "$MOCK_BIN" \
  "$TEST_ROOT/runtime/scripts" \
  "$TEST_ROOT/runtime/target/release" \
  "$TEST_ROOT/runtime/configs"
cp "$ROOT_DIR/scripts/restart_clawd_latest.sh" "$TEST_ROOT/runtime/scripts/"

cat > "$TEST_ROOT/runtime/scripts/version_info.sh" <<'EOF'
print_app_version() { :; }
EOF
cat > "$TEST_ROOT/runtime/scripts/shell_compat.sh" <<'EOF'
configure_platform_command_path() { :; }
configure_python3_with_tomllib() { :; }
EOF
cat > "$TEST_ROOT/runtime/target/release/clawd" <<'EOF'
#!/bin/bash
printf '%s\n' "$$" > "$RESTART_TEST_PROCESS_MARKER"
touch "$RESTART_TEST_LISTEN_MARKER"
trap 'exit 0' TERM INT
while :; do sleep 1; done
EOF
cat > "$MOCK_BIN/ss" <<'EOF'
#!/bin/bash
if [[ -f "$RESTART_TEST_LISTEN_MARKER" ]]; then
  printf '%s\n' 'LISTEN 0 4096 127.0.0.1:8787 0.0.0.0:* users:(("clawd",pid=123,fd=1))'
else
  printf '%s\n' 'State Recv-Q Send-Q Local Address:Port Peer Address:Port'
fi
EOF
cat > "$MOCK_BIN/pgrep" <<'EOF'
#!/bin/bash
[[ -f "$RESTART_TEST_PROCESS_MARKER" ]] || exit 1
pid="$(cat "$RESTART_TEST_PROCESS_MARKER")"
if [[ "${1:-}" == "-af" ]]; then
  printf '%s %s\n' "$pid" "$RESTART_TEST_CLAWD_BIN --config fixture.toml"
else
  printf '%s\n' "$pid"
fi
EOF
cat > "$MOCK_BIN/setsid" <<'EOF'
#!/bin/bash
if [[ "${1:-}" == "--help" ]]; then
  printf '%s\n' '  -w, --wait  wait program to exit, and use the same return'
  exit 0
fi
[[ "${1:-}" == "--wait" ]] && shift
"$@" &
child_pid=$!
wait "$child_pid"
EOF
chmod +x \
  "$TEST_ROOT/runtime/scripts/restart_clawd_latest.sh" \
  "$TEST_ROOT/runtime/target/release/clawd" \
  "$MOCK_BIN/ss" \
  "$MOCK_BIN/pgrep" \
  "$MOCK_BIN/setsid"

PATH="$MOCK_BIN:/bin" \
HOME="$TEST_ROOT/home" \
APP_CONFIG_PATH="$TEST_ROOT/runtime/configs/config.toml" \
APP_CLAWD_STARTUP_WAIT_SECONDS=5 \
RESTART_TEST_LISTEN_MARKER="$LISTEN_MARKER" \
RESTART_TEST_PROCESS_MARKER="$PROCESS_MARKER" \
RESTART_TEST_CLAWD_BIN="$TEST_ROOT/runtime/target/release/clawd" \
  "$TEST_ROOT/runtime/scripts/restart_clawd_latest.sh" > "$TEST_ROOT/output.log"

grep -Fq '127.0.0.1:8787' "$TEST_ROOT/output.log"
[[ -s "$TEST_ROOT/runtime/.pids/clawd.pid" ]]
[[ "$(cat "$TEST_ROOT/runtime/.pids/clawd.pid")" == "$(cat "$PROCESS_MARKER")" ]]
echo "RESTART_CLAWD_LATEST_TESTS ok"
