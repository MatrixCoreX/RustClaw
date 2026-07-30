#!/usr/bin/env bash
# Contract tests for the canonical shutdown entry point.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
UNRELATED_PID=""
OWNED_PID=""
RELATIVE_OWNED_PID=""

cleanup() {
  [[ -z "$UNRELATED_PID" ]] || kill "$UNRELATED_PID" >/dev/null 2>&1 || true
  [[ -z "$OWNED_PID" ]] || kill "$OWNED_PID" >/dev/null 2>&1 || true
  [[ -z "$RELATIVE_OWNED_PID" ]] || kill "$RELATIVE_OWNED_PID" >/dev/null 2>&1 || true
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

mkdir -p "$TMP_ROOT/target/release" "$TMP_ROOT/.pids"
cp "$ROOT_DIR/stop-agent.sh" "$TMP_ROOT/stop-agent.sh"
cat > "$TMP_ROOT/target/release/clawd" <<'EOF'
#!/usr/bin/env bash
while true; do
  sleep 1
done
EOF
chmod +x "$TMP_ROOT/stop-agent.sh" "$TMP_ROOT/target/release/clawd"

"$TMP_ROOT/target/release/clawd" &
OWNED_PID=$!
printf '%s\n' "$OWNED_PID" > "$TMP_ROOT/.pids/clawd.pid"

if [[ "$(uname -s)" == "Linux" && -L /proc/self/exe ]]; then
  cp "$(command -v sh)" "$TMP_ROOT/target/release/webd"
  (
    cd "$TMP_ROOT"
    exec target/release/webd -c 'sleep 30'
  ) &
  RELATIVE_OWNED_PID=$!
  printf '%s\n' "$RELATIVE_OWNED_PID" > "$TMP_ROOT/.pids/webd.pid"
  sleep 0.1
  if ! kill -0 "$RELATIVE_OWNED_PID" >/dev/null 2>&1; then
    echo "Workspace-relative webd test process exited before stop test." >&2
    exit 1
  fi
fi

sleep 30 &
UNRELATED_PID=$!
printf '%s\n' "$UNRELATED_PID" > "$TMP_ROOT/.pids/telegramd.pid"

APP_STOP_GRACE_SECONDS=1 "$TMP_ROOT/stop-agent.sh"

if kill -0 "$OWNED_PID" >/dev/null 2>&1; then
  echo "Owned clawd test process was not stopped." >&2
  exit 1
fi
OWNED_PID=""

if [[ -n "$RELATIVE_OWNED_PID" ]]; then
  if kill -0 "$RELATIVE_OWNED_PID" >/dev/null 2>&1; then
    echo "Workspace-relative webd test process was not stopped." >&2
    exit 1
  fi
  RELATIVE_OWNED_PID=""
fi

if ! kill -0 "$UNRELATED_PID" >/dev/null 2>&1; then
  echo "A process referenced by a mismatched PID file was killed." >&2
  exit 1
fi

if find "$TMP_ROOT/.pids" -type f -name '*.pid' -print -quit | grep -q .; then
  echo "PID files were not cleaned." >&2
  exit 1
fi

echo "STOP_AGENT_TESTS ok"
