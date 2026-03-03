#!/usr/bin/env bash
set -euo pipefail

# Crypto skill regression (direct via skill-runner)
# Usage:
#   ./scripts/regression_crypto_skill.sh [debug|release] [--auto-build] [--help]

PROFILE="debug"
AUTO_BUILD=0
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="$ROOT_DIR/target/$PROFILE/skill-runner"

TOTAL=0
PASS=0
FAIL=0
FAIL_CASES=()

need_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 && return 0
  echo "Missing command: $cmd"
  if [[ "$cmd" == "jq" ]]; then
    echo "Install hint:"
    echo "  Ubuntu/Debian: sudo apt install -y jq"
    echo "  macOS (brew):  brew install jq"
  fi
  exit 2
}

log() { printf '%s\n' "$*"; }
pass() { PASS=$((PASS + 1)); log "[PASS] $1"; }
fail() {
  FAIL=$((FAIL + 1))
  FAIL_CASES+=("$1")
  log "[FAIL] $1"
}

usage() {
  cat <<'EOF'
Usage:
  ./scripts/regression_crypto_skill.sh [debug|release] [--auto-build] [--help]

Options:
  debug|release  Choose build profile (default: debug)
  --auto-build   Build missing binaries automatically
  --help, -h     Show this help
EOF
}

run_skill_raw() {
  local args_json="$1"
  local req_id
  req_id="crypto-reg-$(date +%s)-$RANDOM"

  local req
  req="$(jq -nc \
    --arg rid "$req_id" \
    --argjson args "$args_json" \
    '{
      request_id: $rid,
      user_id: 1,
      chat_id: 1,
      skill_name: "crypto",
      args: $args,
      context: null
    }')"

  printf '%s\n' "$req" | "$RUNNER"
}

assert_status() {
  local resp="$1"
  local expected="$2"
  echo "$resp" | jq -e --arg s "$expected" '.status == $s' >/dev/null
}

assert_error_contains() {
  local resp="$1"
  local needle="$2"
  echo "$resp" | jq -e --arg n "$needle" '
    (.error_text // "") | tostring | test($n)
  ' >/dev/null
}

assert_action() {
  local resp="$1"
  local action="$2"
  echo "$resp" | jq -e --arg a "$action" '.extra.action == $a' >/dev/null
}

runner_knows_crypto() {
  local resp
  resp="$(run_skill_raw '{"action":"positions","exchange":"paper"}' 2>/dev/null || true)"
  [[ -n "$resp" ]] || return 1
  if echo "$resp" | jq -e '.error_text // "" | tostring | test("unknown skill: crypto")' >/dev/null; then
    return 1
  fi
  return 0
}

run_case_ok() {
  local name="$1"
  local args_json="$2"
  local action="$3"

  TOTAL=$((TOTAL + 1))
  local resp
  if ! resp="$(run_skill_raw "$args_json" 2>/dev/null)"; then
    fail "$name (runner execution failed)"
    return
  fi

  if ! assert_status "$resp" "ok"; then
    fail "$name (status not ok): $resp"
    return
  fi

  if ! assert_action "$resp" "$action"; then
    fail "$name (action mismatch): $resp"
    return
  fi

  pass "$name"
}

run_case_err_contains() {
  local name="$1"
  local args_json="$2"
  local needle="$3"

  TOTAL=$((TOTAL + 1))
  local resp
  if ! resp="$(run_skill_raw "$args_json" 2>/dev/null)"; then
    fail "$name (runner execution failed)"
    return
  fi

  if ! assert_status "$resp" "error"; then
    fail "$name (status not error): $resp"
    return
  fi

  if ! assert_error_contains "$resp" "$needle"; then
    fail "$name (error_text missing '$needle'): $resp"
    return
  fi

  pass "$name"
}

main() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      debug|release)
        PROFILE="$1"
        ;;
      --auto-build)
        AUTO_BUILD=1
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      *)
        log "Unknown argument: $1"
        usage
        exit 2
        ;;
    esac
    shift
  done

  RUNNER="$ROOT_DIR/target/$PROFILE/skill-runner"

  need_cmd jq
  need_cmd python3

  if [[ ! -x "$RUNNER" ]]; then
    log "Runner not found: $RUNNER"
    if [[ "$AUTO_BUILD" == "1" ]]; then
      log "[INFO] auto-build enabled, building missing binaries..."
      if [[ "$PROFILE" == "release" ]]; then
        (cd "$ROOT_DIR" && cargo build --release -p skill-runner -p crypto-skill)
      else
        (cd "$ROOT_DIR" && cargo build -p skill-runner -p crypto-skill)
      fi
      if [[ ! -x "$RUNNER" ]]; then
        log "Build completed but runner still missing: $RUNNER"
        exit 2
      fi
    elif [[ "$PROFILE" == "release" ]]; then
      log "Try: cargo build --release -p skill-runner -p crypto-skill"
    else
      log "Try: cargo build -p skill-runner -p crypto-skill"
      log "Or run: ./scripts/regression_crypto_skill.sh $PROFILE --auto-build"
    fi
    if [[ "$AUTO_BUILD" != "1" ]]; then
      exit 2
    fi
  fi

  if ! runner_knows_crypto; then
    log "Current runner does not recognize crypto skill."
    if [[ "$AUTO_BUILD" == "1" ]]; then
      log "[INFO] rebuilding runner + crypto skill..."
      if [[ "$PROFILE" == "release" ]]; then
        (cd "$ROOT_DIR" && cargo build --release -p skill-runner -p crypto-skill)
      else
        (cd "$ROOT_DIR" && cargo build -p skill-runner -p crypto-skill)
      fi
      if ! runner_knows_crypto; then
        log "Runner still does not recognize crypto after build."
        exit 2
      fi
    else
      log "Run with --auto-build to rebuild binaries automatically."
      exit 2
    fi
  fi

  log "== Crypto regression start (profile=$PROFILE) =="

  run_case_ok "quote BTCUSDT" \
    '{"action":"quote","symbol":"BTCUSDT"}' \
    "quote"

  run_case_ok "multi_quote BTC/ETH/SOL" \
    '{"action":"multi_quote","symbols":["BTCUSDT","ETHUSDT","SOLUSDT"]}' \
    "multi_quote"

  run_case_ok "indicator ETH 1h SMA14" \
    '{"action":"indicator","symbol":"ETHUSDT","timeframe":"1h","period":14}' \
    "indicator"

  run_case_ok "onchain bitcoin" \
    '{"action":"onchain","chain":"bitcoin"}' \
    "onchain"

  run_case_ok "trade_preview market buy" \
    '{"action":"trade_preview","exchange":"paper","symbol":"BTCUSDT","side":"buy","order_type":"market","qty":0.01}' \
    "trade_preview"

  run_case_err_contains "trade_submit without confirm rejected" \
    '{"action":"trade_submit","exchange":"paper","symbol":"BTCUSDT","side":"buy","order_type":"market","qty":0.01}' \
    "confirm=true"

  TOTAL=$((TOTAL + 1))
  submit_resp="$(run_skill_raw '{"action":"trade_submit","exchange":"paper","symbol":"ETHUSDT","side":"buy","order_type":"limit","qty":0.02,"price":1000,"confirm":true}' 2>/dev/null || true)"
  if [[ -z "${submit_resp:-}" ]]; then
    fail "trade_submit limit confirm=true (runner failed)"
  elif ! assert_status "$submit_resp" "ok"; then
    fail "trade_submit limit confirm=true (status not ok): $submit_resp"
  elif ! assert_action "$submit_resp" "trade_submit"; then
    fail "trade_submit limit confirm=true (action mismatch): $submit_resp"
  else
    pass "trade_submit limit confirm=true"
  fi

  order_id="$(echo "$submit_resp" | jq -r '.extra.order.order_id // empty')"
  order_status="$(echo "$submit_resp" | jq -r '.extra.order.status // empty')"

  if [[ -n "$order_id" ]]; then
    run_case_ok "order_status by order_id" \
      "$(jq -nc --arg oid "$order_id" '{"action":"order_status","order_id":$oid}')" \
      "order_status"

    if [[ "$order_status" == "NEW" ]]; then
      run_case_ok "cancel_order NEW order" \
        "$(jq -nc --arg oid "$order_id" '{"action":"cancel_order","order_id":$oid}')" \
        "cancel_order"

      run_case_ok "order_status after cancel" \
        "$(jq -nc --arg oid "$order_id" '{"action":"order_status","order_id":$oid}')" \
        "order_status"
    else
      log "[INFO] order status is $order_status, skip cancel branch"
    fi
  else
    TOTAL=$((TOTAL + 1))
    fail "extract order_id from submit response"
  fi

  run_case_ok "positions paper" \
    '{"action":"positions","exchange":"paper"}' \
    "positions"

  log ""
  log "== Summary =="
  log "TOTAL: $TOTAL"
  log "PASS : $PASS"
  log "FAIL : $FAIL"

  if [[ "$FAIL" -gt 0 ]]; then
    log "Failed cases:"
    for c in "${FAIL_CASES[@]}"; do
      log " - $c"
    done
    exit 1
  fi

  log "All crypto regression cases passed."
}

main "$@"
