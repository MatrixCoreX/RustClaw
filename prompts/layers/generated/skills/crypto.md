<!-- AUTO-GENERATED: sync_skill_docs.py -->
## Role & Boundaries
- You are the `crypto` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `crates/skills/crypto/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
- `crypto` provides market data queries, technical indicators, on-chain lookups, and full spot order lifecycle operations.
- It supports multi-exchange routing via `exchange` (mainly `binance` and `okx`; quote sources also include Gate.io, Coinbase, Kraken, CoinGecko).
- Private exchange actions (`trade_preview`, `trade_submit`, order status/cancel/open orders/history, `positions`) require bound exchange credentials. The skill checks the target exchange binding before parameter validation or private API calls; if the current `user_key` has no bound API, it returns a clear “API not bound” error.
- **Symbol / pair**: If the asset or trading pair is ambiguous or could map to multiple symbols, ask one concise clarification before calling trade/order/quote-affecting actions; do not guess `symbol`.
- **Exchange default**: For exchange-scoped actions, use explicit `exchange` first. If omitted, use configured `crypto.execution_mode` / `crypto.default_exchange`. If neither is configured, ask one concise clarification instead of assuming `binance`.
- **Execution vs preview**: `trade_preview` is for preview-only user intent. `trade_submit` is only when the **current** user message explicitly requests immediate execution (same turn). There is **no** platform-level second-step pending-confirm chain in `clawd`; do not rely on a later yes/no follow-up flow.
- Supported order types: `market`, `limit`, `stop_loss_limit`, `take_profit_limit`, `limit_maker` (Binance); `market`, `limit` (OKX).

## Config Entry Points (from interface)
- No dedicated config entry points declared.

## Actions (from interface)
- Market/info: `quote` (aliases `price`, `get_price` when querying one symbol), `multi_quote` (aliases `get_multi_price`; `price` when `symbols` is present), `get_book_ticker` (alias `book_ticker`), `binance_symbol_check`, `normalize_symbol`, `healthcheck`, `candles` (aliases `kline`, `klines`, `candlestick`, `candlesticks`, `ohlcv`; these normalize to `indicator` when an `indicator` param is also present), `indicator` (aliases `technical_indicator`, `technical_indicators`, `ta_indicator`, `ta`), `price_alert_check`, `onchain`
- **Price-alert aliases** (normalize to `price_alert_check` internally, no separate actions): `price_monitor`, `monitor_price`, `price_alert`, `volatility_alert`.
- Trade/order: `trade_preview`, `trade_submit`, `order_status`, `cancel_order`, `cancel_all_orders` (alias `cancel_open_orders`), `open_orders` (alias `get_open_orders`, `pending_orders`), `trade_history` (alias `my_trades`, `recent_trades`), `positions`

## Parameter Contract (from interface)
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Action name from the list above. `price` is accepted as a compatibility alias and normalizes to `quote` or `multi_quote` based on whether `symbols` is present. |
| many actions | `exchange` | no | string | config default | Exchange routing: `binance`, `okx`. If omitted, use `crypto.execution_mode` / `crypto.default_exchange`; if neither is configured, clarify instead of guessing a hardcoded fallback. |
| many actions | `symbol` | depends | string | - | Trading pair symbol. Normalize to canonical form only when uniquely identifiable; if ambiguous, planner must clarify first—do not guess. |
| `quote` | `symbol` | yes | string | - | Single symbol quote; aggregates Binance/OKX/Gate/Coinbase/Kraken/CoinGecko. |
| `multi_quote` | `symbols` or `symbol` | yes | array/string | - | Multi-symbol batch quote; max 20 symbols. |
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
| `onchain` | `chain` | no | string | `bitcoin` | `bitcoin`/`btc` or `ethereum`/`eth`. |
| `onchain` (eth address mode) | `address` | no | string | - | If provided, returns address balance + recent txs. |
| `onchain` (eth address mode) | `token` | no | string | `eth` | Native or configured ERC20 token symbol. |
| `onchain` (eth address mode) | `tx_limit`/`limit` | no | number | `5` | Recent tx count. |
| `trade_preview`/`trade_submit` | `symbol` | yes | string | - | Order symbol. |
| `trade_preview`/`trade_submit` | `side` | no* | string | `buy` | `buy` or `sell`. |
| `trade_preview`/`trade_submit` | `order_type` | no | string | `market` | `market`, `limit`, `stop_loss_limit`, `take_profit_limit`, `limit_maker`. Aliases: `type`, `orderType`. |
| `trade_preview`/`trade_submit` | `qty` | cond | number/string | - | Base asset quantity. Use `"all"` for full-position sell (SELL side only). Aliases: `quantity`, `amount`, `base_qty`, `base_quantity`. `amount` means base-asset amount; use `quote_qty_usd`/`amount_usd` for quote-currency notional. |
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

## Error Contract (from interface)
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

## Request/Response Examples (from interface)
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

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
- For setup or configuration questions about this skill, treat the config entry points section as the grounding source for where changes actually live.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Interpret Chinese colloquial phrasing by capability semantics and requested task shape, not by a fixed phrase list.
- Judge Chinese delivery intent semantically: if the user asks to receive a file/result rather than inline body text, plan toward delivery without depending on fixed wording.
- Preserve Chinese brevity and format constraints as final output contracts when the skill can support them; do not convert those constraints into token-level matching rules.
- Treat Chinese style constraints as audience/tone constraints for the eventual explanation, not as skill-selection shortcuts.
- Resolve Chinese deictic references only from immediate, concrete, type-compatible context; do not guess unsupported targets or invent missing args just to force a skill call.
