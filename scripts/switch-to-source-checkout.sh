#!/usr/bin/env bash
# Convert a packaged agent runtime into a complete Git source checkout.
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT_DIR/scripts/product_identity.sh"
REPOSITORY="https://github.com/${APP_RELEASE_REPOSITORY}.git"
BRANCH="${APP_SOURCE_BRANCH:-main}"
KEEP_BACKUPS=1

STAGE_DIR=""
LOCK_DIR=""
BACKUP_DIR=""
ORIGINAL_MOVED=0
MIGRATION_COMMITTED=0

usage() {
  cat <<'USAGE'
Usage: scripts/switch-to-source-checkout.sh [options]

Clone and verify the configured source repository, preserve local runtime state,
then atomically replace a packaged installation with the source checkout.

Options:
  --root DIR
      Agent runtime root. Default: repository root containing this script.
  --repo URL
      Git repository URL or local test repository.
  --branch NAME
      Source branch. Default: main.
  --keep-backups N
      Successful packaged-installation backups to retain. Default: 1.
  -h, --help
      Show this help.
USAGE
}

die() {
  printf 'source_checkout_error=%s\n' "$*" >&2
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
    --branch)
      require_value "$1" "${2:-}"
      BRANCH="$2"
      shift 2
      ;;
    --branch=*)
      BRANCH="${1#*=}"
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
[[ "$BRANCH" =~ ^[A-Za-z0-9._/-]+$ ]] || die "invalid_branch:$BRANCH"
command -v git >/dev/null 2>&1 || die "git_unavailable"
command -v python3 >/dev/null 2>&1 || die "python3_unavailable"

ROOT_DIR="$(python3 - "$ROOT_DIR" <<'PY'
from pathlib import Path
import sys

print(Path(sys.argv[1]).expanduser().resolve())
PY
)"
[[ "$ROOT_DIR" != "/" ]] || die "unsafe_root"
[[ -d "$ROOT_DIR" ]] || die "runtime_root_missing:$ROOT_DIR"
if [[ -e "$ROOT_DIR/.git" ]]; then
  printf 'source_checkout_status=already_enabled\n'
  exit 0
fi

ROOT_PARENT="$(dirname "$ROOT_DIR")"
ROOT_NAME="$(basename "$ROOT_DIR")"
[[ -w "$ROOT_PARENT" ]] || die "runtime_parent_not_writable:$ROOT_PARENT"

LOCK_DIR="$ROOT_PARENT/.${ROOT_NAME}-source-migration.lock"
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  lock_pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
  case "$lock_pid" in
    ''|*[!0-9]*) lock_pid="" ;;
  esac
  if [[ -n "$lock_pid" ]] && kill -0 "$lock_pid" >/dev/null 2>&1; then
    die "source_migration_already_running"
  fi
  rm -rf "$LOCK_DIR"
  mkdir "$LOCK_DIR" || die "source_migration_lock_failed"
fi
printf '%s\n' "$$" > "$LOCK_DIR/pid"

cleanup() {
  local status=$?
  if [[ "$MIGRATION_COMMITTED" -eq 0 && -d "$ROOT_DIR/.git" ]]; then
    MIGRATION_COMMITTED=1
  fi
  if [[ "$MIGRATION_COMMITTED" -eq 0 && "$ORIGINAL_MOVED" -eq 1 ]]; then
    if [[ ! -e "$ROOT_DIR" && -d "$BACKUP_DIR" ]]; then
      mv "$BACKUP_DIR" "$ROOT_DIR" || true
    fi
  fi
  if [[ -n "$STAGE_DIR" && -d "$STAGE_DIR" ]]; then
    rm -rf "$STAGE_DIR"
  fi
  if [[ -n "$LOCK_DIR" && -d "$LOCK_DIR" ]]; then
    lock_pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
    if [[ "$lock_pid" == "$$" ]]; then
      rm -f "$LOCK_DIR/pid"
      rmdir "$LOCK_DIR" >/dev/null 2>&1 || true
    fi
  fi
  if [[ "$status" -ne 0 && "$MIGRATION_COMMITTED" -eq 0 ]]; then
    printf 'source_checkout_status=unchanged\n' >&2
  fi
  return "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

STAGE_DIR="$(mktemp -d "$ROOT_PARENT/.${ROOT_NAME}-source-stage.XXXXXX")"
CHECKOUT_DIR="$STAGE_DIR/checkout"
printf 'source_checkout_step=cloning\n'
git clone --quiet --single-branch --branch "$BRANCH" -- "$REPOSITORY" "$CHECKOUT_DIR"

for required in \
  Cargo.toml \
  UI/package.json \
  scripts/switch-to-source-checkout.sh \
  start-all-bin.sh; do
  [[ -e "$CHECKOUT_DIR/$required" ]] || die "source_checkout_missing:$required"
done
git -C "$CHECKOUT_DIR" rev-parse --verify HEAD >/dev/null ||
  die "source_checkout_invalid_git_head"
SOURCE_COMMIT="$(git -C "$CHECKOUT_DIR" rev-parse --short HEAD)"
printf 'source_checkout_commit=%s\n' "$SOURCE_COMMIT"

merge_runtime_directory() {
  local relative="$1"
  local source="$ROOT_DIR/$relative"
  local target="$CHECKOUT_DIR/$relative"
  [[ -d "$source" ]] || return 0
  mkdir -p "$target"
  cp -a "$source/." "$target/"
}

printf 'source_checkout_step=preserving_runtime_state\n'
for relative in \
  configs \
  data \
  logs \
  .pids \
  external_skills \
  .release-backups \
  target/release \
  UI/dist; do
  merge_runtime_directory "$relative"
done

for relative in .env .env.local; do
  if [[ -f "$ROOT_DIR/$relative" ]]; then
    cp -a "$ROOT_DIR/$relative" "$CHECKOUT_DIR/$relative"
  fi
done
rm -f "$CHECKOUT_DIR/.release-tag" "$CHECKOUT_DIR/.release-rollback"

[[ -x "$CHECKOUT_DIR/target/release/clawd" ]] ||
  die "preserved_runtime_missing_clawd"
[[ -f "$CHECKOUT_DIR/configs/config.toml" ]] ||
  die "preserved_runtime_missing_config"

BACKUP_ROOT="$ROOT_PARENT/.${ROOT_NAME}-source-backups"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
BACKUP_DIR="$BACKUP_ROOT/release-${TIMESTAMP}-$$"
mkdir -p "$BACKUP_ROOT"

printf 'source_checkout_step=activating\n'
mv "$ROOT_DIR" "$BACKUP_DIR"
ORIGINAL_MOVED=1
if ! mv "$CHECKOUT_DIR" "$ROOT_DIR"; then
  mv "$BACKUP_DIR" "$ROOT_DIR" || true
  ORIGINAL_MOVED=0
  die "source_checkout_activation_failed"
fi
MIGRATION_COMMITTED=1

python3 - "$BACKUP_ROOT" "$KEEP_BACKUPS" <<'PY'
from pathlib import Path
import shutil
import sys

root = Path(sys.argv[1])
keep = int(sys.argv[2])
backups = sorted(
    (path for path in root.iterdir() if path.is_dir() and path.name.startswith("release-")),
    key=lambda path: path.stat().st_mtime,
    reverse=True,
)
for path in backups[keep:]:
    shutil.rmtree(path)
PY

printf 'source_checkout_status=enabled\n'
printf 'source_checkout_backup=%s\n' "$BACKUP_DIR"
printf 'source_checkout_restart_required=1\n'
