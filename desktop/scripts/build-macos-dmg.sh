#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# shellcheck source=/dev/null
source "${REPO_ROOT}/scripts/product_identity.sh"

if [[ "$#" -ne 2 ]]; then
  echo "Usage: build-macos-dmg.sh APP_BUNDLE OUTPUT_DMG" >&2
  exit 2
fi

APP_BUNDLE="$1"
OUTPUT_DMG="$2"

[[ "$(uname -s)" == "Darwin" ]] || {
  echo "The macOS DMG must be built on macOS." >&2
  exit 1
}
[[ -d "$APP_BUNDLE" && "$APP_BUNDLE" == *.app ]] || {
  echo "Missing macOS app bundle: $APP_BUNDLE" >&2
  exit 1
}
for command_name in codesign ditto hdiutil python3; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "Required macOS packaging command is unavailable: $command_name" >&2
    exit 1
  }
done

OUTPUT_DMG="$(python3 - "$OUTPUT_DMG" <<'PY'
from pathlib import Path
import sys

print(Path(sys.argv[1]).expanduser().resolve())
PY
)"
[[ "$OUTPUT_DMG" == *.dmg ]] || {
  echo "The output path must end in .dmg: $OUTPUT_DMG" >&2
  exit 1
}

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
STAGING_DIR="$WORK_DIR/dmg-root"
APP_BUNDLE_NAME="$(basename "$APP_BUNDLE")"

codesign --verify --deep --strict "$APP_BUNDLE"
mkdir -p "$STAGING_DIR" "$(dirname "$OUTPUT_DMG")"
ditto "$APP_BUNDLE" "$STAGING_DIR/$APP_BUNDLE_NAME"
ln -s /Applications "$STAGING_DIR/Applications"
rm -f "$OUTPUT_DMG"
DMG_CREATE_ATTEMPTS="${APP_DMG_CREATE_ATTEMPTS:-4}"
case "$DMG_CREATE_ATTEMPTS" in
  ''|*[!0-9]*|0)
    echo "APP_DMG_CREATE_ATTEMPTS must be a positive integer." >&2
    exit 2
    ;;
esac

dmg_created=0
for ((attempt = 1; attempt <= DMG_CREATE_ATTEMPTS; attempt++)); do
  rm -f "$OUTPUT_DMG" "$OUTPUT_DMG".*
  sync
  if hdiutil create \
    -volname "$APP_DISPLAY_NAME" \
    -srcfolder "$STAGING_DIR" \
    -ov \
    -format UDZO \
    "$OUTPUT_DMG"; then
    dmg_created=1
    break
  fi
  if ((attempt < DMG_CREATE_ATTEMPTS)); then
    delay=$((attempt * 5))
    echo "DMG creation attempt ${attempt}/${DMG_CREATE_ATTEMPTS} failed; retrying in ${delay}s." >&2
    sleep "$delay"
  fi
done
[[ "$dmg_created" -eq 1 ]] || {
  echo "DMG creation failed after ${DMG_CREATE_ATTEMPTS} attempts." >&2
  exit 1
}
hdiutil verify "$OUTPUT_DMG"
printf '%s\n' "$OUTPUT_DMG"
