#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

WORKSPACE="$TMP_ROOT/runtime space"
BIN_DIR="$TMP_ROOT/bin"
SYSTEMD_RUNTIME="$TMP_ROOT/run/systemd/system"
SYSTEMD_UNITS="$TMP_ROOT/etc/systemd/system"
SYSTEMCTL_STATE="$TMP_ROOT/systemctl-state"
mkdir -p "$WORKSPACE/configs" "$BIN_DIR" "$SYSTEMD_RUNTIME" "$SYSTEMD_UNITS"
touch "$WORKSPACE/start-all-bin.sh" "$WORKSPACE/stop-agent.sh" "$WORKSPACE/configs/config.toml"

cat > "$BIN_DIR/uname" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  -s) printf '%s\n' "${MOCK_OS:-Linux}" ;;
  -m) printf '%s\n' "${MOCK_ARCH:-x86_64}" ;;
  *) printf '%s\n' "${MOCK_OS:-Linux}" ;;
esac
EOF

cat > "$BIN_DIR/sudo" <<'EOF'
#!/usr/bin/env bash
if [[ "${1:-}" == "-n" ]]; then
  shift
  [[ "${MOCK_SUDO_ALLOWED:-1}" == "1" ]] || exit 1
fi
exec "$@"
EOF

cat > "$BIN_DIR/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
state="${MOCK_SYSTEMCTL_STATE:?}"
user_scope=0
if [[ "${1:-}" == "--user" ]]; then
  user_scope=1
  shift
fi
command="${1:-}"
shift || true
unit="${1:-agent-runtime.service}"
key="system"
[[ "$user_scope" == "0" ]] || key="user"
case "$command" in
  is-enabled) [[ -f "$state/$key-enabled" ]] ;;
  enable) mkdir -p "$state"; touch "$state/$key-enabled" ;;
  disable) rm -f "$state/$key-enabled" ;;
  daemon-reload|restart) : ;;
  *) : ;;
esac
EOF

cat > "$BIN_DIR/loginctl" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF

cat > "$BIN_DIR/launchctl" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF

chmod +x "$BIN_DIR"/*

COMMON_ENV=(
  "PATH=$BIN_DIR:$PATH"
  "APP_SYSTEMD_RUNTIME_DIR=$SYSTEMD_RUNTIME"
  "APP_SYSTEMD_UNIT_DIR=$SYSTEMD_UNITS"
  "MOCK_SYSTEMCTL_STATE=$SYSTEMCTL_STATE"
  "HOME=$TMP_ROOT/home"
)

env "${COMMON_ENV[@]}" MOCK_OS=Linux MOCK_SUDO_ALLOWED=1 \
  bash "$SCRIPT_DIR/configure-autostart.sh" \
    --enable --workspace "$WORKSPACE" --non-interactive \
  | grep -Fq 'autostart=enabled manager=systemd-system'
test -f "$SYSTEMD_UNITS/agent-runtime.service"
grep -Fq "ExecStart=/bin/bash \"$WORKSPACE/start-all-bin.sh\" release" \
  "$SYSTEMD_UNITS/agent-runtime.service"
env "${COMMON_ENV[@]}" MOCK_OS=Linux \
  bash "$SCRIPT_DIR/configure-autostart.sh" --status --workspace "$WORKSPACE" \
  | grep -Fq 'autostart=enabled manager=systemd-system'
env "${COMMON_ENV[@]}" MOCK_OS=Linux MOCK_SUDO_ALLOWED=1 \
  bash "$SCRIPT_DIR/configure-autostart.sh" \
    --disable --workspace "$WORKSPACE" --non-interactive \
  | grep -Fq 'autostart=disabled manager=systemd'
test ! -e "$SYSTEMD_UNITS/agent-runtime.service"

rm -rf "$SYSTEMCTL_STATE"
env "${COMMON_ENV[@]}" MOCK_OS=Linux MOCK_SUDO_ALLOWED=0 \
  bash "$SCRIPT_DIR/configure-autostart.sh" \
    --enable --workspace "$WORKSPACE" --non-interactive \
  | grep -Fq 'autostart=enabled manager=systemd-user'
test -f "$TMP_ROOT/home/.config/systemd/user/agent-runtime.service"

rm -rf "$TMP_ROOT/home/Library"
env "${COMMON_ENV[@]}" MOCK_OS=Darwin \
  bash "$SCRIPT_DIR/configure-autostart.sh" --enable --workspace "$WORKSPACE" \
  | grep -Fq 'autostart=enabled manager=launchd'
PLIST="$TMP_ROOT/home/Library/LaunchAgents/agent-runtime.plist"
test -f "$PLIST"
python3 - "$PLIST" "$WORKSPACE" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "rb") as stream:
    value = plistlib.load(stream)
assert value["Label"] == "agent-runtime"
assert value["WorkingDirectory"] == sys.argv[2]
assert value["ProgramArguments"] == ["/bin/bash", f"{sys.argv[2]}/start-all-bin.sh", "release"]
assert value["EnvironmentVariables"]["APP_AUTOSTART_MANAGED"] == "1"
PY
env "${COMMON_ENV[@]}" MOCK_OS=Darwin \
  bash "$SCRIPT_DIR/configure-autostart.sh" --disable --workspace "$WORKSPACE" \
  | grep -Fq 'autostart=disabled manager=launchd'
test ! -e "$PLIST"

echo "CONFIGURE_AUTOSTART_TESTS ok"
