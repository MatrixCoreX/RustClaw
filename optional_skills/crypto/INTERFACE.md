# crypto Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with `optional_skills/crypto/src/main.rs`.

## Capability Summary
- `crypto` provides market data queries, technical indicators, on-chain lookups, and full spot order lifecycle operations.
- It supports multi-exchange routing via `exchange` (mainly `binance` and `okx`; quote sources also include Gate.io, Coinbase, Kraken, CoinGecko).
- Private exchange actions (`trade_preview`, `trade_submit`, order status/cancel/open orders/history, `positions`) require bound exchange credentials. The skill checks the target exchange binding before parameter validation or private API calls; if the current `user_key` has no bound API, it returns a clear “API not bound” error.
- **Planner boundary:** the ordinary agent loop can resolve the provider-free `preview_quote_request` plus `quote`, `multi_quote`, and `positions` through `crypto.preview_quote`, `crypto.quote`, `crypto.multi_quote`, and `crypto.positions`.
- **Direct/admin boundary:** every other action is available only through an explicit structured `run_skill`/admin invocation. This interface documents those actions but does not expose them to ordinary planner selection.
- **Symbol / pair:** planner-visible calls require one unambiguous structured symbol. Direct/admin callers are responsible for resolving ambiguity before invocation.
- **Exchange default:** explicit `exchange` wins; otherwise the skill uses configured `crypto.execution_mode` / `crypto.default_exchange`.
- **Execution vs preview:** direct/admin callers use `trade_preview` for non-mutating preview and `trade_submit` for explicitly authorized execution. Runtime confirmation policy applies to the complete runner.
- Supported order types: `market`, `limit`, `stop_loss_limit`, `take_profit_limit`, `limit_maker` (Binance); `market`, `limit` (OKX).

## Config Entry Points

- Trading policy, exchange allow-lists, symbols, and risk limits remain in
  `configs/crypto.toml`.
- Exchange credentials are owned by `crypto` and persisted per `user_key` in
  the runtime-resolved crypto database (normally
  `data/skills/crypto/state.db`). They are never written to the main runtime
  database.
- The registry declares
  `storage = { kind = "sqlite", schema_version = 1, migration_owner = "crypto" }`;
  `[database].skill_data_root` in `configs/config.toml` controls the parent
  directory.
- Bind credentials through the authenticated credential API, the supported
  channel setup flow, or `scripts/import-crypto-credentials.sh`. Do not ask the
  user to paste raw secrets into ordinary agent conversation.
- A legacy main-database credential table is migrated once, verified, and
  physically dropped. New runtime code must use the crypto storage repository.

## Actions
- Market/info: `preview_quote_request` (provider-free normalization only), `quote` (aliases `price`, `get_price` when querying one symbol), `multi_quote` (aliases `get_multi_price`; `price` when `symbols` is present), `get_book_ticker` (alias `book_ticker`), `binance_symbol_check`, `normalize_symbol`, `healthcheck`, `candles` (aliases `kline`, `klines`, `candlestick`, `candlesticks`, `ohlcv`; these normalize to `indicator` when an `indicator` param is also present), `indicator` (aliases `technical_indicator`, `technical_indicators`, `ta_indicator`, `ta`), `price_alert_check`, `onchain`
- **Price-alert aliases** (normalize to `price_alert_check` internally, no separate actions): `price_monitor`, `monitor_price`, `price_alert`, `volatility_alert`.
- Trade/order: `trade_preview`, `trade_submit`, `order_status`, `cancel_order`, `cancel_all_orders` (alias `cancel_open_orders`), `open_orders` (alias `get_open_orders`, `pending_orders`), `trade_history` (alias `my_trades`, `recent_trades`), `positions`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Action name from the list above. `price` is accepted as a compatibility alias and normalizes to `quote` or `multi_quote` based on whether `symbols` is present. |
| many actions | `exchange` | no | string | config default | Exchange routing: `binance`, `okx`. If omitted, use `crypto.execution_mode` / `crypto.default_exchange`; if neither is configured, clarify instead of guessing a hardcoded fallback. |
| many actions | `symbol` | depends | string | - | Trading pair symbol. Normalize to canonical form only when uniquely identifiable; if ambiguous, planner must clarify first—do not guess. |
| `quote` | `symbol` | yes | string | - | Single symbol quote; aggregates Binance/OKX/Gate/Coinbase/Kraken/CoinGecko. |
| `multi_quote` | `symbols` or `symbol` | yes | array/string | - | Multi-symbol batch quote; max 20 symbols. |
| `preview_quote_request` | `symbols` or `symbol` | yes | array/string | - | Normalize up to 20 requested symbols and show the provider plan without credentials or network calls. |
| `get_book_ticker`/`book_ticker` | `symbol` | yes | string | - | Best bid/ask snapshot. |
| `get_book_ticker`/`book_ticker` | `exchange` | no | string | `dual` | `dual` aggregates multiple exchanges. |
| `binance_symbol_check` | `symbol` | yes | string | - | Validate symbol exists on Binance and return lot/filter info. |
| `normalize_symbol` | `symbol` | yes | string | - | Convert to canonical exchange forms. |
| `candles` | `symbol` | yes | string | - | K-line source symbol. |
| `candles` | `timeframe`/`interval` | no | string | `1h` | Candle interval: `1m`,`3m`,`5m`,`15m`,`30m`,`1h`,`2h`,`4h`,`6h`,`8h`,`12h`,`1d`,`3d`,`1w`,`1M`. |
| `candles` | `limit` | no | number | `30` | Candle count (max 500). Returns `close_prices` array and full `candles` OHLCV array. |
| `candles` | `exchange` | no | string | config default | `binance` or `okx`. If omitted, use `crypto.execution_mode` / `crypto.default_exchange`; if neither is configured, clarify. |
| `indicator` | `symbol` | yes | string | - | Symbol for computation. |
| `indicator` | `indicator` | no | string | `sma` | Indicator type: `sma`, `ema`, `rsi`. |
| `indicator` | `period` | no | number | `14` | Indicator period (2–200). |
| `indicator` | `timeframe`/`interval` | no | string | `1h` | Candle interval for source data. |
| `indicator` | `exchange` | no | string | config default | Data source exchange. If omitted, use `crypto.execution_mode` / `crypto.default_exchange`; if neither is configured, clarify. |
| `price_alert_check` | `symbol` | yes | string | - | Symbol to monitor (normalized). |
| `price_alert_check` | `exchange` | no | string | config default | Data source (`binance` or `okx`). If omitted: use config `execution_mode` / `default_exchange`; if neither is configured, clarify. |
| `price_alert_check` | `window_minutes`/`minutes` | no | number | **15** | **Lookback window** (minutes): compares the **latest** 1m close to the close from **~`window_minutes` ago** (not merely a poll interval). After `crypto.alert_default_window_minutes` in config, else **15**. Clamped to **`[5, alert_max_window_minutes]`** (minimum **5**; values `1`–`4` are raised to **5**). |
| `price_alert_check` | `threshold_pct`/`pct`/`percent` | no | number | **5** | After `crypto.alert_default_threshold_pct` in config, else **5** (must be > 0). |
| `price_alert_check` | `direction` | no | string | **`both`** | `up`/`down`/`both` (aliases: rise/drop/pump/dump). |
| `price_alert_check` | listing validation | — | — | — | **Inside this action only:** for non-OKX path, validates symbol against Binance listings (same effect as `binance_symbol_check`). Schedule and other layers must not pre-call `binance_symbol_check` for scheduled jobs. |
| `price_alert_check` | (schedule) | no | — | — | When `clawd` runs a scheduled `run_skill`, request **`context`** may include `schedule_job_id`, `invocation_source` (`schedule`), `scheduled`, `schedule_triggered`; skill echoes into response `extra` when set (`args` may duplicate for tests). |

### `price_alert_check` — semantics & response `extra`
- **Semantics:** Each run fetches **1m** candles covering the lookback span; **reference/base** price is the **oldest** close in that span (window start), **current** price is the **newest** close. Change % is \((current - reference) / reference × 100\). Threshold and `direction` (`up` / `down` / `both`) apply to that percentage.
- **User-visible text** states the lookback window, **reference/base** and **current** prices, change %, threshold, and direction (wording follows `configs/i18n/crypto.*.toml` / built-in defaults).
- **Structured `extra` (success):** includes at least `action`, `symbol`, `exchange`, `window_minutes`, `threshold_pct`, `direction`, `triggered`, `trend`, `change_pct`, **`reference_price`** (same numeric as window-start close), **`current_price`**, **`start_price`** (alias of `reference_price`, kept for backward compatibility), `candles` (count fetched), `notify` (same as `triggered`), plus optional schedule echo fields when present.

### `quote` / `multi_quote` — response `extra`
- `extra.content_excerpt`: compact quote text for runtime evidence checks. Consumers should use this structured field instead of depending on localized `text` parsing.
- `extra.quote` / `extra.quotes`: preferred quote objects with `symbol`, `price_usd`, `change_24h_pct`, `exchange`, and `source`.
- `extra.quotes_by_exchange`: per-exchange quote objects when available.

- `preview_quote_request` returns `requested_symbols`, `normalized_symbols`, provider tokens, `would_execute=false`, and `external_call_count=0`; it does not read credentials, fetch quotes, or submit orders.
| `onchain` | `chain` | no | string | `bitcoin` | `bitcoin`/`btc` or `ethereum`/`eth`. |
| `onchain` (eth address mode) | `address` | no | string | - | If provided, returns address balance + recent txs. |
| `onchain` (eth address mode) | `token` | no | string | `eth` | Native or configured ERC20 token symbol. |
| `onchain` (eth address mode) | `tx_limit`/`limit` | no | number | `5` | Recent tx count. |
| `trade_preview`/`trade_submit` | `symbol` | yes | string | - | Order symbol. |
| `trade_preview`/`trade_submit` | `side` | no* | string | `buy` | `buy` or `sell`. |
| `trade_preview`/`trade_submit` | `order_type` | no | string | `market` | `market`, `limit`, `stop_loss_limit`, `take_profit_limit`, `limit_maker`. Aliases: `type`, `orderType`. |
| `trade_preview`/`trade_submit` | `qty` | cond | number/string | - | Base asset quantity. Language-neutral compatibility tokens `"all"` / `"max"` are accepted for full-position sell (SELL side only). Aliases: `quantity`, `amount`, `base_qty`, `base_quantity`. `amount` means base-asset amount; use `quote_qty_usd`/`amount_usd` for quote-currency notional. |
| `trade_preview`/`trade_submit` | `qty_all` | cond | boolean | `false` | Preferred structured full-position sell marker. When the user asks in any natural language to sell the full position, planner must normalize that intent to `qty_all=true` instead of passing localized words in `qty`. |
| `trade_preview`/`trade_submit` | `quote_qty_usd` | cond | number | - | USDT amount to spend/receive. Aliases: `quote_qty`, `amount_usd`, `notional_usd`. |
| `trade_preview`/`trade_submit` | `price` | required for limit/stop orders | number | - | Required for `limit`, `stop_loss_limit`, `take_profit_limit`, `limit_maker`. |
| `trade_preview`/`trade_submit` | `stop_price` | required for stop orders | number | - | Trigger price for `stop_loss_limit` / `take_profit_limit`. Alias: `stopPrice`. |
| `trade_preview`/`trade_submit` | `time_in_force` | no | string | `GTC` | `GTC`/`IOC`/`FOK` for limit/stop orders (Binance). Alias: `timeInForce`. |
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

## Risk Rules
- **Respond**: Do not summarize unless the user explicitly asks for a summary. When the user did not ask for a summary, return only the skill result or one short necessary reply; no extra recap or conclusion.
- Ordinary planner calls are limited to the three registry-declared read capabilities. Do not construct unregistered `crypto.*` capability names or emit direct trading actions from the planner.
- Direct/admin callers must supply unambiguous structured symbols and parameters. Runtime must not derive authorization by matching user-visible text or localized confirmation phrases.
- `trade_preview` is non-mutating. `trade_submit` and cancel actions are mutating and require direct/admin authorization plus runtime confirmation policy.
- `trade_submit.confirm=true` records direct caller intent but does not replace runtime authorization.
- **`trade_preview` response `extra`**: includes structured **`order`** (submit-shaped fields) plus `effective_qty`, `notional_usd`, `risk_checks`, `decision=preview_only` for transparency; there is **no** platform-level second-step confirm chain in `clawd`.
- **Cancel safety:** direct callers must supply an order identifier for `cancel_order`. `cancel_all_orders` must be an explicit structured direct/admin action.
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
  - `Binance API is not bound for the current key yet...`
  - `OKX API is not bound for the current key yet...`
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
{"request_id":"demo-1","status":"ok","text":"ETHUSDT price_usd=3200.0 ...","error_text":null,"extra":{"action":"quote","content_excerpt":"ETHUSDT price_usd=3200.0 ...","quote":{"symbol":"ETHUSDT","price_usd":3200.0,"change_24h_pct":null,"exchange":"binance","source":"binance_api"},"quotes_by_exchange":{"binance":{"symbol":"ETHUSDT","price_usd":3200.0,"change_24h_pct":null,"exchange":"binance","source":"binance_api"}}}}
```

### Example 1b — Scheduled price alias with multiple symbols
Request:
```json
{"request_id":"demo-1b","args":{"action":"price","symbols":["BTC","ETH","DOGE"]}}
```
Behavior: normalized internally to `multi_quote`.

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

### Example 8 — Trade preview (market buy with USDT amount)
Request:
```json
{"request_id":"demo-8","args":{"action":"trade_preview","exchange":"binance","symbol":"DOGEUSDT","side":"buy","order_type":"market","quote_qty_usd":10}}
```
Response:
```json
{"request_id":"demo-8","status":"ok","text":"trade_preview binance DOGEUSDT buy est_qty=53.2468 quote_usd=10.0000 notional_usd=10.0000 checks=5","error_text":null,"extra":{"action":"trade_preview","order":{"exchange":"binance","symbol":"DOGEUSDT","side":"buy","order_type":"market","quote_qty_usd":10,"qty":53.2468},"effective_qty":53.2468,"notional_usd":10.0,"decision":"preview_only"}}
```
