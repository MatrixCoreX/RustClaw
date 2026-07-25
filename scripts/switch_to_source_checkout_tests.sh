#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rustclaw-source-checkout-test.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

SOURCE_REPO="$TMP_DIR/source"
RUNTIME_ROOT="$TMP_DIR/runtime"

git init --quiet --initial-branch=main "$SOURCE_REPO"
git -C "$SOURCE_REPO" config user.name "RustClaw Test"
git -C "$SOURCE_REPO" config user.email "test@rustclaw.local"
mkdir -p "$SOURCE_REPO/UI" "$SOURCE_REPO/scripts" "$SOURCE_REPO/configs"
printf '%s\n' '[workspace]' > "$SOURCE_REPO/Cargo.toml"
printf '%s\n' '{"name":"rustclaw-test"}' > "$SOURCE_REPO/UI/package.json"
printf '%s\n' '#!/usr/bin/env bash' > "$SOURCE_REPO/start-all-bin.sh"
printf '%s\n' '#!/usr/bin/env bash' > "$SOURCE_REPO/scripts/switch-to-source-checkout.sh"
printf '%s\n' 'source_default=true' > "$SOURCE_REPO/configs/config.toml"
printf '%s\n' 'source checkout content' > "$SOURCE_REPO/README.md"
chmod +x "$SOURCE_REPO/start-all-bin.sh" "$SOURCE_REPO/scripts/switch-to-source-checkout.sh"
git -C "$SOURCE_REPO" add .
git -C "$SOURCE_REPO" commit --quiet -m "source fixture"

mkdir -p \
  "$RUNTIME_ROOT/configs" \
  "$RUNTIME_ROOT/data" \
  "$RUNTIME_ROOT/logs" \
  "$RUNTIME_ROOT/.pids" \
  "$RUNTIME_ROOT/target/release" \
  "$RUNTIME_ROOT/UI/dist"
printf '%s\n' 'local_config=true' > "$RUNTIME_ROOT/configs/config.toml"
printf '%s\n' 'state' > "$RUNTIME_ROOT/data/state.db"
printf '%s\n' 'runtime log' > "$RUNTIME_ROOT/logs/clawd.log"
printf '%s\n' '123' > "$RUNTIME_ROOT/.pids/clawd.pid"
printf '%s\n' '#!/usr/bin/env bash' > "$RUNTIME_ROOT/target/release/clawd"
printf '%s\n' '<html>runtime UI</html>' > "$RUNTIME_ROOT/UI/dist/index.html"
printf '%s\n' 'ubuntu-x86_64-test' > "$RUNTIME_ROOT/.release-tag"
chmod +x "$RUNTIME_ROOT/target/release/clawd"

"$ROOT_DIR/scripts/switch-to-source-checkout.sh" \
  --root "$RUNTIME_ROOT" \
  --repo "$SOURCE_REPO" \
  --branch main

test -d "$RUNTIME_ROOT/.git"
test -f "$RUNTIME_ROOT/Cargo.toml"
test ! -e "$RUNTIME_ROOT/.release-tag"
grep -Fqx 'local_config=true' "$RUNTIME_ROOT/configs/config.toml"
grep -Fqx 'state' "$RUNTIME_ROOT/data/state.db"
grep -Fqx '<html>runtime UI</html>' "$RUNTIME_ROOT/UI/dist/index.html"
test -x "$RUNTIME_ROOT/target/release/clawd"
test "$(find "$TMP_DIR/.runtime-source-backups" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 1

SECOND_OUTPUT="$(
  "$ROOT_DIR/scripts/switch-to-source-checkout.sh" \
    --root "$RUNTIME_ROOT" \
    --repo "$SOURCE_REPO" \
    --branch main
)"
grep -Fq 'source_checkout_status=already_enabled' <<<"$SECOND_OUTPUT"

INVALID_REPO="$TMP_DIR/invalid-source"
UNCHANGED_ROOT="$TMP_DIR/unchanged-runtime"
git init --quiet --initial-branch=main "$INVALID_REPO"
git -C "$INVALID_REPO" config user.name "RustClaw Test"
git -C "$INVALID_REPO" config user.email "test@rustclaw.local"
printf '%s\n' 'incomplete source' > "$INVALID_REPO/README.md"
git -C "$INVALID_REPO" add README.md
git -C "$INVALID_REPO" commit --quiet -m "invalid source fixture"
mkdir -p "$UNCHANGED_ROOT/configs"
printf '%s\n' 'local_config=true' > "$UNCHANGED_ROOT/configs/config.toml"
printf '%s\n' 'ubuntu-x86_64-test' > "$UNCHANGED_ROOT/.release-tag"

if "$ROOT_DIR/scripts/switch-to-source-checkout.sh" \
  --root "$UNCHANGED_ROOT" \
  --repo "$INVALID_REPO" \
  --branch main >/dev/null 2>&1; then
  printf '%s\n' "invalid source unexpectedly succeeded" >&2
  exit 1
fi
test -f "$UNCHANGED_ROOT/.release-tag"
test ! -e "$UNCHANGED_ROOT/.git"
grep -Fqx 'local_config=true' "$UNCHANGED_ROOT/configs/config.toml"

printf '%s\n' "switch_to_source_checkout_tests: ok"
