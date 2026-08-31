#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY_SCRIPT="$ROOT_DIR/deploy-github-release.sh"
# shellcheck source=/dev/null
source "$ROOT_DIR/scripts/product_identity.sh"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
SIGNING_KEY="$TMP_ROOT/release-signing-key"
ALLOWED_SIGNERS="$TMP_ROOT/release-allowed-signers"
ssh-keygen -q -t ed25519 -N '' -f "$SIGNING_KEY"
printf 'release %s %s\n' "$(awk '{print $1}' "$SIGNING_KEY.pub")" "$(awk '{print $2}' "$SIGNING_KEY.pub")" > "$ALLOWED_SIGNERS"
export APP_RELEASE_ALLOWED_SIGNERS_FILE="$ALLOWED_SIGNERS"
export APP_RELEASE_MANIFEST_TOOL="$ROOT_DIR/scripts/security/release_manifest.py"

create_release_evidence() {
  local archive="$1"
  local version="$2"
  local target="${3:-x86_64-unknown-linux-gnu}"
  printf '%s\n' '{"SPDXID":"SPDXRef-DOCUMENT","dataLicense":"CC0-1.0","name":"test","spdxVersion":"SPDX-2.3"}' > "$archive.spdx.json"
  python3 "$ROOT_DIR/scripts/security/release_manifest.py" create \
    --artifact "$archive" \
    --sbom "$archive.spdx.json" \
    --output "$archive.manifest.json" \
    --version "$version" \
    --commit 0123456789abcdef0123456789abcdef01234567 \
    --target "$target" \
    --package-root "$APP_RELEASE_ARTIFACT_ID"
  ssh-keygen -q -Y sign -f "$SIGNING_KEY" -n agent-runtime-release "$archive.manifest.json"
}

write_release_metadata() {
  local output="$1"
  local tag="$2"
  local archive="$3"
  local checksum="$4"
  local manifest="$5"
  local signature="$6"
  local sbom="$7"
  python3 - "$output" "$tag" "$archive" "$checksum" "$manifest" "$signature" "$sbom" <<'PY'
import json
from pathlib import Path
import sys

output, tag, archive, checksum, manifest, signature, sbom = sys.argv[1:]
paths = [Path(value).resolve() for value in (archive, checksum, manifest, signature, sbom)]
archive_path = paths[0]
names = [
    archive_path.name,
    f"{archive_path.name}.sha256",
    f"{archive_path.name}.manifest.json",
    f"{archive_path.name}.manifest.json.sig",
    f"{archive_path.name}.spdx.json",
]
assets = [
    {"name": name, "browser_download_url": path.as_uri()}
    for name, path in zip(names, paths, strict=True)
]
Path(output).write_text(
    json.dumps([{"tag_name": tag, "draft": False, "prerelease": False, "assets": assets}]),
    encoding="utf-8",
)
PY
}

PACKAGE_DIR="$TMP_ROOT/package/$APP_RELEASE_ARTIFACT_ID"
mkdir -p \
  "$PACKAGE_DIR/target/release" \
  "$PACKAGE_DIR/configs/channels" \
  "$PACKAGE_DIR/crates/skills/core_fixture" \
  "$PACKAGE_DIR/optional_skills/store_fixture" \
  "$PACKAGE_DIR/data/skill-packages/core_fixture" \
  "$PACKAGE_DIR/prebuilt/skill-packages/store_fixture" \
  "$PACKAGE_DIR/services/wa-web-bridge" \
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
printf 'release bridge\n' > "$PACKAGE_DIR/services/wa-web-bridge/index.js"
printf '<!doctype html><title>release ui</title>\n' > "$PACKAGE_DIR/UI/dist/index.html"

ARCHIVE="$TMP_ROOT/$APP_RELEASE_ARTIFACT_ID-ubuntu-x86_64-test.tar.gz"
tar -czf "$ARCHIVE" -C "$TMP_ROOT/package" "$APP_RELEASE_ARTIFACT_ID"
ARCHIVE_HASH="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
CHECKSUM="$ARCHIVE.sha256"
printf '%s  %s\n' "$ARCHIVE_HASH" "$(basename "$ARCHIVE")" > "$CHECKSUM"
create_release_evidence "$ARCHIVE" 9.8.7

RELEASES_JSON="$TMP_ROOT/releases.json"
python3 - "$RELEASES_JSON" "$ARCHIVE" "$CHECKSUM" "$ARCHIVE.manifest.json" "$ARCHIVE.manifest.json.sig" "$ARCHIVE.spdx.json" <<'PY'
import json
from pathlib import Path
import sys

output, archive, checksum, manifest, signature, sbom = sys.argv[1:]
archive = Path(archive).resolve()
checksum = Path(checksum).resolve()
manifest = Path(manifest).resolve()
signature = Path(signature).resolve()
sbom = Path(sbom).resolve()
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
            {"name": manifest.name, "url": manifest.as_uri()},
            {"name": signature.name, "url": signature.as_uri()},
            {"name": sbom.name, "url": sbom.as_uri()},
        ],
    },
]
Path(output).write_text(json.dumps(releases), encoding="utf-8")
PY

RUNTIME="$TMP_ROOT/runtime"
mkdir -p \
  "$RUNTIME/configs" \
  "$RUNTIME/data/skill-packages/local_optional" \
  "$RUNTIME/services/wa-web-bridge" \
  "$RUNTIME/target/release"
cp /bin/false "$RUNTIME/target/release/clawd"
printf 'local-secret = "preserve"\n' > "$RUNTIME/configs/config.toml"
printf 'old readme\n' > "$RUNTIME/README.md"
printf 'keep local optional\n' > "$RUNTIME/data/skill-packages/local_optional/current.json"
printf 'old bridge\n' > "$RUNTIME/services/wa-web-bridge/index.js"
printf 'keep source test\n' > "$RUNTIME/services/wa-web-bridge/test.js"
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

assert_rejected_release() {
  local label="$1"
  local metadata="$2"
  local tag="$3"
  local before_hash after_hash
  before_hash="$(sha256sum "$RUNTIME/target/release/clawd" | awk '{print $1}')"
  if APP_RELEASES_JSON_FILE="$metadata" \
    "$DEPLOY_SCRIPT" \
      --root "$RUNTIME" \
      --platform ubuntu-x86_64 \
      --tag "$tag" \
      --no-restart >/dev/null 2>&1; then
    echo "$label unexpectedly succeeded" >&2
    exit 1
  fi
  after_hash="$(sha256sum "$RUNTIME/target/release/clawd" | awk '{print $1}')"
  [[ "$before_hash" == "$after_hash" ]]
  grep -Fxq 'ubuntu-x86_64-test' "$RUNTIME/.release-tag"
}
grep -Fxq 'local-secret = "preserve"' "$RUNTIME/configs/config.toml"
grep -Fxq 'new-default = true' "$RUNTIME/configs/new-default.toml"
grep -Fxq 'new release readme' "$RUNTIME/README.md"
grep -Fxq '9.8.7' "$RUNTIME/VERSION"
grep -Fxq 'name = "core_fixture"' "$RUNTIME/crates/skills/core_fixture/skill.toml"
grep -Fxq 'name = "store_fixture"' "$RUNTIME/optional_skills/store_fixture/skill.toml"
grep -Fxq 'release receipt' "$RUNTIME/data/skill-packages/core_fixture/current.json"
grep -Fxq 'keep local optional' "$RUNTIME/data/skill-packages/local_optional/current.json"
grep -Fxq 'release prebuilt' "$RUNTIME/prebuilt/skill-packages/store_fixture/current.json"
grep -Fxq 'release bridge' "$RUNTIME/services/wa-web-bridge/index.js"
grep -Fxq 'keep source test' "$RUNTIME/services/wa-web-bridge/test.js"
cmp /bin/true "$RUNTIME/target/release/clawd"
find "$RUNTIME/.release-backups" -name files.tar.gz -type f | grep -q .
ROLLBACK_MARKER_BEFORE="$(cat "$RUNTIME/.release-rollback")"

BAD_CHECKSUM="$TMP_ROOT/bad.sha256"
printf '%064d  %s\n' 0 "$(basename "$ARCHIVE")" > "$BAD_CHECKSUM"
BAD_RELEASES_JSON="$TMP_ROOT/bad-releases.json"
python3 - "$BAD_RELEASES_JSON" "$ARCHIVE" "$BAD_CHECKSUM" "$ARCHIVE.manifest.json" "$ARCHIVE.manifest.json.sig" "$ARCHIVE.spdx.json" <<'PY'
import json
from pathlib import Path
import sys

output, archive, checksum, manifest, signature, sbom = sys.argv[1:]
archive = Path(archive).resolve()
checksum = Path(checksum).resolve()
manifest = Path(manifest).resolve()
signature = Path(signature).resolve()
sbom = Path(sbom).resolve()
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
                    {"name": manifest.name, "browser_download_url": manifest.as_uri()},
                    {"name": signature.name, "browser_download_url": signature.as_uri()},
                    {"name": sbom.name, "browser_download_url": sbom.as_uri()},
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

TAMPERED_SIGNATURE="$TMP_ROOT/tampered-signature.sig"
cp "$ARCHIVE.manifest.json.sig" "$TAMPERED_SIGNATURE"
python3 - "$TAMPERED_SIGNATURE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text(encoding="ascii").splitlines()
lines[1] = ("A" if lines[1][0] != "A" else "B") + lines[1][1:]
path.write_text("\n".join(lines) + "\n", encoding="ascii")
PY
TAMPERED_SIGNATURE_JSON="$TMP_ROOT/tampered-signature.json"
write_release_metadata "$TAMPERED_SIGNATURE_JSON" ubuntu-x86_64-tampered-signature \
  "$ARCHIVE" "$CHECKSUM" "$ARCHIVE.manifest.json" "$TAMPERED_SIGNATURE" "$ARCHIVE.spdx.json"
assert_rejected_release signature_tamper "$TAMPERED_SIGNATURE_JSON" ubuntu-x86_64-tampered-signature

TAMPERED_ARCHIVE="$TMP_ROOT/$APP_RELEASE_ARTIFACT_ID-ubuntu-x86_64-tampered-package.tar.gz"
cp "$ARCHIVE" "$TAMPERED_ARCHIVE"
create_release_evidence "$TAMPERED_ARCHIVE" 9.8.8
printf 'tamper' >> "$TAMPERED_ARCHIVE"
TAMPERED_CHECKSUM="$TAMPERED_ARCHIVE.sha256"
printf '%s  %s\n' "$(sha256sum "$TAMPERED_ARCHIVE" | awk '{print $1}')" "$(basename "$TAMPERED_ARCHIVE")" > "$TAMPERED_CHECKSUM"
TAMPERED_ARCHIVE_JSON="$TMP_ROOT/tampered-package.json"
write_release_metadata "$TAMPERED_ARCHIVE_JSON" ubuntu-x86_64-tampered-package \
  "$TAMPERED_ARCHIVE" "$TAMPERED_CHECKSUM" "$TAMPERED_ARCHIVE.manifest.json" \
  "$TAMPERED_ARCHIVE.manifest.json.sig" "$TAMPERED_ARCHIVE.spdx.json"
assert_rejected_release package_tamper "$TAMPERED_ARCHIVE_JSON" ubuntu-x86_64-tampered-package

TAMPERED_SBOM="$TMP_ROOT/tampered.spdx.json"
cp "$ARCHIVE.spdx.json" "$TAMPERED_SBOM"
printf 'x' >> "$TAMPERED_SBOM"
TAMPERED_SBOM_JSON="$TMP_ROOT/tampered-sbom.json"
write_release_metadata "$TAMPERED_SBOM_JSON" ubuntu-x86_64-tampered-sbom \
  "$ARCHIVE" "$CHECKSUM" "$ARCHIVE.manifest.json" "$ARCHIVE.manifest.json.sig" "$TAMPERED_SBOM"
assert_rejected_release sbom_tamper "$TAMPERED_SBOM_JSON" ubuntu-x86_64-tampered-sbom

WRONG_TARGET_ARCHIVE="$TMP_ROOT/$APP_RELEASE_ARTIFACT_ID-ubuntu-x86_64-wrong-target.tar.gz"
cp "$ARCHIVE" "$WRONG_TARGET_ARCHIVE"
printf '%s  %s\n' "$(sha256sum "$WRONG_TARGET_ARCHIVE" | awk '{print $1}')" "$(basename "$WRONG_TARGET_ARCHIVE")" > "$WRONG_TARGET_ARCHIVE.sha256"
create_release_evidence "$WRONG_TARGET_ARCHIVE" 9.8.8 aarch64-unknown-linux-gnu
WRONG_TARGET_JSON="$TMP_ROOT/wrong-target.json"
write_release_metadata "$WRONG_TARGET_JSON" ubuntu-x86_64-wrong-target \
  "$WRONG_TARGET_ARCHIVE" "$WRONG_TARGET_ARCHIVE.sha256" "$WRONG_TARGET_ARCHIVE.manifest.json" \
  "$WRONG_TARGET_ARCHIVE.manifest.json.sig" "$WRONG_TARGET_ARCHIVE.spdx.json"
assert_rejected_release target_mismatch "$WRONG_TARGET_JSON" ubuntu-x86_64-wrong-target

DOWNGRADE_STAGE="$TMP_ROOT/downgrade-stage"
mkdir -p "$DOWNGRADE_STAGE"
cp -a "$PACKAGE_DIR" "$DOWNGRADE_STAGE/$APP_RELEASE_ARTIFACT_ID"
printf '1.0.0\n' > "$DOWNGRADE_STAGE/$APP_RELEASE_ARTIFACT_ID/VERSION"
DOWNGRADE_ARCHIVE="$TMP_ROOT/$APP_RELEASE_ARTIFACT_ID-ubuntu-x86_64-downgrade.tar.gz"
tar -czf "$DOWNGRADE_ARCHIVE" -C "$DOWNGRADE_STAGE" "$APP_RELEASE_ARTIFACT_ID"
printf '%s  %s\n' "$(sha256sum "$DOWNGRADE_ARCHIVE" | awk '{print $1}')" "$(basename "$DOWNGRADE_ARCHIVE")" > "$DOWNGRADE_ARCHIVE.sha256"
create_release_evidence "$DOWNGRADE_ARCHIVE" 1.0.0
DOWNGRADE_JSON="$TMP_ROOT/downgrade.json"
write_release_metadata "$DOWNGRADE_JSON" ubuntu-x86_64-downgrade \
  "$DOWNGRADE_ARCHIVE" "$DOWNGRADE_ARCHIVE.sha256" "$DOWNGRADE_ARCHIVE.manifest.json" \
  "$DOWNGRADE_ARCHIVE.manifest.json.sig" "$DOWNGRADE_ARCHIVE.spdx.json"
assert_rejected_release version_downgrade "$DOWNGRADE_JSON" ubuntu-x86_64-downgrade

make_unsafe_archive() {
  local kind="$1"
  local output="$2"
  python3 - "$PACKAGE_DIR" "$APP_RELEASE_ARTIFACT_ID" "$kind" "$output" <<'PY'
import io
from pathlib import Path
import sys
import tarfile

source, root, kind, output = sys.argv[1:]
with tarfile.open(output, "w:gz") as archive:
    archive.add(source, arcname=root)
    info = tarfile.TarInfo(f"{root}/unsafe-entry")
    if kind == "traversal":
        info.name = f"{root}/../../escape"
        payload = b"escape"
        info.size = len(payload)
        archive.addfile(info, io.BytesIO(payload))
    elif kind == "symlink":
        info.type = tarfile.SYMTYPE
        info.linkname = "/etc/passwd"
        archive.addfile(info)
    elif kind == "hardlink":
        info.type = tarfile.LNKTYPE
        info.linkname = f"{root}/VERSION"
        archive.addfile(info)
    elif kind == "device":
        info.type = tarfile.CHRTYPE
        info.devmajor = 1
        info.devminor = 3
        archive.addfile(info)
    else:
        raise SystemExit("unsupported fixture")
PY
}

for unsafe_kind in traversal symlink hardlink device; do
  unsafe_archive="$TMP_ROOT/$APP_RELEASE_ARTIFACT_ID-ubuntu-x86_64-${unsafe_kind}.tar.gz"
  make_unsafe_archive "$unsafe_kind" "$unsafe_archive"
  printf '%s  %s\n' "$(sha256sum "$unsafe_archive" | awk '{print $1}')" "$(basename "$unsafe_archive")" > "$unsafe_archive.sha256"
  create_release_evidence "$unsafe_archive" 9.8.8
  unsafe_json="$TMP_ROOT/${unsafe_kind}.json"
  write_release_metadata "$unsafe_json" "ubuntu-x86_64-${unsafe_kind}" \
    "$unsafe_archive" "$unsafe_archive.sha256" "$unsafe_archive.manifest.json" \
    "$unsafe_archive.manifest.json.sig" "$unsafe_archive.spdx.json"
  assert_rejected_release "archive_${unsafe_kind}" "$unsafe_json" "ubuntu-x86_64-${unsafe_kind}"
done

WRONG_ARCH_STAGE="$TMP_ROOT/wrong-arch-stage"
mkdir -p "$WRONG_ARCH_STAGE"
cp -a "$PACKAGE_DIR" "$WRONG_ARCH_STAGE/$APP_RELEASE_ARTIFACT_ID"
python3 - "$WRONG_ARCH_STAGE/$APP_RELEASE_ARTIFACT_ID/target/release/clawd" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
data = bytearray(path.read_bytes())
byteorder = "little" if data[5] == 1 else "big"
data[18:20] = (183).to_bytes(2, byteorder=byteorder)
path.write_bytes(data)
PY
WRONG_ARCHIVE="$TMP_ROOT/$APP_RELEASE_ARTIFACT_ID-ubuntu-x86_64-wrong-arch.tar.gz"
tar -czf "$WRONG_ARCHIVE" -C "$WRONG_ARCH_STAGE" "$APP_RELEASE_ARTIFACT_ID"
printf '%s  %s\n' "$(sha256sum "$WRONG_ARCHIVE" | awk '{print $1}')" "$(basename "$WRONG_ARCHIVE")" > "$WRONG_ARCHIVE.sha256"
create_release_evidence "$WRONG_ARCHIVE" 9.8.8
WRONG_ARCH_JSON="$TMP_ROOT/wrong-arch.json"
write_release_metadata "$WRONG_ARCH_JSON" ubuntu-x86_64-wrong-arch \
  "$WRONG_ARCHIVE" "$WRONG_ARCHIVE.sha256" "$WRONG_ARCHIVE.manifest.json" \
  "$WRONG_ARCHIVE.manifest.json.sig" "$WRONG_ARCHIVE.spdx.json"
assert_rejected_release architecture_mismatch "$WRONG_ARCH_JSON" ubuntu-x86_64-wrong-arch

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
create_release_evidence "$FAIL_ARCHIVE" 9.8.7
FAIL_RELEASES_JSON="$TMP_ROOT/fail-releases.json"
python3 - "$FAIL_RELEASES_JSON" "$FAIL_ARCHIVE" "$FAIL_CHECKSUM" "$FAIL_ARCHIVE.manifest.json" "$FAIL_ARCHIVE.manifest.json.sig" "$FAIL_ARCHIVE.spdx.json" <<'PY'
import json
from pathlib import Path
import sys

output, archive, checksum, manifest, signature, sbom = sys.argv[1:]
archive = Path(archive).resolve()
checksum = Path(checksum).resolve()
manifest = Path(manifest).resolve()
signature = Path(signature).resolve()
sbom = Path(sbom).resolve()
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
                    {"name": manifest.name, "browser_download_url": manifest.as_uri()},
                    {"name": signature.name, "browser_download_url": signature.as_uri()},
                    {"name": sbom.name, "browser_download_url": sbom.as_uri()},
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
