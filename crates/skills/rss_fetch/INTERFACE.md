# rss_fetch Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the rss_fetch implementation.

## Capability Summary
- `rss_fetch` reads RSS/Atom feeds and returns normalized news items.
- Successful responses include user-visible `text` plus machine-readable `extra` evidence. Runtime quality checks consume `extra.field_value` / `extra.items`; do not require downstream code to parse localized `text`.
- **Category guardrails (planner / agent)**: For `latest` and `news`, `category` must be a **table key** under `[rss.categories]` in `configs/rss.toml` at runtime (deployment-specific). **Do not invent** category strings. If user intent does not map clearly to one configured category, use **`general`** (or `rss.default_category` when set). Open `configs/rss.toml` for the authoritative list of keys; example keys in the default config include `general`, `crypto`, `tech`, `web3`, `aggregator`, `china`, `business`, `international`—the file always wins if this list drifts.
- **`fetch`** is **direct-feed only**: one or more explicit `http(s)` URLs. It does **not** fall back to category/config sources.
- **`latest`** and **`news`** use **category mode**: all **active** sources for the category (from config) are fetched by default. Same merge/dedupe/sort behavior; `news` is an alias of `latest` (default category for `news` when omitted follows config / `general` as documented below).
- **Category semantics**: A category uses a single list of sources; all listed sources are fetched by default (no primary/fallback tiers). Single-source failure is skipped; only when all sources fail (or return no items) does the skill return an error.
- **Topic semantics**: `extra.items[].topic` is a stable machine token from `args.topic` / `args.topic_token` or `[rss.categories.<name>].topic`. The skill must not classify titles with language keyword lists; if no machine topic is configured, use `other`.
- **Deprecated sources**: Default fetch uses only active sources. Sources that consecutively fail (e.g. `deprecate_after_failures = 3` in config) are moved into `[rss.deprecated]` and no longer fetched; success on a source resets its failure count. Deprecated sources are not used for `latest`/`news` unless restored in config.

## Config Entry Points
- Main RSS config: `configs/rss.toml`.
- Category source lists: `configs/rss.toml` -> `[rss.categories.<name>]`.
- Defaults: `rss.default_category`, `rss.default_limit`, and `rss.timeout_seconds`.
- Optional category topic token: `[rss.categories.<name>].topic`, a lowercase machine token such as `macro_market`, `tech_ecosystem`, or `other`.

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
| `fetch` | `topic` / `topic_token` | no | string | `other` | Stable lowercase machine topic token for `extra.items[].topic`; do not pass user-language phrases. |
| `latest` | `category` | no | string | impl default | Must be a key under `[rss.categories]` in `configs/rss.toml`; all **active** sources for that category are fetched. If unmappable, prefer `general` / `rss.default_category`. Do not invent categories. |
| `latest` | `limit` | no | number | impl default | Maximum returned items (applied after merge/dedupe/sort). |
| `latest` | `timeout_seconds` | no | number | impl default | Request timeout override. |
| `latest` | `topic` / `topic_token` | no | string | category config / `other` | Stable lowercase machine topic override for `extra.items[].topic`; invalid sentence-like values are ignored. |
| `latest` | `url` / `feed_url` / `feed_urls` | no | string/array | - | Optional: if provided, fetches **only** these URLs (explicit list) instead of category config; still uses `latest` merge/deprecation rules for **non-explicit** category fetches only — when using explicit URLs, deprecation state is not updated. |
| `news` | `category` | no | string | `general` | Same as `latest` (category-based); same `[rss.categories]` key rule and no invented category strings. |
| `news` | `limit` | no | number | impl default | Same as `latest`. |
| `news` | `timeout_seconds` | no | number | impl default | Same as `latest`. |
| `news` | `topic` / `topic_token` | no | string | category config / `other` | Same topic-token rule as `latest`. |

## Config (configs/rss.toml)
- `[rss.categories.<name>]`: each **`<name>`** is a valid `category` value for `latest` / `news`. Planner-visible contract: only these keys (plus skill-internal alias defaults like `fetch_crypto_news` → `crypto`) are valid; never pass a string that is not a configured category key unless you accept a skill error.
- `[rss.categories.<name>].topic`: optional stable topic token used for grouping and `extra.items[].topic`; this replaces title-keyword classification and keeps topic behavior language-neutral.
- `rss.default_category` / `rss.default_limit` / `rss.timeout_seconds`: defaults when args omit them.
- `rss.deprecate_after_failures`: number of consecutive failures before a source is moved to deprecated (default 3).
- `rss.deprecated.sources`: list of deprecated entries (url, category, reason, failure_count, last_error, deprecated_at). Only active sources are fetched; deprecated entries are kept for reference and can be restored manually later.

## Error Contract
- Unknown or unconfigured `category` (no entry under `[rss.categories]` or no active sources) returns readable `error_text` plus machine fields in `extra`: `error_kind=category_not_configured`, `failure_phase=pre_dispatch`, `side_effect_applied=false`, `recovery_action=replan_arguments`, `invalid_argument=category`, `rejected_value`, `default_category`, and sorted `available_categories`. Runtime recovery must consume these fields, not parse `error_text`.
- `action` unsupported (after alias normalization).
- **`fetch_feed`** without a direct URL selector → error; use `latest`/`news` for category feeds.
- **`fetch`** without `url`/`feed_url`/non-empty valid `feed_urls`, or with non-http(s) URLs → clear `error_text` (e.g. `fetch requires url, feed_url, or feed_urls`).
- Empty/invalid URL values for `fetch`.
- For `latest`/`news`: only when **all** configured sources for the category fail or return no items does the skill return an error. Partial success returns the successfully fetched items plus a summary (e.g. sources_ok / sources_failed / items).

## Success Response Extra
- `extra.schema_version`: number, currently `1`.
- `extra.action`: canonical action (`fetch`, `latest`, or `news` alias normalized to `latest` internally).
- `extra.mode`: `direct`, `category`, or `explicit_urls`.
- `extra.field_value`: object containing stable execution counters such as `sources_ok`, `sources_failed`, and `items` / `item_count`, plus a compact `titles` array for grounding brief headline answers before evidence truncation.
- `extra.items`: array of normalized feed item objects with `title`, `link`, `date`, `source`, `source_host`, `layer`, and `topic`.
- Generic evidence extractors treat `extra.items` as candidate/list evidence; do not duplicate the same item array under another key.
- `text` remains the localized, user-visible news listing. Consumers must use `extra` for machine evidence instead of parsing `text`.

## Request/Response Examples
### Example 1 (category latest)
Request:
```json
{"request_id":"demo-1","args":{"action":"latest","category":"crypto","limit":5}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"sources_ok=3 sources_failed=0 items=5\n1) ...\n2) ...","extra":{"schema_version":1,"action":"latest","category":"crypto","mode":"category","sources_ok":3,"sources_failed":0,"item_count":5,"field_value":{"sources_ok":3,"sources_failed":0,"items":5,"titles":["..."]},"items":[{"title":"...","link":"https://example.com/news","source_host":"example.com","layer":"feed","topic":"macro_market"}]},"error_text":null}
```

### Example 2 (direct fetch)
Request:
```json
{"request_id":"demo-2","args":{"action":"fetch","url":"https://example.com/feed.xml","limit":10}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"...","extra":{"schema_version":1,"action":"fetch","mode":"direct","source_count":1,"item_count":10,"field_value":{"source_count":1,"item_count":10,"titles":["..."]},"items":[{"title":"...","link":"https://example.com/item","source_host":"example.com","layer":"feed","topic":"other"}]},"error_text":null}
```
