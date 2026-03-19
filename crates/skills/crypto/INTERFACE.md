# crypto Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with `crates/skills/crypto/src/main.rs`.

## Capability Summary
- `crypto` provides market data queries, technical indicators, on-chain lookups, and full spot order lifecycle operations.
- It supports multi-exchange routing via `exchange` (mainly `binance` and `okx`; quote sources also include Gate.io, Coinbase, Kraken, CoinGecko).
- Trading actions require configured exchange credentials. For explicit place-order intents with complete params, the planner should call `trade_submit` directly and return a clear success/failure result.
- **Symbol / pair**: If the asset or trading pair is ambiguous or could map to multiple symbols, ask one concise clarification before calling trade/order/quote-affecting actions; do not guess `symbol`.
- **Execution vs preview**: `trade_preview` is for preview-only user intent. `trade_submit` is only when the **current** user message explicitly requests immediate execution (same turn). There is **no** platform-level second-step pending-confirm chain in `clawd`; do not rely on a later yes/no follow-up flow.
- Supported order types: `market`, `limit`, `stop_loss_limit`, `take_profit_limit`, `limit_maker` (Binance); `market`, `limit` (OKX).

## Actions
- Market/info: `quote`, `multi_quote`, `get_book_ticker` (alias `book_ticker`), `binance_symbol_check`, `normalize_symbol`, `healthcheck`, `candles`, `indicator`, `price_alert_check`, `onchain`
- **Price-alert aliases** (normalize to `price_alert_check` internally, no separate actions): `price_monitor`, `monitor_price`, `price_alert`, `volatility_alert`.
- Trade/order: `trade_preview`, `trade_submit`, `order_status`, `cancel_order`, `cancel_all_orders` (alias `cancel_open_orders`), `open_orders` (alias `get_open_orders`, `pending_orders`), `trade_history` (alias `my_trades`, `recent_trades`), `positions`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Action name from the list above. |
| many actions | `exchange` | no | string | config default / `binance` | Exchange routing: `binance`, `okx`. |
| many actions | `symbol` | depends | string | - | Trading pair symbol. Normalize to canonical form only when uniquely identifiable; if ambiguous, planner must clarify first—do not guess. |
| `quote` | `symbol` | yes | string | - | Single symbol quote; aggregates Binance/OKX/Gate/Coinbase/Kraken/CoinGecko. |
| `multi_quote` | `symbols` or `symbol` | yes | array/string | - | Multi-symbol batch quote; max 20 symbols. |
| `get_book_ticker`/`book_ticker` | `symbol` | yes | string | - | Best bid/ask snapshot. |
| `get_book_ticker`/`book_ticker` | `exchange` | no | string | `dual` | `dual` aggregates multiple exchanges. |
| `binance_symbol_check` | `symbol` | yes | string | - | Validate symbol exists on Binance and return lot/filter info. |
| `normalize_symbol` | `symbol` | yes | string | - | Convert to canonical exchange forms. |
| `candles` | `symbol` | yes | string | - | K-line source symbol. |
| `candles` | `timeframe` | no | string | `1h` | Candle interval: `1m`,`3m`,`5m`,`15m`,`30m`,`1h`,`2h`,`4h`,`6h`,`8h`,`12h`,`1d`,`3d`,`1w`,`1M`. |
| `candles` | `limit` | no | number | `30` | Candle count (max 500). Returns `close_prices` array and full `candles` OHLCV array. |
| `candles` | `exchange` | no | string | `binance` | `binance` or `okx`. |
| `indicator` | `symbol` | yes | string | - | Symbol for computation. |
| `indicator` | `indicator` | no | string | `sma` | Indicator type: `sma`, `ema`, `rsi`. |
| `indicator` | `period` | no | number | `14` | Indicator period (2–200). |
| `indicator` | `timeframe` | no | string | `1h` | Candle interval for source data. |
| `indicator` | `exchange` | no | string | `binance` | Data source exchange. |
| `price_alert_check` | `symbol` | yes | string | - | Symbol to monitor (normalized). |
| `price_alert_check` | `exchange` | no | string | `binance` | Data source (`binance` or `okx`). If omitted: config `default_exchange` / `execution_mode`, else **`binance`**. |
| `price_alert_check` | `window_minutes`/`minutes` | no | number | **15** | **Lookback window** (minutes): compares the **latest** 1m close to the close from **~`window_minutes` ago** (not merely a poll interval). After `crypto.alert_default_window_minutes` in config, else **15**. Clamped to **`[5, alert_max_window_minutes]`** (minimum **5**; values `1`–`4` are raised to **5**). |
| `price_alert_check` | `threshold_pct`/`pct`/`percent` | no | number | **5** | After `crypto.alert_default_threshold_pct` in config, else **5** (must be > 0). |
| `price_alert_check` | `direction` | no | string | **`both`** | `up`/`down`/`both` (aliases: rise/drop/pump/dump). |
| `price_alert_check` | listing validation | — | — | — | **Inside this action only:** for non-OKX path, validates symbol against Binance listings (same effect as `binance_symbol_check`). Schedule and other layers must not pre-call `binance_symbol_check` for scheduled jobs. |
| `price_alert_check` | (schedule) | no | — | — | When `clawd` runs a scheduled `run_skill`, request **`context`** may include `schedule_job_id`, `invocation_source` (`schedule`), `scheduled`, `schedule_triggered`; skill echoes into response `extra` when set (`args` may duplicate for tests). |

### `price_alert_check` — semantics & response `extra`
- **Semantics:** Each run fetches **1m** candles covering the lookback span; **reference/base** price is the **oldest** close in that span (window start), **current** price is the **newest** close. Change % is \((current - reference) / reference × 100\). Threshold and `direction` (`up` / `down` / `both`) apply to that percentage.
- **User-visible text** states the lookback window, **reference/base** and **current** prices, change %, threshold, and direction (wording follows `configs/i18n/crypto.*.toml` / built-in defaults).
- **Structured `extra` (success):** includes at least `action`, `symbol`, `exchange`, `window_minutes`, `threshold_pct`, `direction`, `triggered`, `trend`, `change_pct`, **`reference_price`** (same numeric as window-start close), **`current_price`**, **`start_price`** (alias of `reference_price`, kept for backward compatibility), `candles` (count fetched), `notify` (same as `triggered`), plus optional schedule echo fields when present.
| `onchain` | `chain` | no | string | `bitcoin` | `bitcoin`/`btc` or `ethereum`/`eth`. |
| `onchain` (eth address mode) | `address` | no | string | - | If provided, returns address balance + recent txs. |
| `onchain` (eth address mode) | `token` | no | string | `eth` | Native or configured ERC20 token symbol. |
| `onchain` (eth address mode) | `tx_limit`/`limit` | no | number | `5` | Recent tx count. |
| `trade_preview`/`trade_submit` | `symbol` | yes | string | - | Order symbol. |
| `trade_preview`/`trade_submit` | `side` | no* | string | `buy` | `buy` or `sell`. |
| `trade_preview`/`trade_submit` | `order_type` | no | string | `market` | `market`, `limit`, `stop_loss_limit`, `take_profit_limit`, `limit_maker`. |
| `trade_preview`/`trade_submit` | `qty` | cond | number/string | - | Base asset quantity. Use `"all"` for full-position sell (SELL side only). |
| `trade_preview`/`trade_submit` | `quote_qty_usd` | cond | number | - | USDT amount to spend/receive. Aliases: `quote_qty`, `amount_usd`, `notional_usd`. |
| `trade_preview`/`trade_submit` | `price` | required for limit/stop orders | number | - | Required for `limit`, `stop_loss_limit`, `take_profit_limit`, `limit_maker`. |
| `trade_preview`/`trade_submit` | `stop_price` | required for stop orders | number | - | Trigger price for `stop_loss_limit` / `take_profit_limit`. Alias: `stopPrice`. |
| `trade_preview`/`trade_submit` | `time_in_force` | no | string | `GTC` | `GTC`/`IOC`/`FOK` for limit/stop orders (Binance). |
| `trade_preview`/`trade_submit` | `client_order_id` | no | string | - | Client correlation id. |
| `trade_submit` | `confirm` | no | boolean | `false` | Set `true` only when the **current** user message explicitly indicates immediate / confirmed execution (same turn). Not for inferring confirmation from a prior preview turn or any deprecated yes/no host flow; no runtime enforcement. |
| `order_status` | `order_id` or `client_order_id` | yes | string | - | At least one order identifier. |
| `order_status` | `symbol` | conditional | string | - | Required by Binance/OKX query APIs. |
| `cancel_order` | `order_id` or `client_order_id` | yes | string | - | At least one order identifier. |
| `cancel_order` | `symbol` | conditional | string | - | Required by Binance/OKX cancel APIs. |
| `cancel_all_orders` | `symbol` | required (Binance) / optional (OKX) | string | - | Cancel all open orders for a symbol. Binance requires symbol; OKX cancels all if omitted. |
| `open_orders` | `symbol` | no | string | - | Filter by symbol; returns all open orders if omitted. |
| `open_orders` | `exchange` | no | string | config default | `binance` or `okx`. |
| `trade_history` | `symbol` | required (Binance) / optional (OKX) | string | - | Binance requires symbol; OKX returns all fills if omitted. |
| `trade_history` | `limit` | no | number | `20` | Number of trades to return (max 500). |
| `trade_history` | `exchange` | no | string | config default | `binance` or `okx`. |
| `positions` | none | no | - | - | Returns exchange account balances. |
| all | `timeout_seconds` | no | number | config default | Request timeout override (3–120s). |

## Risk Rules (Important for Agents)
- **Respond**: Do not summarize unless the user explicitly asks for a summary. When the user did not ask for a summary, return only the skill result or one short necessary reply; no extra recap or conclusion.
- **Symbol ambiguity (hard)**: If mapping from user wording to a single concrete `symbol` is ambiguous, low-confidence, or multi-valued, ask exactly one concise clarification (exact pair or coin) before any `trade_preview`, `trade_submit`, or other trade/order/cancel/status call that depends on `symbol`. Do not guess for execution or order paths.
- For explicit place-order intents with complete params **and** an unambiguous symbol in the **same** user message, prefer direct `trade_submit` with `confirm=true` and return a clear success/failure result.
- Use `trade_preview` when the user explicitly asks preview/estimate, or when key submit params are missing.
- `trade_submit` should be used only when the **current** user request itself explicitly indicates immediate execution / confirmed execution (same turn, e.g. clear “市价买入…执行下单” / “place it now” with full params). Pass `confirm=true` in that case. Do **not** treat a prior `trade_preview` plus a separate follow-up as a platform-managed confirmation chain—`clawd` does not host a second-step yes/no or pending-confirm flow.
- **`trade_preview` response `extra`**: includes structured **`order`** (submit-shaped fields) plus `effective_qty`, `notional_usd`, `risk_checks`, `decision=preview_only` for transparency; there is **no** platform-level second-step confirm chain in `clawd`.
- **Planner routing**: Explicit place-order in one message (e.g. “在0.09挂单5U狗狗币”) → `trade_submit` with `confirm=true` when symbol and params are unambiguous. Preview-only (e.g. “预览一下”“先算算”) → only `trade_preview`. Cancel one order → `cancel_order` (require `order_id` or `client_order_id`; if missing, call `open_orders` first or ask). Cancel all for symbol → `cancel_all_orders` only when user said “所有”/“全部” for that symbol. Query open orders → `open_orders` only (do not route “查挂单” to cancel). After `trade_submit`, success must include `order_id` or exchange status; failure must include concrete error reason. For trade_preview and trade_submit, prefer including `exchange` (e.g. binance, okx) when known.
- **Cancel safety**: Do not call `cancel_order` without at least one of `order_id` or `client_order_id` (or a prior step that supplies it). Do not call `cancel_all_orders` unless the user explicitly requested to cancel all orders or all for a symbol.
- Binance spot orders are subject to `min_notional_usd` (default 1.0 USDT; Binance actually requires ~10 USDT) and `max_notional_usd` limits.
- `qty=all` is only valid for `side=sell`.
- `stop_loss_limit`/`take_profit_limit` require both `price` (limit price) and `stop_price` (trigger price).

## Error Contract
- Common validation:
  - `args must be object`
  - `symbol is required`, `symbols or symbol is required`, `symbols is empty`
  - `side must be buy or sell`
  - `order_type must be market/limit/stop_loss_limit/take_profit_limit/limit_maker`
  - `qty is required and must be number`, `qty must be > 0`
  - `price is required for limit order`
  - `stop_loss_limit/take_profit_limit requires stop_price (trigger price)`
  - `qty=all is only supported for sell side`
  - `notional too small: ... < min_notional_usd=...`
- Action/exchange:
  - `unsupported action`
  - `unsupported execution exchange: {exchange}`
  - `unsupported exchange for open_orders|cancel_all_orders|trade_history: {exchange}`
- Order identifiers:
  - `order_id or client_order_id is required`
  - `cancel_all_orders on binance requires symbol`
  - `trade_history on binance requires symbol`
- Trading safety/policy:
  - `exchange is not allowed: {exchange}`
  - `symbol is not allowed: {symbol}`
  - `notional exceeds max_notional_usd: ...`
- On-chain/data failures return readable transport/parse errors.

## Request/Response Examples
### Example 1 — Market quote
Request:
```json
{"request_id":"demo-1","args":{"action":"quote","symbol":"ETHUSDT"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"ETHUSDT price_usd=3200.0 ...","error_text":null}
```

### Example 2 — Candles with OHLCV
Request:
```json
{"request_id":"demo-2","args":{"action":"candles","symbol":"BTCUSDT","timeframe":"4h","limit":50,"exchange":"binance"}}
```
Response extra contains `close_prices` (array of f64) and `candles` (array of `{open,high,low,close,volume,quote_volume}` objects).

### Example 3 — RSI indicator
Request:
```json
{"request_id":"demo-3","args":{"action":"indicator","symbol":"BTCUSDT","indicator":"rsi","period":14,"timeframe":"1h"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"BTCUSDT RSI14=58.23 last=104500.0 signal=neutral","error_text":null}
```

### Example 3b — Price alert / monitor (`price_alert_check`, lookback window)
Request (30-minute lookback, 5% threshold, both directions):
```json
{"request_id":"demo-3b","args":{"action":"price_alert_check","symbol":"BTCUSDT","window_minutes":30,"threshold_pct":5,"direction":"both","exchange":"binance"}}
```
Response `text` includes the lookback window, **reference/base** price, **current** price, change %, threshold, and direction. Response `extra` includes numeric `reference_price`, `current_price`, `change_pct`, `window_minutes`, `start_price` (same as `reference_price`), `triggered`, `trend`, `candles`, etc.

### Example 4 — Stop-loss limit order preview
Request:
```json
{"request_id":"demo-4","args":{"action":"trade_preview","exchange":"binance","symbol":"BTCUSDT","side":"sell","order_type":"stop_loss_limit","qty":0.001,"price":99000,"stop_price":99500}}
```

### Example 5 — Open orders query
Request:
```json
{"request_id":"demo-5","args":{"action":"open_orders","exchange":"binance","symbol":"BTCUSDT"}}
```

### Example 6 — Cancel all orders (Binance)
Request:
```json
{"request_id":"demo-6","args":{"action":"cancel_all_orders","exchange":"binance","symbol":"BTCUSDT"}}
```

### Example 7 — Trade history
Request:
```json
{"request_id":"demo-7","args":{"action":"trade_history","exchange":"binance","symbol":"DOGEUSDT","limit":10}}
```

### Example 8 — Trade preview (market buy with USDT amount)
Request:
```json
{"request_id":"demo-8","args":{"action":"trade_preview","exchange":"binance","symbol":"DOGEUSDT","side":"buy","order_type":"market","quote_qty_usd":10}}
```
Response:
```json
{"request_id":"demo-8","status":"ok","text":"trade_preview binance DOGEUSDT buy est_qty=53.2468 quote_usd=10.0000 notional_usd=10.0000 checks=5","error_text":null,"extra":{"action":"trade_preview","order":{"exchange":"binance","symbol":"DOGEUSDT","side":"buy","order_type":"market","quote_qty_usd":10,"qty":53.2468},"effective_qty":53.2468,"notional_usd":10.0,"decision":"preview_only"}}
```
