# rss_fetch Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the rss_fetch implementation.

## Capability Summary
- `rss_fetch` reads RSS/Atom feeds and returns normalized news items.
- **Category guardrails (planner / agent)**: For `latest` and `news`, `category` must be a **table key** under `[rss.categories]` in `configs/rss.toml` at runtime (deployment-specific). **Do not invent** category strings. If user intent does not map clearly to one configured category, use **`general`** (or `rss.default_category` when set). Open `configs/rss.toml` for the authoritative list of keys; example keys in the default config include `general`, `crypto`, `tech`, `web3`, `aggregator`, `china`, `business`, `international`—the file always wins if this list drifts.
- **`fetch`** is **direct-feed only**: one or more explicit `http(s)` URLs. It does **not** fall back to category/config sources.
- **`latest`** and **`news`** use **category mode**: all **active** sources for the category (from config) are fetched by default. Same merge/dedupe/sort behavior; `news` is an alias of `latest` (default category for `news` when omitted follows config / `general` as documented below).
- **Category semantics**: A category uses a single list of sources; all listed sources are fetched by default (no primary/fallback tiers). Single-source failure is skipped; only when all sources fail (or return no items) does the skill return an error.
- **Deprecated sources**: Default fetch uses only active sources. Sources that consecutively fail (e.g. `deprecate_after_failures = 3` in config) are moved into `[rss.deprecated]` and no longer fetched; success on a source resets its failure count. Deprecated sources are not used for `latest`/`news` unless restored in config.

## Actions
- `fetch` — direct RSS/Atom URL(s) only; requires `url` or `feed_url` or `feed_urls`.
- `latest` — category-based; uses configured sources for `category` (or default category).
- `news` — same pipeline as `latest` (alias); default `category` when omitted is typically `general` per config.

### Backward-compatible action aliases (skill-internal only)
The schedule / `run_skill` persistence layer does **not** rewrite these; normalization happens inside this skill before dispatch.

| Alias | Normalized behavior |
|---|---|
| `fetch_crypto_news` | `action=latest`; if `category` omitted, set `category=crypto`. |
| `fetch_tech_news` | `action=latest`; if `category` omitted, set `category=tech`. |
| `fetch_news` | `action=latest` (category from args or defaults). |
| `fetch_feed` | If `url` / `feed_url` / non-empty `feed_urls` present → `action=fetch` (direct feed). If no URL selector → **error** (do not fall back to category/latest). |

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | no* | string | `latest` | One of `fetch`, `latest`, or `news`. If omitted, behavior is **`latest`** (category mode), not `fetch`. |
| `fetch` | `url` or `feed_url` or `feed_urls` | yes | string/array | - | **At least one** http(s) feed URL. `feed_urls`: JSON array of strings; empty or all-invalid → error. |
| `fetch` | `limit` | no | number | impl default | Per-feed item cap (single URL). |
| `fetch` | `timeout_seconds` | no | number | impl default | Request timeout override. |
| `latest` | `category` | no | string | impl default | Must be a key under `[rss.categories]` in `configs/rss.toml`; all **active** sources for that category are fetched. If unmappable, prefer `general` / `rss.default_category`. Do not invent categories. |
| `latest` | `limit` | no | number | impl default | Maximum returned items (applied after merge/dedupe/sort). |
| `latest` | `timeout_seconds` | no | number | impl default | Request timeout override. |
| `latest` | `url` / `feed_url` / `feed_urls` | no | string/array | - | Optional: if provided, fetches **only** these URLs (explicit list) instead of category config; still uses `latest` merge/deprecation rules for **non-explicit** category fetches only — when using explicit URLs, deprecation state is not updated. |
| `news` | `category` | no | string | `general` | Same as `latest` (category-based); same `[rss.categories]` key rule and no invented category strings. |
| `news` | `limit` | no | number | impl default | Same as `latest`. |
| `news` | `timeout_seconds` | no | number | impl default | Same as `latest`. |

## Config (configs/rss.toml)
- `[rss.categories.<name>]`: each **`<name>`** is a valid `category` value for `latest` / `news`. Planner-visible contract: only these keys (plus skill-internal alias defaults like `fetch_crypto_news` → `crypto`) are valid; never pass a string that is not a configured category key unless you accept a skill error.
- `rss.default_category` / `rss.default_limit` / `rss.timeout_seconds`: defaults when args omit them.
- `rss.deprecate_after_failures`: number of consecutive failures before a source is moved to deprecated (default 3).
- `rss.deprecated.sources`: list of deprecated entries (url, category, reason, failure_count, last_error, deprecated_at). Only active sources are fetched; deprecated entries are kept for reference and can be restored manually later.

## Error Contract
- Unknown or unconfigured `category` (no entry under `[rss.categories]` or no active sources) → readable `error_text` (e.g. `no configured feeds for category=...`).
- `action` unsupported (after alias normalization).
- **`fetch_feed`** without a direct URL selector → error; use `latest`/`news` for category feeds.
- **`fetch`** without `url`/`feed_url`/non-empty valid `feed_urls`, or with non-http(s) URLs → clear `error_text` (e.g. `fetch requires url, feed_url, or feed_urls`).
- Empty/invalid URL values for `fetch`.
- For `latest`/`news`: only when **all** configured sources for the category fail or return no items does the skill return an error. Partial success returns the successfully fetched items plus a summary (e.g. sources_ok / sources_failed / items).

## Request/Response Examples
### Example 1 (category latest)
Request:
```json
{"request_id":"demo-1","args":{"action":"latest","category":"crypto","limit":5}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"sources_ok=3 sources_failed=0 items=5\n1) ...\n2) ...","error_text":null}
```

### Example 2 (direct fetch)
Request:
```json
{"request_id":"demo-2","args":{"action":"fetch","url":"https://example.com/feed.xml","limit":10}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"...","error_text":null}
```
