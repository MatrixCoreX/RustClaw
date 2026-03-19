# rss_fetch Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the rss_fetch implementation.

## Capability Summary
- `rss_fetch` reads RSS/Atom feeds and returns normalized news items.
- It supports direct feed fetching and category-based latest/news retrieval.
- **Category semantics**: A category uses a single list of sources; all listed sources are fetched by default (no primary/fallback tiers). Single-source failure is skipped; only when all sources fail (or return no items) does the skill return an error.
- **Deprecated sources**: Default fetch uses only active sources. Sources that consecutively fail (e.g. `deprecate_after_failures = 3` in config) are moved into `[rss.deprecated]` and no longer fetched; success on a source resets its failure count.

## Actions
- `fetch`
- `latest`
- `news`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of `fetch|latest|news`. |
| `fetch` | `url` or `feed_url` or `feed_urls` | yes | string/array | - | Feed selector (at least one). |
| `fetch` | `timeout_seconds` | no | number | impl default | Request timeout override. |
| `latest` | `category` | no | string | impl default | Category name; all sources for this category are fetched. |
| `latest` | `limit` | no | number | impl default | Maximum returned items (applied after merge/dedupe/sort). |
| `latest` | `timeout_seconds` | no | number | impl default | Request timeout override. |
| `news` | `category` | no | string | `general` | Category name; all sources for this category are fetched. |
| `news` | `limit` | no | number | impl default | Maximum returned items (applied after merge/dedupe/sort). |
| `news` | `timeout_seconds` | no | number | impl default | Request timeout override. |

## Config (configs/rss.toml)
- `rss.deprecate_after_failures`: number of consecutive failures before a source is moved to deprecated (default 3).
- `rss.deprecated.sources`: list of deprecated entries (url, category, reason, failure_count, last_error, deprecated_at). Only active sources are fetched; deprecated entries are kept for reference and can be restored manually later.

## Error Contract
- `action` missing/unsupported.
- `fetch` without a valid feed selector.
- Empty/invalid URL values.
- For `latest`/`news`: only when **all** configured sources for the category fail or return no items does the skill return an error. Partial success returns the successfully fetched items plus a summary (e.g. sources_ok / sources_failed / items).

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"latest","category":"crypto","limit":5}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"sources_ok=3 sources_failed=0 items=5\n1) ...\n2) ...","error_text":null}
```
