#!/bin/bash
set -eu

PAYLOAD_DIR="/Library/Application Support/agent-runtime-installer"
PAYLOAD_ARCHIVE="$PAYLOAD_DIR/agent-desktop.app.zip"
PAYLOAD_CHECKSUM="$PAYLOAD_ARCHIVE.sha256"
DESTINATION="/Applications/agent-desktop.app"
WORK_DIR="$(mktemp -d /Applications/.agent-desktop.install.XXXXXX)"
BACKUP="/Applications/.agent-desktop.previous.$$"
HAD_PREVIOUS=0
INSTALLED_NEW=0

cleanup() {
  status=$?
  trap - EXIT
  /bin/rm -rf "$WORK_DIR"
  if [[ "$status" -ne 0 ]]; then
    if [[ "$INSTALLED_NEW" -eq 1 ]]; then
      /bin/rm -rf "$DESTINATION"
    fi
    if [[ "$HAD_PREVIOUS" -eq 1 && -e "$BACKUP" ]]; then
      /bin/mv "$BACKUP" "$DESTINATION"
    fi
  else
    /bin/rm -rf "$BACKUP"
    /bin/rm -f "$PAYLOAD_ARCHIVE" "$PAYLOAD_CHECKSUM"
    /bin/rmdir "$PAYLOAD_DIR" >/dev/null 2>&1 || true
  fi
  exit "$status"
}
trap cleanup EXIT

[[ -f "$PAYLOAD_ARCHIVE" && -f "$PAYLOAD_CHECKSUM" ]]
expected_checksum="$(/usr/bin/awk '{print $1}' "$PAYLOAD_CHECKSUM")"
actual_checksum="$(/usr/bin/shasum -a 256 "$PAYLOAD_ARCHIVE" | /usr/bin/awk '{print $1}')"
[[ -n "$expected_checksum" && "$actual_checksum" == "$expected_checksum" ]]

/usr/bin/ditto -x -k "$PAYLOAD_ARCHIVE" "$WORK_DIR"
CANDIDATE="$WORK_DIR/agent-desktop.app"
[[ -x "$CANDIDATE/Contents/MacOS/agent-desktop" ]]
[[ -f "$CANDIDATE/Contents/Resources/embedded-runtime/bundle.json" ]]
/usr/bin/codesign --verify --deep --strict "$CANDIDATE"

if [[ -e "$DESTINATION" ]]; then
  /bin/mv "$DESTINATION" "$BACKUP"
  HAD_PREVIOUS=1
fi
/bin/mv "$CANDIDATE" "$DESTINATION"
INSTALLED_NEW=1
/usr/bin/codesign --verify --deep --strict "$DESTINATION"
