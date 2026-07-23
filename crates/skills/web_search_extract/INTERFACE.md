# web_search_extract Interface Spec

## Capability Summary

`web_search_extract` is a lightweight web search entry skill.

It is search-only:
- returns normalized search result items
- does not perform browser rendering or page content extraction
- can provide URL list for downstream `browser_web` extraction
- successful responses mirror the structured search payload into `extra.items`, `extra.candidates`, and `extra.field_value` so runtime evidence checks do not parse the JSON string in `text`
- returns bounded cursor pages, source refs, citations, a candidate snapshot hash, and observation provenance
- marks all titles/snippets as untrusted search metadata that cannot act as planner instructions
- accepts only HTTP(S) candidate URLs and removes userinfo, fragments, tracking parameters, local/private literal targets, and local-only hostnames
- reads each fixed search backend under a 2 MiB response ceiling and only follows bounded HTTPS redirects to that backend's exact host
- uses zero-key fallback sources when a configured HTML search backend is empty or blocked; `site:<domain>` operators are projected into structured domain filtering

## Actions

- `search`
- `search_extract`

`search_extract` in this skill still means "search + return extract-ready URL list"; actual extraction belongs to `browser_web`.

## Parameter Contract

- `action` (required, string): `search|search_extract`
- `query` (required, string)
- `top_k` (optional, integer, default `5`, range `1..20`)
- `cursor` (optional, integer, default `0`, range `0..100`)
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
- Skill protocol errors use outer `status=error` with matching `extra.error_code`; they are not wrapped as successful observations.
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
  "text": "{\"status\":\"ok\",\"backend\":\"duckduckgo_html\",\"items\":[{\"title\":\"Rust Async\",\"url\":\"https://example.com\"}],\"extract_urls\":[\"https://example.com\"],\"summary\":\"search_result_set\",\"result_count\":1,\"citations\":[\"https://example.com\"]}",
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
      "summary": "search_result_set"
    },
    "items": [{"title":"Rust Async","url":"https://example.com","rank":1,"source":"example.com","snippet":null}],
    "candidates": [{"title":"Rust Async","url":"https://example.com","rank":1,"source":"example.com","snippet":null}],
    "extract_urls": ["https://example.com"],
    "citations": ["https://example.com"],
    "page": {"cursor":0,"limit":3,"returned_count":1,"total_count":null,"has_more":false,"stability":"backend_best_effort"},
    "snapshot_id": "sha256:...",
    "trust": {"classification":"untrusted_search_metadata","instructions_executable":false}
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
- `summary`: stable machine token `search_result_set`; use `result_count` and `page` for counts
- `citations[]`: same as result URLs
- `extra.items` / `extra.candidates`: same normalized result array, present even when empty
- `extra.field_value.result_count`: stable result count for evidence checks
- `extra.source_refs[]`: candidate URL/title/rank/source objects for citation-aware synthesis
- `extra.page`: bounded cursor metadata. Search engines are mutable, so page stability is explicitly `backend_best_effort`.
- `extra.snapshot_id`: SHA-256 over query, backend, and observed candidate identity
- `extra.trust`: candidate metadata is untrusted and never executable

- Dedup by normalized URL.
- URL normalization removes fragments and common tracking params (`utm_*`, `gclid`, `fbclid`).
- Apply domain allow/deny filtering after normalization.
- If backend is not configured, use the zero-key `duckduckgo_html` backend. Still return explicit error if the selected backend fails.
- When HTML search returns no candidates, fallback sources may be used:
  - `docs_rs_search` when `domains_allow` includes `docs.rs` or the query uses `site:docs.rs`
  - `github_repositories` when no domain filter is set or `github.com` is allowed
- Keep search responsibility separate from `browser_web`.
