# rss_fetch Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the rss_fetch implementation.

## Capability Summary
- `rss_fetch` reads RSS/Atom feeds and returns normalized news items.
- It supports direct feed fetching and category-based latest/news retrieval.

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
| `latest` | `category` | no | string | impl default | Category filter (often `crypto` for crypto news). |
| `latest` | `limit` | no | number | impl default | Maximum returned items. |
| `latest` | `source_layer` | no | string | impl default | Feed source profile. |
| `latest` | `timeout_seconds` | no | number | impl default | Request timeout override. |
| `news` | `category` | no | string | `general` | News category. |
| `news` | `limit` | no | number | impl default | Maximum returned items. |
| `news` | `source_layer` | no | string | impl default | Feed source profile. |
| `news` | `timeout_seconds` | no | number | impl default | Request timeout override. |

## Error Contract
- `action` missing/unsupported.
- `fetch` without a valid feed selector.
- Empty/invalid URL values.
- HTTP/parse failures return readable upstream error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"latest","category":"crypto","limit":5}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"1) ...\n2) ...","error_text":null}
```
