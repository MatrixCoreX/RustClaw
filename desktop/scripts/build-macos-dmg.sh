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
hdiutil create \
  -volname "$APP_DISPLAY_NAME" \
  -srcfolder "$STAGING_DIR" \
  -ov \
  -format UDZO \
  "$OUTPUT_DMG"
hdiutil verify "$OUTPUT_DMG"
printf '%s\n' "$OUTPUT_DMG"
