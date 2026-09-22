#!/usr/bin/env bash
# Bootstrap a verified, platform-specific Release without a source checkout.
set -euo pipefail

ROOT_DIR="$HOME/agent-runtime"
REPOSITORY=""
SOURCE_REF="main"
START=1
CHECK_ONLY=0

usage() {
  cat <<'USAGE'
Usage: bash install-latest-release.sh --repo OWNER/REPO [options]
  --root DIR       Installation directory (default: ~/agent-runtime).
  --ref REF        Trusted bootstrap source ref (default: main).
  --no-start       Install without starting services.
  --check-only     Show the matching latest Release without installing.

Requires Bash, Python 3.11+, curl, tar and OpenSSH ssh-keygen.
Uses HTTPS GitHub bootstrap files, then verifies the signed Release before
installation. Never compiles source or configures a new nginx site.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo|--root|--ref)
      [[ $# -ge 2 && -n "$2" ]] || { usage >&2; exit 2; }
      case "$1" in
        --repo) REPOSITORY="$2" ;;
        --root) ROOT_DIR="$2" ;;
        --ref) SOURCE_REF="$2" ;;
      esac
      shift 2 ;;
    --no-start) START=0; shift ;;
    --check-only) CHECK_ONLY=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

[[ "$REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ &&
   "$SOURCE_REF" =~ ^[A-Za-z0-9_.-]+$ ]] || { usage >&2; exit 2; }
case "$(uname -s):$(uname -m)" in
  Linux:x86_64|Linux:aarch64|Linux:arm64|Darwin:x86_64|Darwin:arm64) ;;
  *) echo 'Unsupported platform; no source-build fallback.' >&2; exit 1 ;;
esac
for command in python3 curl tar ssh-keygen; do
  command -v "$command" >/dev/null || { echo "Missing dependency: $command" >&2; exit 1; }
done
python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else "Python 3.11+ is required")'
ROOT_DIR="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).expanduser().resolve())' "$ROOT_DIR")"
[[ "$ROOT_DIR" != / && "$ROOT_DIR" != "$HOME" ]] || { echo 'Choose a dedicated installation directory.' >&2; exit 1; }

BOOTSTRAP="$(mktemp -d "${TMPDIR:-/tmp}/agent-release-bootstrap.XXXXXX")"
trap 'rm -rf "$BOOTSTRAP"' EXIT
download() {
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
    --connect-timeout 20 --max-time 180 --retry 3 "$1" --output "$2"
}
download "https://api.github.com/repos/$REPOSITORY/commits/$SOURCE_REF" "$BOOTSTRAP/commit.json"
COMMIT="$(python3 - "$BOOTSTRAP/commit.json" <<'PY'
import json, re, sys
with open(sys.argv[1]) as stream:
    commit = json.load(stream).get("sha", "")
if not re.fullmatch(r"[0-9a-f]{40}", commit):
    raise SystemExit("Invalid bootstrap commit")
print(commit)
PY
)"
for relative in deploy-github-release.sh scripts/product_identity.sh \
  scripts/verify_release_binary.py scripts/security/release_manifest.py \
  configs/product_identity.toml configs/release_allowed_signers; do
  mkdir -p "$BOOTSTRAP/$(dirname "$relative")"
  download "https://raw.githubusercontent.com/$REPOSITORY/$COMMIT/$relative" "$BOOTSTRAP/$relative"
done
# Preserve an existing installation's trust anchor. A fresh installation trusts
# the explicitly selected repository over HTTPS, not keys inside the archive.
if [[ -f "$ROOT_DIR/configs/release_allowed_signers" ]]; then
  export APP_RELEASE_ALLOWED_SIGNERS_FILE="$ROOT_DIR/configs/release_allowed_signers"
else
  export APP_RELEASE_ALLOWED_SIGNERS_FILE="$BOOTSTRAP/configs/release_allowed_signers"
fi
export APP_RELEASE_MANIFEST_TOOL="$BOOTSTRAP/scripts/security/release_manifest.py"
export APP_PRODUCT_IDENTITY_CONFIG="$BOOTSTRAP/configs/product_identity.toml"
source "$BOOTSTRAP/scripts/product_identity.sh"
[[ "$APP_RELEASE_REPOSITORY" == "$REPOSITORY" ]] || { echo 'Repository does not match product identity.' >&2; exit 1; }
printf 'bootstrap_commit=%s\n' "$COMMIT"
if [[ "$CHECK_ONLY" -eq 1 ]]; then
  bash "$BOOTSTRAP/deploy-github-release.sh" --root "$ROOT_DIR" --check-only
  exit 0
fi
restart_option=--no-restart
[[ "$START" -eq 0 ]] || restart_option=--restart
bash "$BOOTSTRAP/deploy-github-release.sh" --root "$ROOT_DIR" "$restart_option"
export APP_PRODUCT_IDENTITY_CONFIG="$ROOT_DIR/configs/product_identity.toml"
bash "$ROOT_DIR/install-agent-cmd.sh" --user --no-deploy-ui
printf 'Installed in %s\n' "$ROOT_DIR"
printf 'Open the configured webd address (default: http://127.0.0.1:8788).\n'
