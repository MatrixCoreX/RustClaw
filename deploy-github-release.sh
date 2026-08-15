#!/usr/bin/env bash
# Deploy the newest GitHub Release matching this host without rebuilding source.
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$SCRIPT_DIR"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/scripts/product_identity.sh"
REPOSITORY="$APP_RELEASE_REPOSITORY"
PLATFORM="auto"
REQUESTED_TAG=""
RESTART_MODE="auto"
CHECK_ONLY=0
FORCE=0
PACKAGE_MODE=0
KEEP_BACKUPS=2
SYSTEMD_UNIT="${APP_SERVICE_NAME}.service"

WORK_DIR=""
WORK_PARENT=""
WORK_PARENT_OWNED=0
LOCK_DIR=""
BACKUP_DIR=""
PACKAGE_STAGE_DIR=""
DEPLOY_STARTED=0
DEPLOY_COMMITTED=0
PACKAGE_MODE_ORIGINAL_MOVED=0
PACKAGE_MODE_COMMITTED=0
RESTART_ATTEMPTED=0
RUNTIME_WAS_ACTIVE=0
MANAGED_PATHS_FILE=""
EXISTING_PATHS_FILE=""
NEW_CONFIG_PATHS_FILE=""

usage() {
  cat <<'USAGE'
Usage: ./deploy-github-release.sh [options]

Download, verify, and deploy the newest compatible GitHub Release.

Options:
  --root DIR
      Agent runtime root. Default: directory containing this script.
  --repo OWNER/REPO
      GitHub repository. Default: product identity release_repository.
  --platform auto|ubuntu-x86_64|pi-aarch64
      Release platform. Default: auto-detect Linux OS and CPU architecture.
  --tag TAG
      Deploy one exact compatible release instead of the newest one.
  --no-restart
      Deploy files without restarting the runtime. Used by the UI update job.
  --restart
      Start or restart the runtime after deployment, even when it was stopped.
  --check-only
      Print the installed and newest compatible release without downloading.
  --force
      Reinstall even when the selected release is already installed.
  --package-mode
      Atomically replace a source checkout with the verified Release package.
      Persistent runtime state is preserved and the source tree is backed up.
  --keep-backups N
      Number of successful deployment backups to retain. Default: 2.
  -h, --help
      Show this help.

Local configuration, data, logs, PID state, external skills, and on-demand
skills are preserved. Packaged configuration files are copied only when the
corresponding local file does not already exist.
USAGE
}

die() {
  printf 'release_deploy_error=%s\n' "$*" >&2
  exit 1
}

require_value() {
  local option="$1"
  local value="${2:-}"
  [[ -n "$value" ]] || die "${option}_requires_value"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --root)
      require_value "$1" "${2:-}"
      ROOT_DIR="$2"
      shift 2
      ;;
    --root=*)
      ROOT_DIR="${1#*=}"
      shift
      ;;
    --repo)
      require_value "$1" "${2:-}"
      REPOSITORY="$2"
      shift 2
      ;;
    --repo=*)
      REPOSITORY="${1#*=}"
      shift
      ;;
    --platform)
      require_value "$1" "${2:-}"
      PLATFORM="$2"
      shift 2
      ;;
    --platform=*)
      PLATFORM="${1#*=}"
      shift
      ;;
    --tag)
      require_value "$1" "${2:-}"
      REQUESTED_TAG="$2"
      shift 2
      ;;
    --tag=*)
      REQUESTED_TAG="${1#*=}"
      shift
      ;;
    --no-restart)
      RESTART_MODE="none"
      shift
      ;;
    --restart)
      RESTART_MODE="always"
      shift
      ;;
    --check-only)
      CHECK_ONLY=1
      shift
      ;;
    --force)
      FORCE=1
      shift
      ;;
    --package-mode)
      PACKAGE_MODE=1
      FORCE=1
      shift
      ;;
    --keep-backups)
      require_value "$1" "${2:-}"
      KEEP_BACKUPS="$2"
      shift 2
      ;;
    --keep-backups=*)
      KEEP_BACKUPS="${1#*=}"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown_option:$1"
      ;;
  esac
done

case "$KEEP_BACKUPS" in
  ''|*[!0-9]*) die "invalid_keep_backups:$KEEP_BACKUPS" ;;
esac
[[ "$REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] ||
  die "invalid_repository:$REPOSITORY"
[[ -z "$REQUESTED_TAG" || "$REQUESTED_TAG" =~ ^[A-Za-z0-9._-]+$ ]] ||
  die "invalid_release_tag:$REQUESTED_TAG"

ROOT_DIR="$(python3 - "$ROOT_DIR" <<'PY'
from pathlib import Path
import sys

print(Path(sys.argv[1]).expanduser().resolve())
PY
)"
mkdir -p "$ROOT_DIR"
ROOT_PARENT="$(dirname "$ROOT_DIR")"
ROOT_NAME="$(basename "$ROOT_DIR")"
if [[ "$PACKAGE_MODE" -eq 1 && ! -e "$ROOT_DIR/.git" ]]; then
  printf 'release_package_status=already_enabled\n'
  exit 0
fi

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  if [[ "$os" != "Linux" ]]; then
    die "unsupported_release_os:$os"
  fi
  case "$arch" in
    x86_64|amd64) printf '%s\n' "ubuntu-x86_64" ;;
    aarch64|arm64) printf '%s\n' "pi-aarch64" ;;
    *) die "unsupported_release_arch:$arch" ;;
  esac
}

if [[ "$PLATFORM" == "auto" ]]; then
  PLATFORM="$(detect_platform)"
fi
case "$PLATFORM" in
  ubuntu-x86_64)
    RELEASE_PREFIX="ubuntu-x86_64-"
    ASSET_PREFIX="${APP_RELEASE_ARTIFACT_ID}-ubuntu-x86_64-"
    ELF_MACHINE=62
    ;;
  pi-aarch64)
    RELEASE_PREFIX="pi-aarch64-"
    ASSET_PREFIX="${APP_RELEASE_ARTIFACT_ID}-pi-aarch64-"
    ELF_MACHINE=183
    ;;
  *)
    die "unsupported_release_platform:$PLATFORM"
    ;;
esac
if [[ -n "$REQUESTED_TAG" && "$REQUESTED_TAG" != "$RELEASE_PREFIX"* ]]; then
  die "release_tag_platform_mismatch:$REQUESTED_TAG"
fi

hash_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
  else
    python3 - "$path" <<'PY'
import hashlib
from pathlib import Path
import sys

digest = hashlib.sha256()
with Path(sys.argv[1]).open("rb") as stream:
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        digest.update(chunk)
print(digest.hexdigest())
PY
  fi
}

download_file() {
  local url="$1"
  local output="$2"
  python3 - "$url" "$output" <<'PY'
import os
import sys
import urllib.request

url, output = sys.argv[1:]
headers = {"User-Agent": "Agent-System-release-deploy"}
token = os.environ.get("GITHUB_TOKEN", "").strip()
if token and url.startswith("https://api.github.com/"):
    headers["Authorization"] = f"Bearer {token}"
request = urllib.request.Request(url, headers=headers)
if url.startswith("https://api.github.com/") and "/releases/assets/" in url:
    request.add_header("Accept", "application/octet-stream")
with urllib.request.urlopen(request, timeout=300) as response, open(output, "wb") as stream:
    while True:
        chunk = response.read(1024 * 1024)
        if not chunk:
            break
        stream.write(chunk)
PY
}

runtime_pid_is_active() {
  local pid_file="$ROOT_DIR/.pids/clawd.pid"
  local pid=""
  [[ -f "$pid_file" ]] || return 1
  pid="$(cat "$pid_file" 2>/dev/null || true)"
  case "$pid" in
    ''|*[!0-9]*) return 1 ;;
  esac
  kill -0 "$pid" >/dev/null 2>&1
}

systemd_unit_exists() {
  [[ "$(uname -s)" == "Linux" ]] || return 1
  command -v systemctl >/dev/null 2>&1 || return 1
  systemctl cat "$SYSTEMD_UNIT" 2>/dev/null | grep -Fq -- "$ROOT_DIR/"
}

if systemd_unit_exists && systemctl is-active --quiet "$SYSTEMD_UNIT"; then
  RUNTIME_WAS_ACTIVE=1
elif runtime_pid_is_active; then
  RUNTIME_WAS_ACTIVE=1
fi

if [[ -n "${TMPDIR:-}" ]]; then
  WORK_PARENT="$TMPDIR"
else
  WORK_PARENT="$ROOT_PARENT/.${ROOT_NAME}-release-work"
  mkdir -p "$WORK_PARENT"
  chmod 700 "$WORK_PARENT"
  WORK_PARENT_OWNED=1
fi
WORK_DIR="$(mktemp -d "$WORK_PARENT/agent-release-deploy.XXXXXX")"
if [[ "$PACKAGE_MODE" -eq 1 ]]; then
  LOCK_DIR="$ROOT_PARENT/.${ROOT_NAME}-release-mode.lock"
else
  LOCK_DIR="$ROOT_DIR/.release-deploy.lock"
fi
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  lock_pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
  case "$lock_pid" in
    ''|*[!0-9]*) lock_pid="" ;;
  esac
  if [[ -n "$lock_pid" ]] && kill -0 "$lock_pid" >/dev/null 2>&1; then
    rm -rf "$WORK_DIR"
    WORK_DIR=""
    die "deployment_already_running:$LOCK_DIR"
  fi
  rm -rf "$LOCK_DIR"
  mkdir "$LOCK_DIR" || die "deployment_lock_failed:$LOCK_DIR"
fi
printf '%s\n' "$$" > "$LOCK_DIR/pid"

restart_runtime() {
  RESTART_ATTEMPTED=1
  if systemd_unit_exists; then
    systemctl restart "$SYSTEMD_UNIT"
    local attempt
    for attempt in $(seq 1 60); do
      if systemctl is-active --quiet "$SYSTEMD_UNIT"; then
        printf 'runtime_restart=systemd:%s\n' "$SYSTEMD_UNIT"
        return 0
      fi
      sleep 1
    done
    return 1
  fi

  if [[ "$RUNTIME_WAS_ACTIVE" -eq 1 || "$RESTART_MODE" == "always" ]]; then
    if [[ -x "$ROOT_DIR/stop-agent.sh" ]]; then
      "$ROOT_DIR/stop-agent.sh"
    fi
    (
      cd "$ROOT_DIR"
      "$ROOT_DIR/start-all-bin.sh" release
    )
    printf 'runtime_restart=direct\n'
  else
    printf 'runtime_restart=skipped_inactive\n'
  fi
}

rollback_deployment() {
  [[ "$DEPLOY_STARTED" -eq 1 && "$DEPLOY_COMMITTED" -eq 0 ]] || return 0
  printf 'release_deploy_rollback=%s\n' "${BACKUP_DIR:-unavailable}" >&2
  set +e
  if [[ -n "$MANAGED_PATHS_FILE" && -f "$MANAGED_PATHS_FILE" ]]; then
    while IFS= read -r relative; do
      [[ -n "$relative" ]] || continue
      rm -rf "$ROOT_DIR/$relative"
    done < "$MANAGED_PATHS_FILE"
  fi
  if [[ -n "$BACKUP_DIR" && -f "$BACKUP_DIR/files.tar.gz" ]]; then
    tar -xzf "$BACKUP_DIR/files.tar.gz" -C "$ROOT_DIR"
  fi
  if [[ -n "$NEW_CONFIG_PATHS_FILE" && -f "$NEW_CONFIG_PATHS_FILE" ]]; then
    while IFS= read -r relative; do
      [[ -n "$relative" ]] || continue
      rm -f "$ROOT_DIR/$relative"
    done < "$NEW_CONFIG_PATHS_FILE"
  fi
  if [[ "$RESTART_ATTEMPTED" -eq 1 ]]; then
    restart_runtime >/dev/null 2>&1 || true
  fi
  set -e
}

rollback_package_mode() {
  [[ "$PACKAGE_MODE_ORIGINAL_MOVED" -eq 1 && "$PACKAGE_MODE_COMMITTED" -eq 0 ]] || return 0
  printf 'release_package_rollback=%s\n' "${BACKUP_DIR:-unavailable}" >&2
  set +e
  rm -rf "$ROOT_DIR"
  if [[ -n "$BACKUP_DIR" && -d "$BACKUP_DIR" ]]; then
    mv "$BACKUP_DIR" "$ROOT_DIR"
  fi
  set -e
}

cleanup() {
  local status=$?
  if [[ "$status" -ne 0 ]]; then
    if [[ "$PACKAGE_MODE" -eq 1 ]]; then
      rollback_package_mode
    else
      rollback_deployment
    fi
  fi
  [[ -z "$WORK_DIR" ]] || rm -rf "$WORK_DIR"
  [[ -z "$PACKAGE_STAGE_DIR" ]] || rm -rf "$PACKAGE_STAGE_DIR"
  if [[ "$WORK_PARENT_OWNED" -eq 1 && -n "$WORK_PARENT" ]]; then
    rmdir "$WORK_PARENT" >/dev/null 2>&1 || true
  fi
  if [[ -n "$LOCK_DIR" && -d "$LOCK_DIR" ]]; then
    lock_pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
    if [[ "$lock_pid" == "$$" ]]; then
      rm -f "$LOCK_DIR/pid"
      rmdir "$LOCK_DIR" >/dev/null 2>&1 || true
    fi
  fi
  return "$status"
}
trap cleanup EXIT

printf 'release_repo=%s\n' "$REPOSITORY"
printf 'release_platform=%s\n' "$PLATFORM"

RELEASES_JSON="$WORK_DIR/releases.json"
RELEASES_JSON_OVERRIDE="${APP_RELEASES_JSON_FILE:-}"
if [[ -n "$RELEASES_JSON_OVERRIDE" ]]; then
  cp "$RELEASES_JSON_OVERRIDE" "$RELEASES_JSON"
else
  download_file \
    "https://api.github.com/repos/${REPOSITORY}/releases?per_page=50" \
    "$RELEASES_JSON"
fi

RELEASE_META="$WORK_DIR/release-meta.json"
python3 - \
  "$RELEASES_JSON" \
  "$RELEASE_PREFIX" \
  "$ASSET_PREFIX" \
  "$REQUESTED_TAG" \
  "$APP_RELEASE_ARTIFACT_ID" > "$RELEASE_META" <<'PY'
import json
from pathlib import Path
import sys

source, release_prefix, asset_prefix, requested_tag, app_id = sys.argv[1:]
releases = json.loads(Path(source).read_text(encoding="utf-8"))
if not isinstance(releases, list):
    raise SystemExit("release metadata is not a list")

for release in releases:
    if not isinstance(release, dict) or release.get("draft") or release.get("prerelease"):
        continue
    tag = str(release.get("tag_name") or "")
    if not tag.startswith(release_prefix):
        continue
    if requested_tag and tag != requested_tag:
        continue
    assets = release.get("assets") or []
    archives = []
    for asset in assets:
        if not isinstance(asset, dict):
            continue
        name = str(asset.get("name") or "")
        if "/" in name or "\\" in name:
            continue
        if not name or any(not (char.isalnum() or char in "._-") for char in name):
            continue
        if name.endswith(".tar.gz") and (
            name.startswith(asset_prefix)
            or name == f"{app_id}-{tag}.tar.gz"
        ):
            archives.append(asset)
    if len(archives) != 1:
        continue
    archive = archives[0]
    archive_name = str(archive.get("name") or "")
    checksum_name = f"{archive_name}.sha256"
    checksum = next(
        (
            asset
            for asset in assets
            if isinstance(asset, dict) and str(asset.get("name") or "") == checksum_name
        ),
        None,
    )
    if checksum is None:
        continue
    result = {
        "tag": tag,
        "archive_name": archive_name,
        "archive_url": str(
            archive.get("url") or archive.get("browser_download_url") or ""
        ),
        "checksum_name": checksum_name,
        "checksum_url": str(
            checksum.get("url") or checksum.get("browser_download_url") or ""
        ),
    }
    if not result["archive_url"] or not result["checksum_url"]:
        continue
    print(json.dumps(result))
    break
else:
    suffix = f":{requested_tag}" if requested_tag else ""
    raise SystemExit(f"compatible release with checksum not found{suffix}")
PY

eval "$(
  python3 - "$RELEASE_META" <<'PY'
import json
import shlex
from pathlib import Path
import sys

meta = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
for key, value in meta.items():
    print(f"{key.upper()}={shlex.quote(str(value))}")
PY
)"

printf 'release_tag=%s\n' "$TAG"
printf 'release_asset=%s\n' "$ARCHIVE_NAME"
INSTALLED_TAG=""
if [[ -f "$ROOT_DIR/.release-tag" ]]; then
  INSTALLED_TAG="$(head -n 1 "$ROOT_DIR/.release-tag" | tr -d '\r\n')"
fi
printf 'installed_release_tag=%s\n' "${INSTALLED_TAG:-none}"
if [[ "$CHECK_ONLY" -eq 1 ]]; then
  if [[ "$INSTALLED_TAG" == "$TAG" ]]; then
    printf 'release_update_status=current\n'
  else
    printf 'release_update_status=available\n'
  fi
  exit 0
fi
if [[ "$INSTALLED_TAG" == "$TAG" && "$FORCE" -eq 0 ]]; then
  printf 'release_update_status=already_current\n'
  exit 0
fi

ARCHIVE_PATH="$WORK_DIR/$ARCHIVE_NAME"
CHECKSUM_PATH="$WORK_DIR/$CHECKSUM_NAME"
download_file "$ARCHIVE_URL" "$ARCHIVE_PATH"
download_file "$CHECKSUM_URL" "$CHECKSUM_PATH"

EXPECTED_HASH="$(awk 'NR == 1 {print tolower($1)}' "$CHECKSUM_PATH")"
[[ "$EXPECTED_HASH" =~ ^[0-9a-f]{64}$ ]] || die "invalid_release_checksum_file"
ACTUAL_HASH="$(hash_file "$ARCHIVE_PATH")"
[[ "$ACTUAL_HASH" == "$EXPECTED_HASH" ]] || die "release_checksum_mismatch"
printf 'release_checksum=verified\n'

EXTRACT_DIR="$WORK_DIR/extract"
mkdir -p "$EXTRACT_DIR"
python3 - "$ARCHIVE_PATH" "$EXTRACT_DIR" "$APP_RELEASE_ARTIFACT_ID" <<'PY'
from pathlib import Path, PurePosixPath
import inspect
import sys
import tarfile

archive = Path(sys.argv[1])
destination = Path(sys.argv[2])
app_id = sys.argv[3]
with tarfile.open(archive, "r:gz") as package:
    members = package.getmembers()
    if not members:
        raise SystemExit("release archive is empty")
    for member in members:
        path = PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts:
            raise SystemExit(f"unsafe release path: {member.name}")
        if not path.parts or path.parts[0] != app_id:
            raise SystemExit(f"release path is outside an accepted package root: {member.name}")
        if member.issym() or member.islnk() or member.isdev():
            raise SystemExit(f"unsupported release entry: {member.name}")
    if "filter" in inspect.signature(package.extractall).parameters:
        package.extractall(destination, members=members, filter="fully_trusted")
    else:
        package.extractall(destination, members=members)
PY

if [[ -d "$EXTRACT_DIR/$APP_RELEASE_ARTIFACT_ID" ]]; then
  PACKAGE_DIR="$EXTRACT_DIR/$APP_RELEASE_ARTIFACT_ID"
else
  die "release_package_root_missing:$APP_RELEASE_ARTIFACT_ID"
fi
[[ -x "$PACKAGE_DIR/target/release/clawd" ]] ||
  die "release_package_missing_clawd"
python3 - "$PACKAGE_DIR/target/release/clawd" "$ELF_MACHINE" <<'PY'
from pathlib import Path
import sys

binary = Path(sys.argv[1]).read_bytes()[:20]
expected = int(sys.argv[2])
if len(binary) < 20 or binary[:4] != b"\x7fELF":
    raise SystemExit("clawd is not an ELF binary")
byteorder = "little" if binary[5] == 1 else "big"
machine = int.from_bytes(binary[18:20], byteorder=byteorder)
if machine != expected:
    raise SystemExit(f"release ELF architecture mismatch: expected={expected} actual={machine}")
PY

if [[ "$PACKAGE_MODE" -eq 1 ]]; then
  PACKAGE_STAGE_DIR="$(mktemp -d "$ROOT_PARENT/.${ROOT_NAME}-release-stage.XXXXXX")"
  STAGED_ROOT="$PACKAGE_STAGE_DIR/runtime"
  cp -a "$PACKAGE_DIR" "$STAGED_ROOT"

  merge_runtime_directory() {
    local relative="$1"
    local source="$ROOT_DIR/$relative"
    local target="$STAGED_ROOT/$relative"
    [[ -d "$source" ]] || return 0
    mkdir -p "$target"
    cp -a "$source/." "$target/"
  }

  printf 'release_package_step=preserving_runtime_state\n'
  for relative in \
    configs \
    data \
    logs \
    .pids \
    .agent-system \
    .agent-runtime \
    run \
    skills_output \
    external_skills \
    optional_skills \
    .release-backups \
    image/download; do
    merge_runtime_directory "$relative"
  done

  mkdir -p "$STAGED_ROOT/target/release"
  if [[ -d "$ROOT_DIR/target/release" ]]; then
    while IFS= read -r runtime_binary; do
      binary_name="$(basename "$runtime_binary")"
      if [[ ! -e "$STAGED_ROOT/target/release/$binary_name" ]]; then
        cp -a "$runtime_binary" "$STAGED_ROOT/target/release/$binary_name"
      fi
    done < <(find "$ROOT_DIR/target/release" -mindepth 1 -maxdepth 1 -type f | LC_ALL=C sort)
  fi

  for runtime_file in \
    "$ROOT_DIR"/.env \
    "$ROOT_DIR"/.env.local \
    "$ROOT_DIR"/.env.*.local \
    "$ROOT_DIR"/*.env \
    "$ROOT_DIR"/runtime_env*.sh \
    "$ROOT_DIR"/pi_app/.agent_small_screen_*; do
    [[ -f "$runtime_file" ]] || continue
    relative="${runtime_file#"$ROOT_DIR/"}"
    mkdir -p "$(dirname "$STAGED_ROOT/$relative")"
    cp -a "$runtime_file" "$STAGED_ROOT/$relative"
  done

  [[ -x "$STAGED_ROOT/target/release/clawd" ]] ||
    die "release_package_staged_clawd_missing"
  [[ -f "$STAGED_ROOT/configs/config.toml" ]] ||
    die "release_package_staged_config_missing"
  [[ ! -e "$STAGED_ROOT/.git" ]] ||
    die "release_package_staged_git_metadata_present"

  BACKUP_ROOT="$ROOT_PARENT/.${ROOT_NAME}-release-mode-backups"
  TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
  BACKUP_DIR="$BACKUP_ROOT/source-${TIMESTAMP}-$$"
  mkdir -p "$BACKUP_ROOT"
  chmod 700 "$BACKUP_ROOT"
  printf '%s\n' "$TAG" > "$STAGED_ROOT/.release-tag"
  printf '%s\n' "$BACKUP_DIR" > "$STAGED_ROOT/.release-rollback"

  printf 'release_package_step=activating\n'
  mv "$ROOT_DIR" "$BACKUP_DIR"
  PACKAGE_MODE_ORIGINAL_MOVED=1
  if ! mv "$STAGED_ROOT" "$ROOT_DIR"; then
    mv "$BACKUP_DIR" "$ROOT_DIR" || true
    PACKAGE_MODE_ORIGINAL_MOVED=0
    die "release_package_activation_failed"
  fi

  if [[ -x "$ROOT_DIR/build-ui-nginx.sh" && -f "$ROOT_DIR/UI/dist/index.html" ]]; then
    "$ROOT_DIR/build-ui-nginx.sh" --copy-if-configured
  fi

  if [[ "$RESTART_MODE" == "none" ]]; then
    printf 'runtime_restart=deferred\n'
  elif [[ "$RUNTIME_WAS_ACTIVE" -eq 1 || "$RESTART_MODE" == "always" ]]; then
    restart_runtime
  else
    printf 'runtime_restart=skipped_inactive\n'
  fi

  PACKAGE_MODE_COMMITTED=1
  printf 'deployed_release_tag=%s\n' "$TAG"
  printf 'release_package_backup=%s\n' "$BACKUP_DIR"

  python3 - "$BACKUP_ROOT" "$KEEP_BACKUPS" <<'PY'
from pathlib import Path
import shutil
import sys

root = Path(sys.argv[1])
keep = int(sys.argv[2])
backups = sorted(
    (path for path in root.iterdir() if path.is_dir() and path.name.startswith("source-")),
    key=lambda path: path.stat().st_mtime,
    reverse=True,
)
for path in backups[keep:]:
    shutil.rmtree(path)
PY

  printf 'release_package_status=enabled\n'
  printf 'release_update_status=deployed\n'
  exit 0
fi

TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
BACKUP_DIR="$ROOT_DIR/.release-backups/${INSTALLED_TAG:-unversioned}-${TIMESTAMP}"
mkdir -p "$BACKUP_DIR"
MANAGED_PATHS_FILE="$WORK_DIR/managed-paths.txt"
EXISTING_PATHS_FILE="$WORK_DIR/existing-paths.txt"
NEW_CONFIG_PATHS_FILE="$WORK_DIR/new-config-paths.txt"
: > "$MANAGED_PATHS_FILE"
: > "$EXISTING_PATHS_FILE"
: > "$NEW_CONFIG_PATHS_FILE"

for relative in \
  prompts \
  migrations \
  scripts \
  component_start \
  pi_app \
  UI/dist; do
  [[ -e "$PACKAGE_DIR/$relative" ]] || continue
  printf '%s\n' "$relative" >> "$MANAGED_PATHS_FILE"
done

if [[ -d "$PACKAGE_DIR/services/wa-web-bridge" ]]; then
  while IFS= read -r bridge_file; do
    [[ -f "$bridge_file" ]] || continue
    printf '%s\n' "${bridge_file#"$PACKAGE_DIR/"}" >> "$MANAGED_PATHS_FILE"
  done < <(
    find "$PACKAGE_DIR/services/wa-web-bridge" \
      -mindepth 1 -maxdepth 1 -type f | LC_ALL=C sort
  )
fi

for relative in \
  README.md \
  README.zh-CN.md \
  USAGE.md \
  VERSION \
  agentctl \
  install-agent-cmd.sh \
  stop-agent.sh \
  build-ui-nginx.sh \
  start-all.sh \
  start-all-bin.sh \
  deploy-github-release.sh; do
  [[ -e "$PACKAGE_DIR/$relative" ]] || continue
  printf '%s\n' "$relative" >> "$MANAGED_PATHS_FILE"
done

while IFS= read -r binary; do
  [[ -f "$binary" ]] || continue
  printf 'target/release/%s\n' "$(basename "$binary")" >> "$MANAGED_PATHS_FILE"
done < <(find "$PACKAGE_DIR/target/release" -mindepth 1 -maxdepth 1 -type f | LC_ALL=C sort)

for manifest_root in crates/skills optional_skills external_skills; do
  [[ -d "$PACKAGE_DIR/$manifest_root" ]] || continue
  while IFS= read -r manifest; do
    [[ -f "$manifest" ]] || continue
    printf '%s\n' "${manifest#"$PACKAGE_DIR/"}" >> "$MANAGED_PATHS_FILE"
  done < <(
    find "$PACKAGE_DIR/$manifest_root" \
      -mindepth 2 -maxdepth 2 -type f -name skill.toml | LC_ALL=C sort
  )
done

# The packaged data tree contains repository-maintained proactive receipts only.
# Install those directories individually so optional/external runtime receipts
# that exist only on this host remain untouched.
if [[ -d "$PACKAGE_DIR/data/skill-packages" ]]; then
  while IFS= read -r receipt_dir; do
    [[ -d "$receipt_dir" ]] || continue
    printf 'data/skill-packages/%s\n' "$(basename "$receipt_dir")" >> "$MANAGED_PATHS_FILE"
  done < <(
    find "$PACKAGE_DIR/data/skill-packages" \
      -mindepth 1 -maxdepth 1 -type d | LC_ALL=C sort
  )
fi
if [[ -d "$PACKAGE_DIR/prebuilt/skill-packages" ]]; then
  printf '%s\n' "prebuilt/skill-packages" >> "$MANAGED_PATHS_FILE"
fi
printf '%s\n' ".release-tag" >> "$MANAGED_PATHS_FILE"
printf '%s\n' ".release-rollback" >> "$MANAGED_PATHS_FILE"

while IFS= read -r relative; do
  [[ -n "$relative" ]] || continue
  if [[ -e "$ROOT_DIR/$relative" ]]; then
    printf '%s\n' "$relative" >> "$EXISTING_PATHS_FILE"
  fi
done < "$MANAGED_PATHS_FILE"
if [[ -s "$EXISTING_PATHS_FILE" ]]; then
  tar -czf "$BACKUP_DIR/files.tar.gz" -C "$ROOT_DIR" -T "$EXISTING_PATHS_FILE"
else
  tar -czf "$BACKUP_DIR/files.tar.gz" --files-from /dev/null
fi
cp "$MANAGED_PATHS_FILE" "$BACKUP_DIR/managed-paths.txt"
cp "$EXISTING_PATHS_FILE" "$BACKUP_DIR/existing-paths.txt"

if [[ -d "$PACKAGE_DIR/configs" ]]; then
  while IFS= read -r source; do
    relative="${source#"$PACKAGE_DIR/"}"
    if [[ ! -e "$ROOT_DIR/$relative" ]]; then
      printf '%s\n' "$relative" >> "$NEW_CONFIG_PATHS_FILE"
    fi
  done < <(find "$PACKAGE_DIR/configs" -type f | LC_ALL=C sort)
fi
cp "$NEW_CONFIG_PATHS_FILE" "$BACKUP_DIR/new-config-paths.txt"

install_managed_path() {
  local relative="$1"
  local source="$PACKAGE_DIR/$relative"
  local target="$ROOT_DIR/$relative"
  local parent temp
  parent="$(dirname "$target")"
  temp="$parent/.$(basename "$target").agent-new.$$"
  mkdir -p "$parent"
  rm -rf "$temp"
  cp -a "$source" "$temp"
  rm -rf "$target"
  mv "$temp" "$target"
}

DEPLOY_STARTED=1
while IFS= read -r relative; do
  [[ -n "$relative" ]] || continue
  case "$relative" in
    .release-tag|.release-rollback) continue ;;
  esac
  install_managed_path "$relative"
done < "$MANAGED_PATHS_FILE"

if [[ -d "$PACKAGE_DIR/configs" ]]; then
  while IFS= read -r relative; do
    [[ -n "$relative" ]] || continue
    mkdir -p "$(dirname "$ROOT_DIR/$relative")"
    cp -a "$PACKAGE_DIR/$relative" "$ROOT_DIR/$relative"
  done < "$NEW_CONFIG_PATHS_FILE"
fi
printf '%s\n' "$TAG" > "$ROOT_DIR/.release-tag"
printf '%s\n' "$BACKUP_DIR" > "$ROOT_DIR/.release-rollback"

if [[ -x "$ROOT_DIR/build-ui-nginx.sh" && -f "$ROOT_DIR/UI/dist/index.html" ]]; then
  "$ROOT_DIR/build-ui-nginx.sh" --copy-if-configured
fi

if [[ "$RESTART_MODE" == "none" ]]; then
  printf 'runtime_restart=deferred\n'
elif [[ "$RUNTIME_WAS_ACTIVE" -eq 1 || "$RESTART_MODE" == "always" ]]; then
  restart_runtime
else
  printf 'runtime_restart=skipped_inactive\n'
fi

DEPLOY_COMMITTED=1
printf 'deployed_release_tag=%s\n' "$TAG"
printf 'release_backup=%s\n' "$BACKUP_DIR"

python3 - "$ROOT_DIR/.release-backups" "$KEEP_BACKUPS" <<'PY'
from pathlib import Path
import shutil
import sys

root = Path(sys.argv[1])
keep = int(sys.argv[2])
if root.is_dir():
    backups = sorted(
        (path for path in root.iterdir() if path.is_dir()),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    for path in backups[keep:]:
        shutil.rmtree(path)
PY

printf 'release_update_status=deployed\n'
