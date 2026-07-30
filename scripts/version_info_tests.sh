#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT_DIR/scripts/version_info.sh"

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

mkdir -p "$TMP_ROOT/source" "$TMP_ROOT/release" "$TMP_ROOT/unknown"
cat > "$TMP_ROOT/source/Cargo.toml" <<'EOF'
[workspace.package]
version = "1.2.3"
edition = "2024"
EOF
printf '2.0.1\n' > "$TMP_ROOT/release/VERSION"

[[ "$(app_version_from_root "$TMP_ROOT/source")" == "1.2.3" ]]
[[ "$(app_version_from_root "$TMP_ROOT/release")" == "2.0.1" ]]
[[ "$(APP_VERSION=3.4.5 app_version_from_root "$TMP_ROOT/release")" == "3.4.5" ]]
[[ "$(APP_VERSION='invalid value' app_version_from_root "$TMP_ROOT/unknown")" == "unknown" ]]
[[ "$(app_version_from_root "$TMP_ROOT/unknown")" == "unknown" ]]

echo "VERSION_INFO_TESTS ok"
