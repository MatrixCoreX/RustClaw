#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

SKILL_NAME="${SKILL_NAME:-}"
DEFAULT_ARGS="${DEFAULT_ARGS:-}"
if [[ -z "$DEFAULT_ARGS" ]]; then
  DEFAULT_ARGS='{}'
fi
PROFILE="release"
AUTO_BUILD=0
RAW=0
ARGS_JSON=""
USER_ID="${USER_ID:-1}"
CHAT_ID="${CHAT_ID:-1}"

usage_common() {
  cat <<EOF
Usage:
  bash scripts/skill_calls/call_<skill>.sh [options]

Options:
  --profile debug|release   Runner profile (default: release)
  --args '<json>'           Args JSON passed to skill (default: wrapper preset)
  --user-id N               Request user_id (default: 1)
  --chat-id N               Request chat_id (default: 1)
  --auto-build              Auto build missing runner/skill binary
  --raw                     Print raw one-line JSON response
  --help, -h                Show help

Examples:
  bash scripts/skill_calls/call_crypto.sh --args '{"action":"quote","symbol":"BTCUSDT"}'
  bash scripts/skill_calls/call_health_check.sh
EOF
}

need_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "Missing command: $cmd"
    exit 2
  }
}

skill_bin_name() {
  case "$1" in
    x) echo "x-skill" ;;
    system_basic) echo "system-basic-skill" ;;
    http_basic) echo "http-basic-skill" ;;
    git_basic) echo "git-basic-skill" ;;
    install_module) echo "install-module-skill" ;;
    process_basic) echo "process-basic-skill" ;;
    package_manager) echo "package-manager-skill" ;;
    archive_basic) echo "archive-basic-skill" ;;
    db_basic) echo "db-basic-skill" ;;
    docker_basic) echo "docker-basic-skill" ;;
    fs_search) echo "fs-search-skill" ;;
    rss_fetch) echo "rss-fetch-skill" ;;
    image_vision) echo "image-vision-skill" ;;
    image_generate) echo "image-generate-skill" ;;
    image_edit) echo "image-edit-skill" ;;
    audio_transcribe) echo "audio-transcribe-skill" ;;
    audio_synthesize) echo "audio-synthesize-skill" ;;
    health_check) echo "health-check-skill" ;;
    log_analyze) echo "log-analyze-skill" ;;
    service_control) echo "service-control-skill" ;;
    config_guard) echo "config-guard-skill" ;;
    crypto) echo "crypto-skill" ;;
    *) return 1 ;;
  esac
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    --args)
      ARGS_JSON="${2:-}"
      shift 2
      ;;
    --user-id)
      USER_ID="${2:-1}"
      shift 2
      ;;
    --chat-id)
      CHAT_ID="${2:-1}"
      shift 2
      ;;
    --auto-build)
      AUTO_BUILD=1
      shift
      ;;
    --raw)
      RAW=1
      shift
      ;;
    --help|-h)
      usage_common
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      usage_common
      exit 2
      ;;
  esac
done

if [[ -z "$SKILL_NAME" ]]; then
  echo "SKILL_NAME is empty in wrapper."
  exit 2
fi

if [[ "$PROFILE" != "debug" && "$PROFILE" != "release" ]]; then
  echo "--profile must be debug or release"
  exit 2
fi

need_cmd jq

if [[ -z "$ARGS_JSON" ]]; then
  ARGS_JSON="$DEFAULT_ARGS"
fi
echo "$ARGS_JSON" | jq -e . >/dev/null 2>&1 || {
  echo "--args is not valid JSON: $ARGS_JSON"
  exit 2
}

RUNNER="$ROOT_DIR/target/$PROFILE/skill-runner"
ALT_PROFILE="debug"
if [[ "$PROFILE" == "debug" ]]; then
  ALT_PROFILE="release"
fi
ALT_RUNNER="$ROOT_DIR/target/$ALT_PROFILE/skill-runner"
if [[ ! -x "$RUNNER" && -x "$ALT_RUNNER" ]]; then
  RUNNER="$ALT_RUNNER"
fi

if [[ ! -x "$RUNNER" ]]; then
  if [[ "$AUTO_BUILD" != "1" ]]; then
    echo "skill-runner not found: $RUNNER"
    echo "Try: ./build-all.sh $PROFILE or rerun with --auto-build"
    exit 1
  fi
  if [[ "$PROFILE" == "release" ]]; then
    (cd "$ROOT_DIR" && cargo build -p skill-runner --release)
  else
    (cd "$ROOT_DIR" && cargo build -p skill-runner)
  fi
  if [[ ! -x "$RUNNER" && -x "$ALT_RUNNER" ]]; then
    RUNNER="$ALT_RUNNER"
  fi
fi

if [[ "$AUTO_BUILD" == "1" ]]; then
  if bin_name="$(skill_bin_name "$SKILL_NAME")"; then
    skill_bin="$ROOT_DIR/target/$PROFILE/$bin_name"
    alt_skill_bin="$ROOT_DIR/target/$ALT_PROFILE/$bin_name"
    if [[ ! -x "$skill_bin" && ! -x "$alt_skill_bin" ]]; then
      if [[ "$PROFILE" == "release" ]]; then
        (cd "$ROOT_DIR" && cargo build --bin "$bin_name" --release)
      else
        (cd "$ROOT_DIR" && cargo build --bin "$bin_name")
      fi
    fi
  fi
fi

request_id="skill-call-${SKILL_NAME}-$(date +%s)-$RANDOM"
req="$(
  jq -nc \
    --arg rid "$request_id" \
    --arg skill "$SKILL_NAME" \
    --argjson args "$ARGS_JSON" \
    --argjson uid "$USER_ID" \
    --argjson cid "$CHAT_ID" \
    '{
      request_id: $rid,
      user_id: $uid,
      chat_id: $cid,
      skill_name: $skill,
      args: $args,
      context: null
    }'
)"

resp="$(printf '%s\n' "$req" | "$RUNNER")"

if [[ "$RAW" == "1" ]]; then
  printf '%s\n' "$resp"
  exit 0
fi

echo "$resp" | jq .
