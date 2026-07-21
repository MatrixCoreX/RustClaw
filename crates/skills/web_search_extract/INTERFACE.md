# web_search_extract Interface Spec

## Capability Summary

`web_search_extract` is a lightweight web search entry skill.

It is search-only:
- returns normalized search result items
- does not perform browser rendering or page content extraction
- can provide URL list for downstream `browser_web` extraction
- successful responses mirror the structured search payload into `extra.items`, `extra.candidates`, and `extra.field_value` so runtime evidence checks do not parse the JSON string in `text`
- uses zero-key fallback sources when a configured HTML search backend is empty or blocked; `site:<domain>` operators are projected into structured domain filtering

## Actions

- `search`
- `search_extract`

`search_extract` in this skill still means "search + return extract-ready URL list"; actual extraction belongs to `browser_web`.

## Parameter Contract

- `action` (required, string): `search|search_extract`
- `query` (required, string)
- `top_k` (optional, integer, default `5`, range `1..20`)
- `lang` (optional, string)
- `time_range` (optional, string): backend-dependent passthrough
- `domains_allow` (optional, string[])
- `domains_deny` (optional, string[])
- `backend` (optional, string): `serpapi|bing_html|duckduckgo_html`; defaults to `duckduckgo_html` when no API-backed backend is configured.
- `include_snippet` (optional, bool, default `true`)

## Error Contract

- `INVALID_INPUT`: required fields like `query` are missing or malformed.
- `INVALID_ACTION`: `action` is not one of `search` or `search_extract`.
- `SEARCH_FAILED`: configured backend failed or no fallback backend can complete the request.
- Never return fake empty success when backend configuration is missing.

## Request/Response Examples

### Example 1

Request:
```json
{
  "request_id": "web-1",
  "args": {
    "action": "search_extract",
    "query": "rust async tutorial",
    "top_k": 3,
    "include_snippet": true
  }
}
```

Response:
```json
{
  "request_id": "web-1",
  "status": "ok",
  "text": "{\"status\":\"ok\",\"backend\":\"duckduckgo_html\",\"items\":[{\"title\":\"Rust Async\",\"url\":\"https://example.com\"}],\"extract_urls\":[\"https://example.com\"],\"summary\":\"1 result\",\"citations\":[\"https://example.com\"],\"notes\":\"search only\"}",
  "extra": {
    "schema_version": 1,
    "action": "search_extract",
    "query": "rust async tutorial",
    "top_k": 3,
    "backend": "duckduckgo_html",
    "backend_connected": true,
    "status": "ok",
    "field_value": {
      "status": "ok",
      "result_count": 1,
      "summary": "1 result"
    },
    "items": [{"title":"Rust Async","url":"https://example.com","rank":1,"source":"example.com","snippet":null}],
    "candidates": [{"title":"Rust Async","url":"https://example.com","rank":1,"source":"example.com","snippet":null}],
    "extract_urls": ["https://example.com"],
    "citations": ["https://example.com"]
  },
  "error_text": null
}
```

Returned JSON inside `text` contains:

- `status`: `ok|error`
- `error_code`: nullable (`INVALID_INPUT|INVALID_ACTION|SEARCH_FAILED`)
- `error`: nullable string
- `backend`: backend name or null
- `items[]`:
  - `title`
  - `url` (normalized)
  - `snippet` (nullable by `include_snippet`)
  - `source` (standardized host)
  - `rank`
- `extract_urls[]`: URL list ready for `browser.open_extract`
- `summary`: lightweight result summary (based on result metadata only)
- `citations[]`: same as result URLs
- `notes`: boundary hint
- `extra.items` / `extra.candidates`: same normalized result array, present even when empty
- `extra.field_value.result_count`: stable result count for evidence checks

- Dedup by normalized URL.
- URL normalization removes fragments and common tracking params (`utm_*`, `gclid`, `fbclid`).
- Apply domain allow/deny filtering after normalization.
- If backend is not configured, use the zero-key `duckduckgo_html` backend. Still return explicit error if the selected backend fails.
- When HTML search returns no candidates, fallback sources may be used:
  - `docs_rs_search` when `domains_allow` includes `docs.rs` or the query uses `site:docs.rs`
  - `github_repositories` when no domain filter is set or `github.com` is allowed
- Keep search responsibility separate from `browser_web`.
