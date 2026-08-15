#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY_SCRIPT="$ROOT_DIR/deploy-github-release.sh"
# shellcheck source=/dev/null
source "$ROOT_DIR/scripts/product_identity.sh"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

PACKAGE_DIR="$TMP_ROOT/package/$APP_RELEASE_ARTIFACT_ID"
mkdir -p \
  "$PACKAGE_DIR/target/release" \
  "$PACKAGE_DIR/configs/channels" \
  "$PACKAGE_DIR/crates/skills/core_fixture" \
  "$PACKAGE_DIR/optional_skills/store_fixture" \
  "$PACKAGE_DIR/data/skill-packages/core_fixture" \
  "$PACKAGE_DIR/prebuilt/skill-packages/store_fixture" \
  "$PACKAGE_DIR/UI/dist"
cp /bin/true "$PACKAGE_DIR/target/release/clawd"
printf 'new release readme\n' > "$PACKAGE_DIR/README.md"
printf '9.8.7\n' > "$PACKAGE_DIR/VERSION"
printf 'new-default = true\n' > "$PACKAGE_DIR/configs/new-default.toml"
printf 'release-channel = true\n' > "$PACKAGE_DIR/configs/channels/release.toml"
printf 'name = "core_fixture"\n' > "$PACKAGE_DIR/crates/skills/core_fixture/skill.toml"
printf 'name = "store_fixture"\n' > "$PACKAGE_DIR/optional_skills/store_fixture/skill.toml"
printf 'release receipt\n' > "$PACKAGE_DIR/data/skill-packages/core_fixture/current.json"
printf 'release prebuilt\n' > "$PACKAGE_DIR/prebuilt/skill-packages/store_fixture/current.json"
printf '<!doctype html><title>release ui</title>\n' > "$PACKAGE_DIR/UI/dist/index.html"

ARCHIVE="$TMP_ROOT/$APP_RELEASE_ARTIFACT_ID-ubuntu-x86_64-test.tar.gz"
tar -czf "$ARCHIVE" -C "$TMP_ROOT/package" "$APP_RELEASE_ARTIFACT_ID"
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
                "url": archive.as_uri(),
                "browser_download_url": "file:///api-asset-preference-must-win/archive",
            },
            {
                "name": checksum.name,
                "url": checksum.as_uri(),
                "browser_download_url": "file:///api-asset-preference-must-win/checksum",
            },
        ],
    },
]
Path(output).write_text(json.dumps(releases), encoding="utf-8")
PY

RUNTIME="$TMP_ROOT/runtime"
mkdir -p \
  "$RUNTIME/configs" \
  "$RUNTIME/data/skill-packages/local_optional" \
  "$RUNTIME/target/release"
cp /bin/false "$RUNTIME/target/release/clawd"
printf 'local-secret = "preserve"\n' > "$RUNTIME/configs/config.toml"
printf 'old readme\n' > "$RUNTIME/README.md"
printf 'keep local optional\n' > "$RUNTIME/data/skill-packages/local_optional/current.json"
printf 'ubuntu-x86_64-old\n' > "$RUNTIME/.release-tag"

mkdir "$RUNTIME/.release-deploy.lock"
printf '%s\n' "$$" > "$RUNTIME/.release-deploy.lock/pid"
if APP_RELEASES_JSON_FILE="$RELEASES_JSON" \
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
  APP_RELEASES_JSON_FILE="$RELEASES_JSON" \
    "$DEPLOY_SCRIPT" \
      --root "$RUNTIME" \
      --platform ubuntu-x86_64 \
      --check-only
)"
grep -Fq 'release_tag=ubuntu-x86_64-test' <<< "$CHECK_OUTPUT"
grep -Fq 'release_update_status=available' <<< "$CHECK_OUTPUT"
[[ ! -e "$RUNTIME/.release-deploy.lock" ]]

OUTPUT="$(
  APP_RELEASES_JSON_FILE="$RELEASES_JSON" \
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
grep -Fxq '9.8.7' "$RUNTIME/VERSION"
grep -Fxq 'name = "core_fixture"' "$RUNTIME/crates/skills/core_fixture/skill.toml"
grep -Fxq 'name = "store_fixture"' "$RUNTIME/optional_skills/store_fixture/skill.toml"
grep -Fxq 'release receipt' "$RUNTIME/data/skill-packages/core_fixture/current.json"
grep -Fxq 'keep local optional' "$RUNTIME/data/skill-packages/local_optional/current.json"
grep -Fxq 'release prebuilt' "$RUNTIME/prebuilt/skill-packages/store_fixture/current.json"
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
if APP_RELEASES_JSON_FILE="$BAD_RELEASES_JSON" \
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
FAIL_PACKAGE_ROOT="$FAIL_STAGE/$APP_RELEASE_ARTIFACT_ID"
mkdir -p "$FAIL_STAGE"
cp -a "$PACKAGE_DIR" "$FAIL_PACKAGE_ROOT"
cp /bin/false "$FAIL_PACKAGE_ROOT/target/release/clawd"
cat > "$FAIL_PACKAGE_ROOT/build-ui-nginx.sh" <<'EOF'
#!/usr/bin/env bash
exit 9
EOF
chmod +x "$FAIL_PACKAGE_ROOT/build-ui-nginx.sh"
FAIL_ARCHIVE="$TMP_ROOT/$APP_RELEASE_ARTIFACT_ID-ubuntu-x86_64-rollback.tar.gz"
tar -czf "$FAIL_ARCHIVE" -C "$FAIL_STAGE" "$APP_RELEASE_ARTIFACT_ID"
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

if APP_RELEASES_JSON_FILE="$FAIL_RELEASES_JSON" \
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
grep -Fxq '9.8.7' "$RUNTIME/VERSION"
[[ ! -e "$RUNTIME/build-ui-nginx.sh" ]]
[[ "$(cat "$RUNTIME/.release-rollback")" == "$ROLLBACK_MARKER_BEFORE" ]]

SOURCE_RUNTIME="$TMP_ROOT/source-runtime"
mkdir -p \
  "$SOURCE_RUNTIME/.git" \
  "$SOURCE_RUNTIME/configs" \
  "$SOURCE_RUNTIME/data" \
  "$SOURCE_RUNTIME/logs" \
  "$SOURCE_RUNTIME/.pids" \
  "$SOURCE_RUNTIME/.agent-runtime" \
  "$SOURCE_RUNTIME/run" \
  "$SOURCE_RUNTIME/external_skills/local" \
  "$SOURCE_RUNTIME/optional_skills/local" \
  "$SOURCE_RUNTIME/target/release"
printf 'source-only\n' > "$SOURCE_RUNTIME/Cargo.toml"
printf 'local-secret = "preserve"\n' > "$SOURCE_RUNTIME/configs/config.toml"
printf 'runtime-state\n' > "$SOURCE_RUNTIME/data/state.db"
printf 'checkpoint\n' > "$SOURCE_RUNTIME/.agent-runtime/checkpoint"
printf 'external\n' > "$SOURCE_RUNTIME/external_skills/local/INTERFACE.md"
printf 'optional\n' > "$SOURCE_RUNTIME/optional_skills/local/INTERFACE.md"
printf '#!/usr/bin/env bash\n' > "$SOURCE_RUNTIME/target/release/optional-skill"
chmod +x "$SOURCE_RUNTIME/target/release/optional-skill"

PACKAGE_MODE_OUTPUT="$(
  APP_RELEASES_JSON_FILE="$RELEASES_JSON" \
    "$DEPLOY_SCRIPT" \
      --root "$SOURCE_RUNTIME" \
      --platform ubuntu-x86_64 \
      --package-mode \
      --no-restart \
      --keep-backups 1
)"
grep -Fq 'release_package_status=enabled' <<< "$PACKAGE_MODE_OUTPUT"
grep -Fq 'release_update_status=deployed' <<< "$PACKAGE_MODE_OUTPUT"
test ! -e "$SOURCE_RUNTIME/.git"
test ! -e "$SOURCE_RUNTIME/Cargo.toml"
grep -Fxq 'ubuntu-x86_64-test' "$SOURCE_RUNTIME/.release-tag"
grep -Fxq 'local-secret = "preserve"' "$SOURCE_RUNTIME/configs/config.toml"
grep -Fxq 'runtime-state' "$SOURCE_RUNTIME/data/state.db"
grep -Fxq 'checkpoint' "$SOURCE_RUNTIME/.agent-runtime/checkpoint"
grep -Fxq 'external' "$SOURCE_RUNTIME/external_skills/local/INTERFACE.md"
grep -Fxq 'optional' "$SOURCE_RUNTIME/optional_skills/local/INTERFACE.md"
test -x "$SOURCE_RUNTIME/target/release/optional-skill"
cmp /bin/true "$SOURCE_RUNTIME/target/release/clawd"
SOURCE_BACKUP="$(find "$TMP_ROOT/.source-runtime-release-mode-backups" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
test -d "$SOURCE_BACKUP/.git"
grep -Fxq 'source-only' "$SOURCE_BACKUP/Cargo.toml"
test ! -e "$TMP_ROOT/.source-runtime-release-mode.lock"

ALREADY_PACKAGE_OUTPUT="$(
  "$DEPLOY_SCRIPT" \
    --root "$SOURCE_RUNTIME" \
    --platform ubuntu-x86_64 \
    --package-mode \
    --no-restart
)"
grep -Fq 'release_package_status=already_enabled' <<< "$ALREADY_PACKAGE_OUTPUT"

echo "DEPLOY_GITHUB_RELEASE_TESTS ok"
