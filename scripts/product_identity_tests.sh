#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

PRIMARY_FIXTURE="$REPO_ROOT/scripts/fixtures/product_identity/brand-primary.toml"
ALTERNATE_FIXTURE="$REPO_ROOT/scripts/fixtures/product_identity/brand-alternate.toml"

assert_shell_identity() {
  local fixture="$1"
  env -i \
    HOME="$HOME" \
    PATH="$PATH" \
    APP_PRODUCT_IDENTITY_CONFIG="$fixture" \
    APP_DISPLAY_NAME="ignored-environment-value" \
    APP_RELEASE_ARTIFACT_ID="ignored-environment-value" \
    bash -c '
    set -euo pipefail
    source "$1/scripts/product_identity.sh"
    python3 - "$APP_PRODUCT_IDENTITY_CONFIG" <<"PY"
import os
import sys
import tomllib
from pathlib import Path

config = tomllib.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
assert os.environ["APP_DISPLAY_NAME"] == config["display_name"]
assert os.environ["APP_RELEASE_ARTIFACT_ID"] == config["release_artifact_id"]
assert os.environ["APP_SERVICE_NAME"] == "agent-runtime"
assert os.environ["APP_DATA_NAMESPACE"] == "agent-runtime"
assert os.environ["APP_TERMINAL_BANNER"] == config["terminal_banner"].rstrip("\n")
assert os.environ["APP_RELEASE_REPOSITORY"] == config["release_repository"]
assert os.environ["APP_SMALL_SCREEN_SPLASH_IMAGE"] == config["small_screen_splash_image"]
PY
  ' _ "$REPO_ROOT"
}

assert_shell_identity "$PRIMARY_FIXTURE"
assert_shell_identity "$ALTERNATE_FIXTURE"

assert_ui_identity() {
  python3 - "$1" "$2" <<'PY'
import sys
import tomllib
from pathlib import Path

expected = tomllib.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))["display_name"].encode("utf-8")
output = Path(sys.argv[2])
if not expected or not any(
    expected in path.read_bytes() for path in output.rglob("*")
    if path.is_file() and not path.is_symlink()
):
    raise SystemExit("product_identity_ui_artifacts: configured identity missing")
PY
}

if [[ "${1:-}" == "--with-ui" ]]; then
  (
    cd "$REPO_ROOT/UI"
    APP_PRODUCT_IDENTITY_CONFIG="$PRIMARY_FIXTURE" \
      npm run build -- --outDir "$FIXTURE_DIR/ui-first"
    assert_ui_identity "$PRIMARY_FIXTURE" "$FIXTURE_DIR/ui-first"
    APP_PRODUCT_IDENTITY_CONFIG="$ALTERNATE_FIXTURE" \
      npm run build -- --outDir "$FIXTURE_DIR/ui-second"
    assert_ui_identity "$ALTERNATE_FIXTURE" "$FIXTURE_DIR/ui-second"
  )
fi

echo "product_identity_tests: PASS"
