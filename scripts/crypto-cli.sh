#!/usr/bin/env bash
# =============================================================================
# crypto-cli.sh — crypto-skill 单独调用工具
# =============================================================================
# 协议：单行 JSON stdin → 单行 JSON stdout
# 二进制：target/debug/crypto-skill  或  target/release/crypto-skill
# 配置：WORKSPACE_ROOT 下的 configs/crypto.toml (或 configs/config.toml)
# =============================================================================

set -euo pipefail

# ---------- 路径解析 ----------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="${WORKSPACE_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
BIN_DEBUG="$WORKSPACE/target/debug/crypto-skill"
BIN_RELEASE="$WORKSPACE/target/release/crypto-skill"

# 优先 debug（最新编译），release 作为 fallback
if [[ -x "$BIN_DEBUG" ]]; then
    BIN="$BIN_DEBUG"
elif [[ -x "$BIN_RELEASE" ]]; then
    BIN="$BIN_RELEASE"
else
    echo "错误：未找到 crypto-skill 二进制，请先 cargo build -p crypto-skill" >&2
    exit 1
fi

# ---------- 工具函数 ----------
REQ_ID="cli-$$"

# 美化输出：优先 jq，否则原样
fmt_json() {
    if command -v jq &>/dev/null; then
        jq '.' 2>/dev/null || cat
    else
        cat
    fi
}

# 核心调用：传入 JSON args 对象，返回格式化响应
call_crypto() {
    local args="$1"
    local req="{\"request_id\":\"${REQ_ID}\",\"args\":${args}}"
    local resp
    resp=$(echo "$req" | WORKSPACE_ROOT="$WORKSPACE" "$BIN")
    local status text err
    status=$(echo "$resp" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('status','?'))" 2>/dev/null || echo "?")
    text=$(echo "$resp"   | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('text',''))"   2>/dev/null || echo "")
    err=$(echo "$resp"    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('error_text') or '')" 2>/dev/null || echo "")

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "状态: $status"
    if [[ -n "$text" ]]; then
        echo "文本:"
        echo "$text"
    fi
    if [[ -n "$err" ]]; then
        echo "错误: $err"
    fi
    if [[ "${SHOW_EXTRA:-0}" == "1" ]]; then
        echo "--- extra JSON ---"
        echo "$resp" | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d.get('extra') or {}, indent=2, ensure_ascii=False))" 2>/dev/null || true
    fi
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
}

# 显示完整 JSON 响应（不解析）
call_crypto_raw() {
    local args="$1"
    local req="{\"request_id\":\"${REQ_ID}\",\"args\":${args}}"
    echo "$req" | WORKSPACE_ROOT="$WORKSPACE" "$BIN" | fmt_json
}

# ---------- 使用说明 ----------
usage() {
cat <<'EOF'
用法：
  crypto-cli.sh <命令> [参数...]
  SHOW_EXTRA=1 crypto-cli.sh <命令>   # 显示完整 extra JSON
  RAW=1 crypto-cli.sh <命令>           # 显示原始 JSON 响应

━━━━━━━━━━ 行情 / 市场数据 ━━━━━━━━━━

  quote <SYMBOL>
      单币种聚合报价（Binance/OKX/Gate/Coinbase/Kraken/CoinGecko）
      示例：crypto-cli.sh quote BTCUSDT

  multi-quote <SYM1> [SYM2] ...
      多币种批量报价（最多 20 个）
      示例：crypto-cli.sh multi-quote BTC ETH SOL DOGE

  book-ticker <SYMBOL> [EXCHANGE]
      最优买一/卖一盘口（默认 dual 聚合多交易所）
      示例：crypto-cli.sh book-ticker ETHUSDT binance

  candles <SYMBOL> [TIMEFRAME] [LIMIT] [EXCHANGE]
      K 线数据，返回完整 OHLCV（open/high/low/close/volume）
      TIMEFRAME：1m 3m 5m 15m 30m 1h 2h 4h 6h 8h 12h 1d 3d 1w 1M
      示例：crypto-cli.sh candles BTCUSDT 4h 50 binance

  indicator <SYMBOL> [INDICATOR] [PERIOD] [TIMEFRAME] [EXCHANGE]
      技术指标：sma / ema / rsi（默认 sma，period 默认 14）
      示例：crypto-cli.sh indicator BTCUSDT rsi 14 1h
      示例：crypto-cli.sh indicator ETHUSDT ema 20 4h okx

  price-alert <SYMBOL> [WINDOW_MIN] [THRESHOLD_PCT] [DIRECTION] [EXCHANGE]
      价格波动检测（direction：up/down/both）
      返回 [PRICE_ALERT_TRIGGERED] 或 [PRICE_ALERT_NOT_TRIGGERED] 前缀
      示例：crypto-cli.sh price-alert BTCUSDT 15 3.0 both

  onchain [CHAIN] [ADDRESS]
      链上数据：bitcoin=BTC 费率，ethereum=ETH 网络 / 地址余额
      示例：crypto-cli.sh onchain bitcoin
      示例：crypto-cli.sh onchain ethereum 0xYourAddress

  symbol-check <SYMBOL>
      验证交易对在 Binance 上是否存在并返回 LOT_SIZE 等过滤参数
      示例：crypto-cli.sh symbol-check DOGEUSDT

━━━━━━━━━━ 账户 / 订单查询 ━━━━━━━━━━

  positions [EXCHANGE]
      账户持仓余额（非零资产列表）
      示例：crypto-cli.sh positions binance

  open-orders [EXCHANGE] [SYMBOL]
      查询所有未成交挂单
      示例：crypto-cli.sh open-orders binance BTCUSDT
      示例：crypto-cli.sh open-orders binance

  trade-history <EXCHANGE> <SYMBOL> [LIMIT]
      成交历史（Binance: myTrades; OKX: fills）
      示例：crypto-cli.sh trade-history binance DOGEUSDT 20

  order-status <EXCHANGE> <SYMBOL> <ORDER_ID>
      查询单个订单状态
      示例：crypto-cli.sh order-status binance BTCUSDT 123456789

━━━━━━━━━━ 下单（需要 API 密钥 + 测试谨慎！）━━━━━━━━━━

  preview <EXCHANGE> <SYMBOL> <SIDE> <ORDER_TYPE> <QTY_OR_QUOTE>
      交易预览（不下单，显示风控检查结果）
      示例：crypto-cli.sh preview binance DOGEUSDT buy market quote=5
      示例：crypto-cli.sh preview binance BTCUSDT sell limit qty=0.001 price=95000
      示例：crypto-cli.sh preview binance BTCUSDT sell stop_loss_limit qty=0.001 price=94000 stop=95000

  cancel-order <EXCHANGE> <SYMBOL> <ORDER_ID>
      撤销单个订单
      示例：crypto-cli.sh cancel-order binance BTCUSDT 123456789

  cancel-all <EXCHANGE> <SYMBOL>
      撤销某交易对所有挂单
      示例：crypto-cli.sh cancel-all binance BTCUSDT

━━━━━━━━━━ 直接传 JSON ━━━━━━━━━━

  raw '<JSON_ARGS>'
      直接传递 args JSON，原始输出
      示例：crypto-cli.sh raw '{"action":"quote","symbol":"SOLUSDT"}'

  raw-call '<JSON_ARGS>'
      直接传递 args JSON，格式化输出（同上但加状态标头）

环境变量：
  WORKSPACE_ROOT   项目根目录（默认为脚本上一级目录）
  SHOW_EXTRA=1     同时输出 extra JSON 字段
  RAW=1            输出原始 JSON（等同于 raw 子命令）

EOF
}

# ---------- 子命令分发 ----------
CMD="${1:-help}"
shift || true

case "$CMD" in

  # ── 行情 ──────────────────────────────────────────────────
  quote)
    SYMBOL="${1:?用法: crypto-cli.sh quote <SYMBOL>}"
    ARGS="{\"action\":\"quote\",\"symbol\":\"$SYMBOL\"}"
    ;;

  multi-quote|multi_quote)
    [[ $# -lt 1 ]] && { echo "用法: crypto-cli.sh multi-quote SYM1 SYM2 ..."; exit 1; }
    SYMS=$(printf '"%s",' "$@" | sed 's/,$//')
    ARGS="{\"action\":\"multi_quote\",\"symbols\":[$SYMS]}"
    ;;

  book-ticker|book_ticker)
    SYMBOL="${1:?用法: crypto-cli.sh book-ticker <SYMBOL> [EXCHANGE]}"
    EXCHANGE="${2:-dual}"
    ARGS="{\"action\":\"get_book_ticker\",\"symbol\":\"$SYMBOL\",\"exchange\":\"$EXCHANGE\"}"
    ;;

  candles)
    SYMBOL="${1:?用法: crypto-cli.sh candles <SYMBOL> [TIMEFRAME] [LIMIT] [EXCHANGE]}"
    TF="${2:-1h}"
    LIMIT="${3:-30}"
    EXCHANGE="${4:-binance}"
    ARGS="{\"action\":\"candles\",\"symbol\":\"$SYMBOL\",\"timeframe\":\"$TF\",\"limit\":$LIMIT,\"exchange\":\"$EXCHANGE\"}"
    ;;

  indicator)
    SYMBOL="${1:?用法: crypto-cli.sh indicator <SYMBOL> [INDICATOR] [PERIOD] [TIMEFRAME] [EXCHANGE]}"
    INDTYPE="${2:-sma}"
    PERIOD="${3:-14}"
    TF="${4:-1h}"
    EXCHANGE="${5:-binance}"
    ARGS="{\"action\":\"indicator\",\"symbol\":\"$SYMBOL\",\"indicator\":\"$INDTYPE\",\"period\":$PERIOD,\"timeframe\":\"$TF\",\"exchange\":\"$EXCHANGE\"}"
    ;;

  price-alert|price_alert)
    SYMBOL="${1:?用法: crypto-cli.sh price-alert <SYMBOL> [WINDOW_MIN] [THRESHOLD_PCT] [DIRECTION] [EXCHANGE]}"
    WINDOW="${2:-15}"
    THRESH="${3:-5.0}"
    DIR="${4:-both}"
    EXCHANGE="${5:-binance}"
    ARGS="{\"action\":\"price_alert_check\",\"symbol\":\"$SYMBOL\",\"window_minutes\":$WINDOW,\"threshold_pct\":$THRESH,\"direction\":\"$DIR\",\"exchange\":\"$EXCHANGE\"}"
    ;;

  onchain)
    CHAIN="${1:-bitcoin}"
    ADDR="${2:-}"
    if [[ -n "$ADDR" ]]; then
        ARGS="{\"action\":\"onchain\",\"chain\":\"$CHAIN\",\"address\":\"$ADDR\"}"
    else
        ARGS="{\"action\":\"onchain\",\"chain\":\"$CHAIN\"}"
    fi
    ;;

  symbol-check|symbol_check)
    SYMBOL="${1:?用法: crypto-cli.sh symbol-check <SYMBOL>}"
    ARGS="{\"action\":\"binance_symbol_check\",\"symbol\":\"$SYMBOL\"}"
    ;;

  # ── 账户 / 订单 ───────────────────────────────────────────
  positions)
    EXCHANGE="${1:-binance}"
    ARGS="{\"action\":\"positions\",\"exchange\":\"$EXCHANGE\"}"
    ;;

  open-orders|open_orders)
    EXCHANGE="${1:-binance}"
    SYMBOL="${2:-}"
    if [[ -n "$SYMBOL" ]]; then
        ARGS="{\"action\":\"open_orders\",\"exchange\":\"$EXCHANGE\",\"symbol\":\"$SYMBOL\"}"
    else
        ARGS="{\"action\":\"open_orders\",\"exchange\":\"$EXCHANGE\"}"
    fi
    ;;

  trade-history|trade_history)
    EXCHANGE="${1:?用法: crypto-cli.sh trade-history <EXCHANGE> <SYMBOL> [LIMIT]}"
    SYMBOL="${2:?用法: crypto-cli.sh trade-history <EXCHANGE> <SYMBOL> [LIMIT]}"
    LIMIT="${3:-20}"
    ARGS="{\"action\":\"trade_history\",\"exchange\":\"$EXCHANGE\",\"symbol\":\"$SYMBOL\",\"limit\":$LIMIT}"
    ;;

  order-status|order_status)
    EXCHANGE="${1:?用法: crypto-cli.sh order-status <EXCHANGE> <SYMBOL> <ORDER_ID>}"
    SYMBOL="${2:?缺少 SYMBOL}"
    ORDER_ID="${3:?缺少 ORDER_ID}"
    ARGS="{\"action\":\"order_status\",\"exchange\":\"$EXCHANGE\",\"symbol\":\"$SYMBOL\",\"order_id\":\"$ORDER_ID\"}"
    ;;

  # ── 下单 / 撤单 ───────────────────────────────────────────
  preview)
    # preview <EXCHANGE> <SYMBOL> <SIDE> <ORDER_TYPE> [qty=N | quote=N] [price=N] [stop=N]
    EXCHANGE="${1:?用法: crypto-cli.sh preview <EXCHANGE> <SYMBOL> <SIDE> <ORDER_TYPE> ...}"
    SYMBOL="${2:?缺少 SYMBOL}"
    SIDE="${3:-buy}"
    ORDER_TYPE="${4:-market}"
    shift 4 || true
    QTY_PART=""
    PRICE_PART=""
    STOP_PART=""
    for arg in "$@"; do
        case "$arg" in
            quote=*) VAL="${arg#quote=}"; QTY_PART=",\"quote_qty_usd\":$VAL" ;;
            qty=*)   VAL="${arg#qty=}";   QTY_PART=",\"qty\":$VAL"           ;;
            price=*) VAL="${arg#price=}"; PRICE_PART=",\"price\":$VAL"       ;;
            stop=*)  VAL="${arg#stop=}";  STOP_PART=",\"stop_price\":$VAL"   ;;
        esac
    done
    ARGS="{\"action\":\"trade_preview\",\"exchange\":\"$EXCHANGE\",\"symbol\":\"$SYMBOL\",\"side\":\"$SIDE\",\"order_type\":\"$ORDER_TYPE\"${QTY_PART}${PRICE_PART}${STOP_PART}}"
    ;;

  cancel-order|cancel_order)
    EXCHANGE="${1:?用法: crypto-cli.sh cancel-order <EXCHANGE> <SYMBOL> <ORDER_ID>}"
    SYMBOL="${2:?缺少 SYMBOL}"
    ORDER_ID="${3:?缺少 ORDER_ID}"
    ARGS="{\"action\":\"cancel_order\",\"exchange\":\"$EXCHANGE\",\"symbol\":\"$SYMBOL\",\"order_id\":\"$ORDER_ID\"}"
    ;;

  cancel-all|cancel_all)
    EXCHANGE="${1:?用法: crypto-cli.sh cancel-all <EXCHANGE> <SYMBOL>}"
    SYMBOL="${2:?缺少 SYMBOL（Binance 必填）}"
    ARGS="{\"action\":\"cancel_all_orders\",\"exchange\":\"$EXCHANGE\",\"symbol\":\"$SYMBOL\"}"
    ;;

  # ── 直接传 JSON ───────────────────────────────────────────
  raw)
    JSON_ARGS="${1:?用法: crypto-cli.sh raw '<JSON_ARGS>'}"
    if [[ "${RAW:-0}" == "1" ]]; then
        call_crypto_raw "$JSON_ARGS"
    else
        call_crypto_raw "$JSON_ARGS"
    fi
    exit 0
    ;;

  raw-call)
    JSON_ARGS="${1:?用法: crypto-cli.sh raw-call '<JSON_ARGS>'}"
    ARGS="$JSON_ARGS"
    ;;

  help|--help|-h|"")
    usage
    exit 0
    ;;

  *)
    echo "未知命令: $CMD" >&2
    echo "运行 crypto-cli.sh help 查看帮助" >&2
    exit 1
    ;;
esac

# ---------- 执行调用 ----------
if [[ "${RAW:-0}" == "1" ]]; then
    call_crypto_raw "$ARGS"
else
    call_crypto "$ARGS"
fi
