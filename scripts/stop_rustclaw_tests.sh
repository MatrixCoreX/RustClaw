#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
UNRELATED_PID=""
OWNED_PID=""

cleanup() {
  [[ -z "$UNRELATED_PID" ]] || kill "$UNRELATED_PID" >/dev/null 2>&1 || true
  [[ -z "$OWNED_PID" ]] || kill "$OWNED_PID" >/dev/null 2>&1 || true
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

mkdir -p "$TMP_ROOT/target/release" "$TMP_ROOT/.pids"
cp "$ROOT_DIR/stop-rustclaw.sh" "$TMP_ROOT/stop-rustclaw.sh"
cat > "$TMP_ROOT/target/release/clawd" <<'EOF'
#!/usr/bin/env bash
while true; do
  sleep 1
done
EOF
chmod +x "$TMP_ROOT/stop-rustclaw.sh" "$TMP_ROOT/target/release/clawd"

"$TMP_ROOT/target/release/clawd" &
OWNED_PID=$!
printf '%s\n' "$OWNED_PID" > "$TMP_ROOT/.pids/clawd.pid"

sleep 30 &
UNRELATED_PID=$!
printf '%s\n' "$UNRELATED_PID" > "$TMP_ROOT/.pids/webd.pid"

RUSTCLAW_STOP_GRACE_SECONDS=1 "$TMP_ROOT/stop-rustclaw.sh"

if kill -0 "$OWNED_PID" >/dev/null 2>&1; then
  echo "Owned clawd test process was not stopped." >&2
  exit 1
fi
OWNED_PID=""

if ! kill -0 "$UNRELATED_PID" >/dev/null 2>&1; then
  echo "A process referenced by a mismatched PID file was killed." >&2
  exit 1
fi

if find "$TMP_ROOT/.pids" -type f -name '*.pid' -print -quit | grep -q .; then
  echo "PID files were not cleaned." >&2
  exit 1
fi

echo "STOP_RUSTCLAW_TESTS ok"
