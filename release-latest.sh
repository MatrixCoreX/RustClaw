#!/usr/bin/env bash
# Create and push the latest RustClaw release tags.
#
# Default behavior:
#   - push current main branch to origin
#   - create one Ubuntu tag and one Raspberry Pi tag for today's date
#   - push those tags, which triggers the GitHub release workflows
#
# Examples:
#   ./release-latest.sh
#   ./release-latest.sh --platform ubuntu
#   ./release-latest.sh --platform pi --date 20260622
#   ./release-latest.sh --dry-run
set -euo pipefail

REMOTE="origin"
REQUIRED_BRANCH="main"
RELEASE_DATE="$(date +%Y%m%d)"
REF="HEAD"
FETCH_FIRST=1
PUSH_BRANCH=1
DRY_RUN=0
STRICT_CLEAN=0
ALLOW_NON_MAIN=0
PLATFORMS=("ubuntu-x86_64" "pi-aarch64")

usage() {
  cat <<'USAGE'
Usage: ./release-latest.sh [options]

Create Git tags that trigger GitHub Actions release builds.

Options:
  --platform all|ubuntu|pi|ubuntu-x86_64|pi-aarch64
      Platform to release. Default: all.
      Can be passed multiple times or as comma-separated values.
  --date YYYYMMDD
      Date used in tag names. Default: today from local system date.
  --ref REF
      Commit/ref to tag. Default: HEAD.
  --remote NAME
      Git remote to push to. Default: origin.
  --no-fetch
      Do not fetch remote branch/tags before creating tags.
  --no-push-branch
      Do not push the current branch before pushing tags.
  --allow-non-main
      Allow creating tags when the current branch is not main.
  --strict-clean
      Fail when the working tree has uncommitted changes.
  --dry-run
      Print the git commands without creating or pushing tags.
  -h, --help
      Show this help.

Tag format:
  ubuntu-x86_64-YYYYMMDD
  pi-aarch64-YYYYMMDD

If a tag already exists, the script creates the next free suffix:
  ubuntu-x86_64-YYYYMMDD-2
USAGE
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '%s\n' "$*"
}

quote_command() {
  local first=1
  local arg
  for arg in "$@"; do
    if [[ "$first" -eq 1 ]]; then
      first=0
    else
      printf ' '
    fi
    printf '%q' "$arg"
  done
  printf '\n'
}

run() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '+ '
    quote_command "$@"
  else
    "$@"
  fi
}

append_platform() {
  local raw="$1"
  case "$raw" in
    all)
      PLATFORMS=("ubuntu-x86_64" "pi-aarch64")
      ;;
    ubuntu|ubuntu-x86_64)
      PLATFORMS+=("ubuntu-x86_64")
      ;;
    pi|raspi|raspberry-pi|pi-aarch64)
      PLATFORMS+=("pi-aarch64")
      ;;
    *)
      die "unknown platform: $raw"
      ;;
  esac
}

set_platforms_from_arg() {
  local raw="$1"
  local item
  if [[ "$raw" == "all" ]]; then
    PLATFORMS=("ubuntu-x86_64" "pi-aarch64")
    return
  fi
  if [[ "${PLATFORM_ARG_SEEN:-0}" -eq 0 ]]; then
    PLATFORMS=()
    PLATFORM_ARG_SEEN=1
  fi
  IFS=',' read -ra items <<< "$raw"
  for item in "${items[@]}"; do
    item="${item//[[:space:]]/}"
    [[ -n "$item" ]] || continue
    append_platform "$item"
  done
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --platform)
      [[ $# -ge 2 ]] || die "--platform requires a value"
      set_platforms_from_arg "$2"
      shift 2
      ;;
    --platform=*)
      set_platforms_from_arg "${1#*=}"
      shift
      ;;
    --date)
      [[ $# -ge 2 ]] || die "--date requires YYYYMMDD"
      RELEASE_DATE="$2"
      shift 2
      ;;
    --date=*)
      RELEASE_DATE="${1#*=}"
      shift
      ;;
    --ref)
      [[ $# -ge 2 ]] || die "--ref requires a value"
      REF="$2"
      shift 2
      ;;
    --ref=*)
      REF="${1#*=}"
      shift
      ;;
    --remote)
      [[ $# -ge 2 ]] || die "--remote requires a value"
      REMOTE="$2"
      shift 2
      ;;
    --remote=*)
      REMOTE="${1#*=}"
      shift
      ;;
    --no-fetch)
      FETCH_FIRST=0
      shift
      ;;
    --no-push-branch)
      PUSH_BRANCH=0
      shift
      ;;
    --allow-non-main)
      ALLOW_NON_MAIN=1
      shift
      ;;
    --strict-clean)
      STRICT_CLEAN=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

[[ "$RELEASE_DATE" =~ ^[0-9]{8}$ ]] || die "--date must be YYYYMMDD: $RELEASE_DATE"
[[ "${#PLATFORMS[@]}" -gt 0 ]] || die "no release platforms selected"

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

if [[ "$FETCH_FIRST" -eq 1 ]]; then
  log "[1/5] Fetch remote branch and tags..."
  run git fetch "$REMOTE" "$REQUIRED_BRANCH" --tags
else
  log "[1/5] Skip fetch."
fi

CURRENT_BRANCH="$(git branch --show-current || true)"
if [[ "$REF" == "HEAD" && "$ALLOW_NON_MAIN" -ne 1 && "$CURRENT_BRANCH" != "$REQUIRED_BRANCH" ]]; then
  die "current branch is '$CURRENT_BRANCH', expected '$REQUIRED_BRANCH'. Use --allow-non-main to override."
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
  if [[ "$STRICT_CLEAN" -eq 1 ]]; then
    git status --short
    die "working tree has uncommitted changes; commit or stash them first"
  fi
  log "[warning] Working tree has uncommitted changes. Release tags point to committed Git history only:"
  git status --short
fi

COMMIT="$(git rev-parse --verify "${REF}^{commit}")"
SHORT_COMMIT="$(git rev-parse --short=12 "$COMMIT")"

if [[ "$PUSH_BRANCH" -eq 1 && "$REF" == "HEAD" && -n "$CURRENT_BRANCH" ]]; then
  log "[2/5] Push current branch '$CURRENT_BRANCH' to '$REMOTE'..."
  run git push "$REMOTE" "$CURRENT_BRANCH"
else
  log "[2/5] Skip branch push."
fi

tag_exists() {
  local tag="$1"
  if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
    return 0
  fi
  git ls-remote --exit-code --tags "$REMOTE" "refs/tags/${tag}" >/dev/null 2>&1
}

next_tag_for_platform() {
  local platform="$1"
  local base="${platform}-${RELEASE_DATE}"
  local tag="$base"
  local suffix=2
  while tag_exists "$tag"; do
    tag="${base}-${suffix}"
    suffix=$((suffix + 1))
  done
  printf '%s\n' "$tag"
}

unique_platforms=()
for platform in "${PLATFORMS[@]}"; do
  duplicate=0
  for existing in "${unique_platforms[@]}"; do
    if [[ "$existing" == "$platform" ]]; then
      duplicate=1
      break
    fi
  done
  [[ "$duplicate" -eq 0 ]] && unique_platforms+=("$platform")
done
PLATFORMS=("${unique_platforms[@]}")

log "[3/5] Resolve release tags for commit ${SHORT_COMMIT}..."
TAGS=()
for platform in "${PLATFORMS[@]}"; do
  tag="$(next_tag_for_platform "$platform")"
  TAGS+=("$tag")
  log "  ${platform}: ${tag}"
done

log "[4/5] Create local annotated tags..."
for tag in "${TAGS[@]}"; do
  run git tag -a "$tag" "$COMMIT" -m "RustClaw release ${tag} (${SHORT_COMMIT})"
done

log "[5/5] Push release tags..."
push_args=("$REMOTE")
for tag in "${TAGS[@]}"; do
  push_args+=("refs/tags/${tag}")
done
run git push "${push_args[@]}"

REPO_URL="$(git remote get-url "$REMOTE" 2>/dev/null || true)"
case "$REPO_URL" in
  git@github.com:*.git)
    REPO_URL="https://github.com/${REPO_URL#git@github.com:}"
    REPO_URL="${REPO_URL%.git}"
    ;;
  https://github.com/*.git)
    REPO_URL="${REPO_URL%.git}"
    ;;
esac

log ""
if [[ "$DRY_RUN" -eq 1 ]]; then
  log "Release tags planned:"
else
  log "Release tags submitted:"
fi
for tag in "${TAGS[@]}"; do
  log "  - ${tag}"
  if [[ "$REPO_URL" == https://github.com/* ]]; then
    log "    ${REPO_URL}/releases/tag/${tag}"
  fi
done

if [[ "$REPO_URL" == https://github.com/* ]]; then
  log ""
  log "Workflow pages:"
  log "  ${REPO_URL}/actions/workflows/ubuntu-x86_64-release.yml"
  log "  ${REPO_URL}/actions/workflows/pi-aarch64-release.yml"
fi
