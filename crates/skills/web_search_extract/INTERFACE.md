# web_search_extract Interface Spec

## Capability Summary

`web_search_extract` is a lightweight web search entry skill.

It is search-only:
- returns normalized search result items
- does not perform browser rendering or page content extraction
- can provide URL list for downstream `browser_web` extraction

## Actions

- `search`
- `search_extract`

`search_extract` in this skill still means "search + return extract-ready URL list"; actual extraction belongs to `browser_web`.

## Input Parameters

- `action` (required, string): `search|search_extract`
- `query` (required, string)
- `top_k` (optional, integer, default `5`, range `1..20`)
- `lang` (optional, string)
- `time_range` (optional, string): backend-dependent passthrough
- `domains_allow` (optional, string[])
- `domains_deny` (optional, string[])
- `backend` (optional, string): `serpapi|duckduckgo_html`
- `include_snippet` (optional, bool, default `true`)

## Output Schema

The skill returns JSON in `text` with:

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
- `extract_urls[]`: URL list ready for `browser_web.open_extract`
- `summary`: lightweight result summary (based on result metadata only)
- `citations[]`: same as result URLs
- `notes`: boundary hint

## Behavior Notes

- Dedup by normalized URL.
- URL normalization removes fragments and common tracking params (`utm_*`, `gclid`, `fbclid`).
- Apply domain allow/deny filtering after normalization.
- If backend is not configured, return explicit error (do not fake empty success).
- Keep search responsibility separate from `browser_web`.

