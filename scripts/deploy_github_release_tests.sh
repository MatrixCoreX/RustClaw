#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY_SCRIPT="$ROOT_DIR/deploy-github-release.sh"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

PACKAGE_DIR="$TMP_ROOT/package/RustClaw"
mkdir -p \
  "$PACKAGE_DIR/target/release" \
  "$PACKAGE_DIR/configs/channels" \
  "$PACKAGE_DIR/UI/dist"
cp /bin/true "$PACKAGE_DIR/target/release/clawd"
printf 'new release readme\n' > "$PACKAGE_DIR/README.md"
printf 'new-default = true\n' > "$PACKAGE_DIR/configs/new-default.toml"
printf 'release-channel = true\n' > "$PACKAGE_DIR/configs/channels/release.toml"
printf '<!doctype html><title>release ui</title>\n' > "$PACKAGE_DIR/UI/dist/index.html"

ARCHIVE="$TMP_ROOT/RustClaw-ubuntu-x86_64-test.tar.gz"
tar -czf "$ARCHIVE" -C "$TMP_ROOT/package" RustClaw
ARCHIVE_HASH="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
CHECKSUM="$ARCHIVE.sha256"
printf '%s  %s\n' "$ARCHIVE_HASH" "$(basename "$ARCHIVE")" > "$CHECKSUM"

RELEASES_JSON="$TMP_ROOT/releases.json"
python3 - "$RELEASES_JSON" "$ARCHIVE" "$CHECKSUM" <<'PY'
import json
from pathlib import Path
import sys

output, archive, checksum = sys.argv[1:]
archive = Path(archive).resolve()
checksum = Path(checksum).resolve()
releases = [
    {
        "tag_name": "pi-aarch64-newer-but-wrong-platform",
        "draft": False,
        "prerelease": False,
        "assets": [],
    },
    {
        "tag_name": "ubuntu-x86_64-test",
        "draft": False,
        "prerelease": False,
        "assets": [
            {
                "name": archive.name,
                "browser_download_url": archive.as_uri(),
            },
            {
                "name": checksum.name,
                "browser_download_url": checksum.as_uri(),
            },
        ],
    },
]
Path(output).write_text(json.dumps(releases), encoding="utf-8")
PY

RUNTIME="$TMP_ROOT/runtime"
mkdir -p "$RUNTIME/configs" "$RUNTIME/target/release"
cp /bin/false "$RUNTIME/target/release/clawd"
printf 'local-secret = "preserve"\n' > "$RUNTIME/configs/config.toml"
printf 'old readme\n' > "$RUNTIME/README.md"
printf 'ubuntu-x86_64-old\n' > "$RUNTIME/.release-tag"

mkdir "$RUNTIME/.release-deploy.lock"
printf '%s\n' "$$" > "$RUNTIME/.release-deploy.lock/pid"
if RUSTCLAW_RELEASES_JSON_FILE="$RELEASES_JSON" \
  "$DEPLOY_SCRIPT" \
    --root "$RUNTIME" \
    --platform ubuntu-x86_64 \
    --check-only >/dev/null 2>&1; then
  echo "concurrent deployment lock unexpectedly succeeded" >&2
  exit 1
fi
[[ -d "$RUNTIME/.release-deploy.lock" ]]
rm -rf "$RUNTIME/.release-deploy.lock"
mkdir "$RUNTIME/.release-deploy.lock"
printf 'stale\n' > "$RUNTIME/.release-deploy.lock/pid"

CHECK_OUTPUT="$(
  RUSTCLAW_RELEASES_JSON_FILE="$RELEASES_JSON" \
    "$DEPLOY_SCRIPT" \
      --root "$RUNTIME" \
      --platform ubuntu-x86_64 \
      --check-only
)"
grep -Fq 'release_tag=ubuntu-x86_64-test' <<< "$CHECK_OUTPUT"
grep -Fq 'release_update_status=available' <<< "$CHECK_OUTPUT"
[[ ! -e "$RUNTIME/.release-deploy.lock" ]]

OUTPUT="$(
  RUSTCLAW_RELEASES_JSON_FILE="$RELEASES_JSON" \
    "$DEPLOY_SCRIPT" \
      --root "$RUNTIME" \
      --platform ubuntu-x86_64 \
      --no-restart
)"
grep -Fq 'release_checksum=verified' <<< "$OUTPUT"
grep -Fq 'release_update_status=deployed' <<< "$OUTPUT"
grep -Fxq 'ubuntu-x86_64-test' "$RUNTIME/.release-tag"
grep -Fxq 'local-secret = "preserve"' "$RUNTIME/configs/config.toml"
grep -Fxq 'new-default = true' "$RUNTIME/configs/new-default.toml"
grep -Fxq 'new release readme' "$RUNTIME/README.md"
cmp /bin/true "$RUNTIME/target/release/clawd"
find "$RUNTIME/.release-backups" -name files.tar.gz -type f | grep -q .
ROLLBACK_MARKER_BEFORE="$(cat "$RUNTIME/.release-rollback")"

BAD_CHECKSUM="$TMP_ROOT/bad.sha256"
printf '%064d  %s\n' 0 "$(basename "$ARCHIVE")" > "$BAD_CHECKSUM"
BAD_RELEASES_JSON="$TMP_ROOT/bad-releases.json"
python3 - "$BAD_RELEASES_JSON" "$ARCHIVE" "$BAD_CHECKSUM" <<'PY'
import json
from pathlib import Path
import sys

output, archive, checksum = sys.argv[1:]
archive = Path(archive).resolve()
checksum = Path(checksum).resolve()
Path(output).write_text(
    json.dumps(
        [
            {
                "tag_name": "ubuntu-x86_64-bad",
                "draft": False,
                "prerelease": False,
                "assets": [
                    {"name": archive.name, "browser_download_url": archive.as_uri()},
                    {
                        "name": f"{archive.name}.sha256",
                        "browser_download_url": checksum.as_uri(),
                    },
                ],
            }
        ]
    ),
    encoding="utf-8",
)
PY

BEFORE_HASH="$(sha256sum "$RUNTIME/target/release/clawd" | awk '{print $1}')"
if RUSTCLAW_RELEASES_JSON_FILE="$BAD_RELEASES_JSON" \
  "$DEPLOY_SCRIPT" \
    --root "$RUNTIME" \
    --platform ubuntu-x86_64 \
    --tag ubuntu-x86_64-bad \
    --no-restart >/dev/null 2>&1; then
  echo "checksum mismatch deployment unexpectedly succeeded" >&2
  exit 1
fi
AFTER_HASH="$(sha256sum "$RUNTIME/target/release/clawd" | awk '{print $1}')"
[[ "$BEFORE_HASH" == "$AFTER_HASH" ]]
grep -Fxq 'ubuntu-x86_64-test' "$RUNTIME/.release-tag"

FAIL_STAGE="$TMP_ROOT/fail-stage"
FAIL_PACKAGE_ROOT="$FAIL_STAGE/RustClaw"
mkdir -p "$FAIL_STAGE"
cp -a "$PACKAGE_DIR" "$FAIL_PACKAGE_ROOT"
cp /bin/false "$FAIL_PACKAGE_ROOT/target/release/clawd"
cat > "$FAIL_PACKAGE_ROOT/build-ui-nginx.sh" <<'EOF'
#!/usr/bin/env bash
exit 9
EOF
chmod +x "$FAIL_PACKAGE_ROOT/build-ui-nginx.sh"
FAIL_ARCHIVE="$TMP_ROOT/RustClaw-ubuntu-x86_64-rollback.tar.gz"
tar -czf "$FAIL_ARCHIVE" -C "$FAIL_STAGE" RustClaw
FAIL_HASH="$(sha256sum "$FAIL_ARCHIVE" | awk '{print $1}')"
FAIL_CHECKSUM="$FAIL_ARCHIVE.sha256"
printf '%s  %s\n' "$FAIL_HASH" "$(basename "$FAIL_ARCHIVE")" > "$FAIL_CHECKSUM"
FAIL_RELEASES_JSON="$TMP_ROOT/fail-releases.json"
python3 - "$FAIL_RELEASES_JSON" "$FAIL_ARCHIVE" "$FAIL_CHECKSUM" <<'PY'
import json
from pathlib import Path
import sys

output, archive, checksum = sys.argv[1:]
archive = Path(archive).resolve()
checksum = Path(checksum).resolve()
Path(output).write_text(
    json.dumps(
        [
            {
                "tag_name": "ubuntu-x86_64-rollback",
                "draft": False,
                "prerelease": False,
                "assets": [
                    {"name": archive.name, "browser_download_url": archive.as_uri()},
                    {"name": checksum.name, "browser_download_url": checksum.as_uri()},
                ],
            }
        ]
    ),
    encoding="utf-8",
)
PY

if RUSTCLAW_RELEASES_JSON_FILE="$FAIL_RELEASES_JSON" \
  "$DEPLOY_SCRIPT" \
    --root "$RUNTIME" \
    --platform ubuntu-x86_64 \
    --no-restart >/dev/null 2>&1; then
  echo "post-copy failure deployment unexpectedly succeeded" >&2
  exit 1
fi
cmp /bin/true "$RUNTIME/target/release/clawd"
grep -Fxq 'ubuntu-x86_64-test' "$RUNTIME/.release-tag"
grep -Fxq 'new release readme' "$RUNTIME/README.md"
[[ ! -e "$RUNTIME/build-ui-nginx.sh" ]]
[[ "$(cat "$RUNTIME/.release-rollback")" == "$ROLLBACK_MARKER_BEFORE" ]]

echo "DEPLOY_GITHUB_RELEASE_TESTS ok"
