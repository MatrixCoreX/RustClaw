#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT_DIR/scripts/shell_compat.sh"

SKILL_NAME="${SKILL_NAME:-}"
DEFAULT_ARGS="${DEFAULT_ARGS:-}"
if [[ -z "$DEFAULT_ARGS" ]]; then
  DEFAULT_ARGS='{}'
fi
PROFILE="release"
AUTO_BUILD=0
RAW=0
ALLOW_NETWORK=0
TIMEOUT_SECONDS="${SKILL_TIMEOUT_SECONDS:-}"
ARGS_JSON=""
CONTEXT_JSON="${CONTEXT_JSON:-null}"
USER_ID="${USER_ID:-1}"
CHAT_ID="${CHAT_ID:-1}"

usage_common() {
  cat <<EOF
Usage:
  bash scripts/skill_calls/call_<skill>.sh [options]

Options:
  --profile release         Runner profile (default: release)
  --args '<json>'           Args JSON passed to skill (default: wrapper preset)
  --context '<json>'        Structured runner context (default: null)
  --user-id N               Request user_id (default: 1)
  --chat-id N               Request chat_id (default: 1)
  --auto-build              Auto build missing runner/skill binary
  --network                 Approve manifest-declared install network for this run
  --timeout-seconds N       Optional runner cap; defaults to the skill manifest timeout
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
    --context)
      CONTEXT_JSON="${2:-}"
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
    --network)
      ALLOW_NETWORK=1
      shift
      ;;
    --timeout-seconds)
      TIMEOUT_SECONDS="${2:-}"
      shift 2
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

if [[ "$PROFILE" != "release" ]]; then
  echo "--profile must be release"
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
echo "$CONTEXT_JSON" | jq -e '. == null or type == "object"' >/dev/null 2>&1 || {
  echo "--context must be a JSON object or null"
  exit 2
}
if [[ -n "$TIMEOUT_SECONDS" && ! "$TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]]; then
  echo "--timeout-seconds must be a positive integer"
  exit 2
fi
if [[ -n "$TIMEOUT_SECONDS" && "$TIMEOUT_SECONDS" -gt 86400 ]]; then
  echo "--timeout-seconds must not exceed 86400"
  exit 2
fi

RUNNER="$ROOT_DIR/target/$PROFILE/skill-runner"
SKILL_RECORD="$(
  python3 "$ROOT_DIR/scripts/skill_store_packages.py" \
    --scope selected --target host --skill "$SKILL_NAME" --format records
)" || exit 1
canonical_skill="$(jq -r '.skill_name' <<<"$SKILL_RECORD")"
manifest_path="$(jq -r '.manifest_path' <<<"$SKILL_RECORD")"
SDK_CLI="$ROOT_DIR/target/$PROFILE/skillctl"
PACKAGE_ROOT="$ROOT_DIR/data/skill-packages"

if [[ ! -x "$RUNNER" ]]; then
  if [[ "$AUTO_BUILD" != "1" ]]; then
    echo "skill-runner not found: $RUNNER"
    echo "Try: ./build-all.sh $PROFILE or rerun with --auto-build"
    exit 1
  fi
  configure_cargo_build_environment
  (cd "$ROOT_DIR" && cargo build -p skill-runner --release)
fi

if [[ "$AUTO_BUILD" == "1" ]]; then
  if [[ ! -x "$SDK_CLI" ]]; then
    configure_cargo_build_environment
    (cd "$ROOT_DIR" && cargo build -p agent-skill-sdk --release)
  fi
  install_args=(install-local "$manifest_path" "$ROOT_DIR" "$PACKAGE_ROOT")
  [[ "$ALLOW_NETWORK" == "1" ]] && install_args+=(--network)
  "$SDK_CLI" "${install_args[@]}" >/dev/null
elif [[ ! -f "$PACKAGE_ROOT/$canonical_skill/current.json" ]]; then
  echo "verified skill receipt not found: $canonical_skill"
  echo "Install it from Skill Store or rerun with --auto-build"
  exit 1
fi

pointer_path="$PACKAGE_ROOT/$canonical_skill/current.json"
install_dir="$(jq -er '.install_dir | select(type == "string" and length > 0)' "$pointer_path")"
pointer_receipt_digest="$(jq -er '.receipt_digest | select(type == "string" and length == 64)' "$pointer_path")"
receipt_path="$PACKAGE_ROOT/$canonical_skill/versions/$install_dir/install-receipt.json"
if [[ ! -f "$receipt_path" ]]; then
  echo "installed skill receipt is missing: $receipt_path"
  exit 1
fi
expected_skill_version="$(jq -er '.version | select(type == "string" and length > 0)' "$receipt_path")"
expected_manifest_digest="$(jq -er '.manifest_digest | select(type == "string" and length == 64)' "$receipt_path")"
expected_receipt_digest="$pointer_receipt_digest"
jq -e --arg skill "$canonical_skill" --arg digest "$pointer_receipt_digest" \
  '.skill_name == $skill and .schema_version >= 1 and $digest != ""' \
  "$receipt_path" >/dev/null || {
  echo "installed skill receipt does not match the selected skill: $canonical_skill"
  exit 1
}

request_id="skill-call-${SKILL_NAME}-$(date +%s)-$RANDOM"
req="$(
  jq -nc \
    --arg rid "$request_id" \
    --arg skill "$canonical_skill" \
    --arg version "$expected_skill_version" \
    --arg manifest_digest "$expected_manifest_digest" \
    --arg receipt_digest "$expected_receipt_digest" \
    --argjson args "$ARGS_JSON" \
    --argjson context "$CONTEXT_JSON" \
    --argjson uid "$USER_ID" \
    --argjson cid "$CHAT_ID" \
    '{
      request_id: $rid,
      user_id: $uid,
      chat_id: $cid,
      skill_name: $skill,
      expected_skill_version: $version,
      expected_manifest_digest: $manifest_digest,
      expected_receipt_digest: $receipt_digest,
      expected_registry_generation: 0,
      expected_registry_generation_digest: null,
      expected_base_registry_digest: null,
      expected_overlay_generation_digest: null,
      expected_policy_digest: null,
      expected_admission_receipt_digest: null,
      args: $args,
      context: $context
    }'
)"

if [[ -n "$TIMEOUT_SECONDS" ]]; then
  export SKILL_TIMEOUT_SECONDS="$TIMEOUT_SECONDS"
else
  unset SKILL_TIMEOUT_SECONDS
fi

resp="$(printf '%s\n' "$req" | \
  WORKSPACE_ROOT="$ROOT_DIR" \
  APP_SKILL_PACKAGES_ROOT="$PACKAGE_ROOT" \
  "$RUNNER")"

if [[ "$RAW" == "1" ]]; then
  printf '%s\n' "$resp"
  exit 0
fi

echo "$resp" | jq .
