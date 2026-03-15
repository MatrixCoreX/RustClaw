#!/usr/bin/env bash
set -euo pipefail

# Test external APIs used by crypto skill (from configs/crypto.toml)
#
# Usage:
#   ./scripts/test_crypto_apis.sh [--config PATH] [--address 0x...] [--token usdt] [--timeout 10] [--help]

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_PATH="$ROOT_DIR/configs/crypto.toml"
HTTP_TIMEOUT="${HTTP_TIMEOUT:-12}"
ETH_TEST_ADDRESS="${ETH_TEST_ADDRESS:-0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045}" # vitalik.eth
ETH_TEST_TOKEN="${ETH_TEST_TOKEN:-usdt}"
VERBOSE=0

TOTAL=0
PASS=0
FAIL=0
FAIL_CASES=()

usage() {
  cat <<'EOF'
Usage:
  ./scripts/test_crypto_apis.sh [options]

Options:
  --config PATH        Path to crypto.toml (default: configs/crypto.toml)
  --address 0x...      ETH address for address-based API tests
  --token SYMBOL       Token symbol in [crypto.eth_token_contracts] (default: usdt)
  --timeout N          curl timeout seconds (default: 12)
  --verbose, -v        Print debug details for failed checks
  --help, -h           Show this help

Env:
  ETH_TEST_ADDRESS
  ETH_TEST_TOKEN
  HTTP_TIMEOUT
EOF
}

need_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "Missing command: $cmd"
    exit 2
  }
}

log() { printf '%s\n' "$*"; }
verbose_log() { [[ "$VERBOSE" -eq 1 ]] && log "$*"; }
pass() { PASS=$((PASS + 1)); log "[PASS] $1"; }
fail() {
  FAIL=$((FAIL + 1))
  FAIL_CASES+=("$1")
  log "[FAIL] $1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --config)
      CONFIG_PATH="${2:-}"
      shift 2
      ;;
    --address)
      ETH_TEST_ADDRESS="${2:-}"
      shift 2
      ;;
    --token)
      ETH_TEST_TOKEN="${2:-}"
      shift 2
      ;;
    --timeout)
      HTTP_TIMEOUT="${2:-}"
      shift 2
      ;;
    --verbose|-v)
      VERBOSE=1
      shift
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
done

need_cmd python3
need_cmd curl
need_cmd jq

if [[ ! -f "$CONFIG_PATH" ]]; then
  echo "Config not found: $CONFIG_PATH"
  exit 2
fi

read_cfg_json() {
  python3 - "$CONFIG_PATH" <<'PY'
import json
import sys
import tomllib
from pathlib import Path

p = Path(sys.argv[1])
cfg = tomllib.loads(p.read_text(encoding="utf-8"))
rss_path = p.parent / "rss.toml"
rss_cfg = tomllib.loads(rss_path.read_text(encoding="utf-8")) if rss_path.exists() else {}
crypto = cfg.get("crypto", {}) or {}
binance = cfg.get("binance", {}) or {}
okx = cfg.get("okx", {}) or {}
rss = rss_cfg.get("rss", {}) or {}
rss_crypto = (rss.get("categories") or {}).get("crypto") or {}
rss_first_feed = (
    (rss_crypto.get("primary") or [None])[0]
    or (rss_crypto.get("secondary") or [None])[0]
    or (rss_crypto.get("fallback") or [None])[0]
    or ""
)
print(json.dumps({
    "rss_first_feed": rss_first_feed,
    "btc_onchain_fees_api_url": crypto.get("btc_onchain_fees_api_url", ""),
    "eth_onchain_stats_api_url": crypto.get("eth_onchain_stats_api_url", ""),
    "coingecko_simple_price_api_url": crypto.get("coingecko_simple_price_api_url", ""),
    "binance_quote_24hr_api_path": crypto.get("binance_quote_24hr_api_path", "/api/v3/ticker/24hr?symbol={symbol}"),
    "binance_quote_price_api_path": crypto.get("binance_quote_price_api_path", "/api/v3/ticker/price?symbol={symbol}"),
    "binance_book_ticker_api_path": crypto.get("binance_book_ticker_api_path", "/api/v3/ticker/bookTicker?symbol={symbol}"),
    "okx_market_ticker_api_path": crypto.get("okx_market_ticker_api_path", "/api/v5/market/ticker?instId={inst_id}"),
    "eth_address_native_balance_api_url": crypto.get("eth_address_native_balance_api_url", ""),
    "eth_address_token_balance_api_url": crypto.get("eth_address_token_balance_api_url", ""),
    "eth_address_tx_list_api_url": crypto.get("eth_address_tx_list_api_url", ""),
    "eth_token_contracts": crypto.get("eth_token_contracts") or {},
    "eth_token_decimals": crypto.get("eth_token_decimals") or {},
    "binance_base_url": binance.get("base_url", "https://api.binance.com"),
    "okx_base_url": okx.get("base_url", "https://www.okx.com"),
}))
PY
}

render_url() {
  local template="$1"
  local address="$2"
  local contract="$3"
  local limit="$4"
  python3 - "$template" "$address" "$contract" "$limit" <<'PY'
import sys
from urllib.parse import quote

tpl = sys.argv[1]
address = quote(sys.argv[2], safe="")
contract = quote(sys.argv[3], safe="")
limit = quote(sys.argv[4], safe="")

out = tpl.replace("{address}", address).replace("{contract}", contract).replace("{limit}", limit)
print(out)
PY
}

build_exchange_url() {
  local base="$1"
  local path_or_url="$2"
  local symbol="$3"
  local inst_id="$4"
  python3 - "$base" "$path_or_url" "$symbol" "$inst_id" <<'PY'
import sys
from urllib.parse import quote

base = sys.argv[1].rstrip("/")
tpl = sys.argv[2]
symbol = quote(sys.argv[3], safe="")
inst_id = quote(sys.argv[4], safe="")
out = tpl.replace("{symbol}", symbol).replace("{inst_id}", inst_id).replace("{instId}", inst_id)
if out.startswith("http://") or out.startswith("https://"):
    print(out)
else:
    print(f"{base}/{out.lstrip('/')}")
PY
}

http_get_json() {
  local url="$1"
  curl -sSL --max-time "$HTTP_TIMEOUT" "$url"
}

run_case() {
  local name="$1"
  local cmd="$2"
  TOTAL=$((TOTAL + 1))
  if eval "$cmd" >/dev/null 2>&1; then
    pass "$name"
  else
    verbose_log "  command: $cmd"
    fail "$name"
  fi
}

snippet() {
  local text="$1"
  python3 - "$text" <<'PY'
import sys
s = sys.argv[1].replace("\n", "\\n")
print(s[:400])
PY
}

run_json_case() {
  local name="$1"
  local url="$2"
  local jq_expr="$3"
  TOTAL=$((TOTAL + 1))
  local body=""
  local ok=0
  if body="$(http_get_json "$url")"; then
    if echo "$body" | jq -e "$jq_expr" >/dev/null 2>&1; then
      ok=1
    fi
  fi
  if [[ "$ok" -eq 1 ]]; then
    pass "$name"
    return 0
  fi
  if [[ "$VERBOSE" -eq 1 ]]; then
    log "  url: $url"
    log "  jq : $jq_expr"
    if [[ -n "$body" ]]; then
      log "  body: $(snippet "$body")"
    else
      log "  body: <empty>"
    fi
  fi
  fail "$name"
  return 1
}

run_news_feed_case() {
  local name="$1"
  local url="$2"
  TOTAL=$((TOTAL + 1))
  local body=""
  local ok=0
  if [[ -n "$url" ]]; then
    if body="$(curl -sSL --max-time "$HTTP_TIMEOUT" "$url")"; then
      if [[ "$body" == *"<item"* || "$body" == *"<entry"* ]]; then
        ok=1
      fi
    fi
  fi
  if [[ "$ok" -eq 1 ]]; then
    pass "$name"
    return 0
  fi
  if [[ "$VERBOSE" -eq 1 ]]; then
    log "  url: $url"
    if [[ -n "$body" ]]; then
      log "  body: $(snippet "$body")"
    else
      log "  body: <empty>"
    fi
  fi
  fail "$name"
  return 1
}

CFG_JSON="$(read_cfg_json)"

btc_fees_url="$(echo "$CFG_JSON" | jq -r '.btc_onchain_fees_api_url')"
eth_stats_url="$(echo "$CFG_JSON" | jq -r '.eth_onchain_stats_api_url')"
cg_tpl="$(echo "$CFG_JSON" | jq -r '.coingecko_simple_price_api_url')"
binance_quote_24hr_path="$(echo "$CFG_JSON" | jq -r '.binance_quote_24hr_api_path')"
binance_quote_price_path="$(echo "$CFG_JSON" | jq -r '.binance_quote_price_api_path')"
binance_book_ticker_path="$(echo "$CFG_JSON" | jq -r '.binance_book_ticker_api_path')"
okx_market_ticker_path="$(echo "$CFG_JSON" | jq -r '.okx_market_ticker_api_path')"
eth_native_tpl="$(echo "$CFG_JSON" | jq -r '.eth_address_native_balance_api_url')"
eth_token_tpl="$(echo "$CFG_JSON" | jq -r '.eth_address_token_balance_api_url')"
eth_tx_tpl="$(echo "$CFG_JSON" | jq -r '.eth_address_tx_list_api_url')"
binance_base="$(echo "$CFG_JSON" | jq -r '.binance_base_url')"
okx_base="$(echo "$CFG_JSON" | jq -r '.okx_base_url')"
token_contract="$(echo "$CFG_JSON" | jq -r --arg t "$ETH_TEST_TOKEN" '.eth_token_contracts[$t] // .eth_token_contracts[($t|ascii_upcase)] // ""')"

if [[ -z "$token_contract" && "$ETH_TEST_TOKEN" != "eth" ]]; then
  log "[WARN] token '$ETH_TEST_TOKEN' has no contract in config; token balance test will be skipped."
fi

coingecko_url="$(python3 - "$cg_tpl" <<'PY'
import sys
tpl = sys.argv[1]
ids = "bitcoin"
if "{ids}" in tpl:
    print(tpl.replace("{ids}", ids))
elif "?" in tpl:
    print(f"{tpl}&ids={ids}&vs_currencies=usd&include_24hr_change=true")
else:
    print(f"{tpl}?ids={ids}&vs_currencies=usd&include_24hr_change=true")
PY
)"

binance_quote_24hr_url="$(build_exchange_url "$binance_base" "$binance_quote_24hr_path" "BTCUSDT" "BTC-USDT")"
binance_quote_price_url="$(build_exchange_url "$binance_base" "$binance_quote_price_path" "BTCUSDT" "BTC-USDT")"
binance_book_ticker_url="$(build_exchange_url "$binance_base" "$binance_book_ticker_path" "BTCUSDT" "BTC-USDT")"
okx_market_ticker_url="$(build_exchange_url "$okx_base" "$okx_market_ticker_path" "BTCUSDT" "BTC-USDT")"

eth_native_url="$(render_url "$eth_native_tpl" "$ETH_TEST_ADDRESS" "" "5")"
eth_tx_url="$(render_url "$eth_tx_tpl" "$ETH_TEST_ADDRESS" "" "5")"
eth_token_url=""
if [[ -n "$token_contract" ]]; then
  eth_token_url="$(render_url "$eth_token_tpl" "$ETH_TEST_ADDRESS" "$token_contract" "5")"
fi

log "== Crypto API connectivity tests =="
log "config: $CONFIG_PATH"
log "address: $ETH_TEST_ADDRESS"
log "token: $ETH_TEST_TOKEN"
log "verbose: $VERBOSE"
log

first_feed="$(echo "$CFG_JSON" | jq -r '.rss_first_feed // empty')"
run_news_feed_case "news feed[0] reachable" "$first_feed"

run_json_case "btc onchain fees api" "$btc_fees_url" ".fastestFee or .halfHourFee or .hourFee"
run_json_case "eth onchain stats api" "$eth_stats_url" ".data"
run_json_case "coingecko simple price api" "$coingecko_url" ".bitcoin.usd"
run_json_case "binance quote price api (config)" "$binance_quote_price_url" ".price"
run_json_case "binance quote 24hr api (config)" "$binance_quote_24hr_url" ".lastPrice or .price or .last or .close"
run_json_case "binance bookTicker api (config)" "$binance_book_ticker_url" ".bidPrice and .askPrice"
run_json_case "okx market ticker api (config)" "$okx_market_ticker_url" ".code == \"0\" and (.data|length>0)"
run_json_case "eth address native balance api" "$eth_native_url" ".result != null"

if [[ -n "$eth_token_url" ]]; then
  run_json_case "eth address token balance api ($ETH_TEST_TOKEN)" "$eth_token_url" ".result != null"
fi

run_json_case "eth address recent tx api" "$eth_tx_url" ".result != null"

log
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

log "All crypto API tests passed."
