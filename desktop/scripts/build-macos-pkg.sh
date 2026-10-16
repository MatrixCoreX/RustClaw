#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# shellcheck source=/dev/null
source "${REPO_ROOT}/scripts/product_identity.sh"

usage() {
  cat >&2 <<'EOF'
Usage: build-macos-pkg.sh APP_BUNDLE OUTPUT_PKG

Builds a macOS product archive that installs the all-in-one desktop app in
/Applications. The app must already contain the embedded signed Release bundle.
EOF
}

if [[ "$#" -ne 2 ]]; then
  usage
  exit 2
fi

APP_BUNDLE="$1"
OUTPUT_PKG="$2"

[[ "$(uname -s)" == "Darwin" ]] || {
  echo "The macOS PKG must be built on macOS." >&2
  exit 1
}
[[ -d "${APP_BUNDLE}" && "${APP_BUNDLE}" == *.app ]] || {
  echo "Missing macOS app bundle: ${APP_BUNDLE}" >&2
  exit 1
}
[[ -f "${APP_BUNDLE}/Contents/Resources/embedded-runtime/bundle.json" ]] || {
  echo "The app does not contain an embedded runtime descriptor." >&2
  exit 1
}
[[ -d "${APP_BUNDLE}/Contents/Resources/embedded-runtime/release" ]] || {
  echo "The app does not contain an embedded signed Release." >&2
  exit 1
}

for command_name in pkgbuild productbuild pkgutil codesign ditto python3 shasum; do
  command -v "${command_name}" >/dev/null 2>&1 || {
    echo "Required macOS packaging command is unavailable: ${command_name}" >&2
    exit 1
  }
done

package_metadata="$(python3 - "${REPO_ROOT}/desktop/tauri.conf.json" <<'PY'
import json
import re
import sys
from pathlib import Path

config = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
identifier = config.get("identifier")
version = config.get("version")
if not isinstance(identifier, str) or not re.fullmatch(r"[A-Za-z0-9.-]+", identifier):
    raise SystemExit("desktop identifier is missing or invalid")
if not isinstance(version, str) or not re.fullmatch(r"[0-9]+(?:\.[0-9]+){1,3}", version):
    raise SystemExit("desktop version is missing or invalid")
print(f"{identifier}\t{version}")
PY
)"

IFS=$'\t' read -r DESKTOP_IDENTIFIER DESKTOP_VERSION <<< "${package_metadata}"
[[ -n "${DESKTOP_IDENTIFIER}" && -n "${DESKTOP_VERSION}" ]]

OUTPUT_PKG="$(python3 - "${OUTPUT_PKG}" <<'PY'
from pathlib import Path
import sys

print(Path(sys.argv[1]).expanduser().resolve())
PY
)"
mkdir -p "$(dirname "${OUTPUT_PKG}")"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT
PAYLOAD_ROOT="${WORK_DIR}/payload-root"
PAYLOAD_DIR="${PAYLOAD_ROOT}/Library/Application Support/agent-runtime-installer"
SCRIPTS_DIR="${WORK_DIR}/scripts"
COMPONENT_PKG="${WORK_DIR}/desktop-component.pkg"
UNSIGNED_PKG="${WORK_DIR}/installer-unsigned.pkg"
APP_BUNDLE_NAME="$(basename "${APP_BUNDLE}")"
APP_ARCHIVE="${PAYLOAD_DIR}/agent-desktop.app.zip"

[[ "${APP_BUNDLE_NAME}" == "agent-desktop.app" ]] || {
  echo "Unexpected canonical macOS app bundle name: ${APP_BUNDLE_NAME}" >&2
  exit 1
}

codesign --verify --deep --strict "${APP_BUNDLE}"
mkdir -p "${PAYLOAD_DIR}" "${SCRIPTS_DIR}"
ditto -c -k --sequesterRsrc --keepParent "${APP_BUNDLE}" "${APP_ARCHIVE}"
(
  cd "${PAYLOAD_DIR}"
  shasum -a 256 "$(basename "${APP_ARCHIVE}")" > "$(basename "${APP_ARCHIVE}").sha256"
)
cp "${SCRIPT_DIR}/macos-pkg-postinstall.sh" "${SCRIPTS_DIR}/postinstall"
chmod 0755 "${SCRIPTS_DIR}/postinstall"

pkgbuild \
  --root "${PAYLOAD_ROOT}" \
  --install-location / \
  --scripts "${SCRIPTS_DIR}" \
  --identifier "${DESKTOP_IDENTIFIER}.runtime" \
  --version "${DESKTOP_VERSION}" \
  --ownership recommended \
  "${COMPONENT_PKG}"

pkgutil --payload-files "${COMPONENT_PKG}" > "${WORK_DIR}/payload-files.txt"
grep -Fq "agent-runtime-installer/agent-desktop.app.zip" "${WORK_DIR}/payload-files.txt" || {
  echo "PKG payload is missing the macOS app archive." >&2
  exit 1
}
grep -Fq "agent-runtime-installer/agent-desktop.app.zip.sha256" \
  "${WORK_DIR}/payload-files.txt" || {
  echo "PKG payload is missing the macOS app archive checksum." >&2
  exit 1
}

productbuild --package "${COMPONENT_PKG}" "${UNSIGNED_PKG}"

if [[ -n "${APP_MACOS_INSTALLER_SIGNING_IDENTITY:-}" ]]; then
  command -v productsign >/dev/null 2>&1 || {
    echo "productsign is required when APP_MACOS_INSTALLER_SIGNING_IDENTITY is set." >&2
    exit 1
  }
  productsign \
    --sign "${APP_MACOS_INSTALLER_SIGNING_IDENTITY}" \
    "${UNSIGNED_PKG}" \
    "${OUTPUT_PKG}"
else
  cp "${UNSIGNED_PKG}" "${OUTPUT_PKG}"
fi

pkgutil --expand "${OUTPUT_PKG}" "${WORK_DIR}/expanded"
[[ -f "${WORK_DIR}/expanded/Distribution" ]] || {
  echo "Expanded product archive is missing Distribution metadata." >&2
  exit 1
}
component_found=0
for expanded_component in "${WORK_DIR}/expanded"/*.pkg; do
  if [[ -d "${expanded_component}" ]]; then
    component_found=1
    break
  fi
done
[[ "${component_found}" -eq 1 ]] || {
  echo "Expanded product archive is missing its component package." >&2
  exit 1
}

(
  cd "$(dirname "${OUTPUT_PKG}")"
  shasum -a 256 "$(basename "${OUTPUT_PKG}")" \
    > "$(basename "${OUTPUT_PKG}").sha256"
)
printf '%s\n' "${OUTPUT_PKG}"
