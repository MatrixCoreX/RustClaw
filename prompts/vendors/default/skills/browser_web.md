<!-- AUTO-GENERATED: sync_skill_docs.py -->
## Role & Boundaries
- You are the `browser_web` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `crates/skills/browser_web/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
`browser_web` is a browser-layer extraction skill built on Node.js + Playwright.
It supports:
- open URL(s) and extract readable page content (`open_extract`)
- browser-based Google result extraction (`search_page`)
- search first, then extract top result pages (`search_extract`)

This skill is read-only. It does not submit forms, log in, or execute business actions on web pages.

Runtime behavior notes:
- Browser launch prefers system Chromium (`/usr/bin/chromium`, `/usr/bin/chromium-browser`) when available.
- If system Chromium is unavailable, fallback is Playwright-managed Chromium.
- Browser runs in forced headless mode and clears `DISPLAY/WAYLAND_DISPLAY/XAUTHORITY` in child runtime to avoid X11 popup requests.
- `open_extract` writes capture artifacts by default under `skills_output/browser_web/...` with raw HTML, cleaned text, images, and JSONL indexes.

## Actions (from interface)
### 1. `open_extract`

Open one or more URLs in a browser, apply readability checks, and return extracted content.

**Parameters:**
- `action` (required, string): `"open_extract"`
- `url` (optional, string): single URL
- `urls` (optional, string[]): multiple URLs
- `max_pages` (optional, integer, default `3`, range `1..10`)
- `wait_until` (optional, string, default `domcontentloaded`): `domcontentloaded|load|networkidle`
- `save_screenshot` (optional, bool, default `false`)
- `screenshot_dir` (optional, string, default `skills_output/browser_web/screenshots`)
- `content_mode` (optional, string, default `clean`): `clean|raw`
- `max_text_chars` (optional, integer, default `12000`, range `100..200000`)
- `min_content_chars` (optional, integer, default `200`, range `20..10000`): readability threshold for extracted text
- `fail_fast` (optional, bool, default `false`): if true, stop on first page failure
- `wait_map_path` (optional, string): optional wait strategy mapping file
- `enable_capture` (optional, bool, default `true`): whether to persist capture artifacts
- `capture_root` (optional, string, default `skills_output`): root directory for capture runs
- `capture_source` (optional, string, default `browser_web`): source namespace folder name
- `run_id` (optional, string): custom run id for output folder
- `chunk_chars` (optional, integer, default `3000`): chunk size for `index/chunks.jsonl`
- `capture_images` (optional, bool, default `true`): download page images to `assets/images` for archive
- `max_capture_images` (optional, integer, default `12`): max image files downloaded per page

At least one of `url` or `urls` is required.

**Output (per successful item):**
- `url`
- `final_url`
- `title`
- `text`
- `content_excerpt`
- `source`
- `published_at`
- `fetch_method` (`browser`)
- `extracted_at` (ISO timestamp)
- `nav_wait_until`
- `nav_attempts`
- `nav_attempt_trace[]`
- `latency_ms`
- `extraction_trace`
- `screenshot_path`
- `capture_artifacts` (absolute paths to html/text/image + manifest/chunks files for this item)

**Output (per failed item):**
- `fetch_method` (`unavailable`)
- `error_code`
- `error`
- `diagnostics` (optional structured details)

**Top-level output:**
- `capture.run_root`
- `capture.raw_html_dir`
- `capture.processed_text_dir`
- `capture.images_dir`
- `capture.manifest_path`
- `capture.chunks_path`

### 2. `search_page`

Use browser-rendered Google search result page extraction with selector fallback.

**Parameters:**
- `action` (required, string): `"search_page"`
- `query` (required, string)
- `engine` (optional, string, default `google`) (current implementation supports only `google`)
- `top_k` (optional, integer, default `5`, range `1..20`)
- `region` (optional, string): forwarded to Google `gl`
- `lang` (optional, string, default `en`): forwarded to Google `hl`

**Output:**
- `items[]` with `title/url/snippet/source`
- `summary`
- `citations[]`
- `nav_wait_until`
- `nav_attempts`
- `nav_attempt_trace[]`
- `diagnostics` (selector strategy hit/miss diagnostics)

### 3. `search_extract`

Search first (`search_page`), then extract top result pages (`open_extract`).

**Parameters:**
- `action` (required, string): `"search_extract"`
- `query` (required, string)
- `engine` (optional, string, default `google`)
- `top_k` (optional, integer, default `5`, range `1..20`)
- `extract_top_n` (optional, integer, default `3`, range `1..10`)
- `wait_until` (optional, string, default `domcontentloaded`)
- `summarize` (optional, bool, default `true`): include short summary per extracted item
- `content_mode` (optional, string, default `clean`): `clean|raw`
- `max_text_chars` (optional, integer, default `12000`, range `100..200000`)
- `min_content_chars` (optional, integer, default `200`, range `20..10000`)
- `fail_fast` (optional, bool, default `false`)
- `region` (optional, string)
- `lang` (optional, string, default `en`)

**Output:**
- same `items[]` shape as `open_extract`
- `summary`
- `citations[]`
- `search_diagnostics` (selector drift diagnostics from search step)

## Parameter Contract (from interface)
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| open_extract | action | yes | string | - | `open_extract` |
| open_extract | url / urls | yes (one) | string / string[] | - | URL targets to open |
| open_extract | max_pages | no | integer | 3 | max pages from input list |
| open_extract | wait_until | no | string | domcontentloaded | navigation wait strategy |
| open_extract | save_screenshot | no | bool | false | capture screenshots only when explicitly needed |
| open_extract | capture_images | no | bool | true | archive page images to assets/images |
| open_extract | max_capture_images | no | integer | 12 | image download cap per page |
| open_extract | content_mode | no | string | clean | clean or raw text mode |
| open_extract | max_text_chars | no | integer | 12000 | max returned text chars |
| open_extract | min_content_chars | no | integer | 200 | readability threshold for extracted text |
| open_extract | fail_fast | no | bool | false | stop on first page failure |
| open_extract | wait_map_path | no | string | null | domain-based wait strategy map |
| search_page | action | yes | string | - | `search_page` |
| search_page | query | yes | string | - | search query |
| search_page | top_k | no | integer | 5 | max results |
| search_page | region | no | string | null | Google `gl` region |
| search_page | lang | no | string | en | Google `hl` language |
| search_extract | action | yes | string | - | `search_extract` |
| search_extract | query | yes | string | - | search query |
| search_extract | extract_top_n | no | integer | 3 | number of results to extract |
| search_extract | summarize | no | bool | true | include short summary field |
| search_extract | content_mode | no | string | clean | clean or raw mode |
| search_extract | max_text_chars | no | integer | 12000 | max returned text chars |
| search_extract | min_content_chars | no | integer | 200 | readability threshold for extracted text |

## Error Contract (from interface)
Standard error codes used in helper-level failures or per-item failures:
- `NAV_TIMEOUT`: navigation retries exhausted
- `BOT_BLOCKED`: anti-bot / challenge / blocked page detected
- `SELECTOR_MISS`: page/search selector extraction failed
- `EMPTY_CONTENT`: readability threshold not met or invalid input
- `DEPENDENCY_MISSING`: Node.js / Playwright / Chromium dependency missing
- `DEPENDENCY_MISSING`: also used when runtime sandbox restrictions block Chromium launch (for example `NoNewPrivs=1` + `Seccomp=2`)

Rules:
- No silent failure.
- `open_extract` default behavior is partial success with per-item `error_code` + `error`.
- `fail_fast=true` makes `open_extract/search_extract` fail immediately on first page failure.

## Request/Response Examples (from interface)
### Example 1: open_extract (clean)

Request:
```json
{
  "request_id": "bw-open-1",
  "args": {
    "action": "open_extract",
    "urls": ["https://github.com/microsoft/playwright"],
    "content_mode": "clean",
    "max_text_chars": 8000,
    "wait_until": "load"
  }
}
```

Response (simplified):
```json
{
  "request_id": "bw-open-1",
  "status": "ok",
  "text": "{\"items\":[{\"url\":\"https://github.com/microsoft/playwright\",\"final_url\":\"https://github.com/microsoft/playwright\",\"title\":\"microsoft/playwright\",\"source\":\"github.com\",\"fetch_method\":\"browser\",\"nav_wait_until\":\"load\",\"nav_attempts\":1,\"latency_ms\":1784,\"extracted_at\":\"2026-03-21T10:12:33Z\"}],\"summary\":\"Extracted 1 page(s) using browser\",\"citations\":[\"https://github.com/microsoft/playwright\"]}",
  "error_text": null
}
```

### Example 2: search_page with region/lang

Request:
```json
{
  "request_id": "bw-search-1",
  "args": {
    "action": "search_page",
    "query": "Playwright browser automation",
    "region": "us",
    "lang": "en",
    "top_k": 5
  }
}
```

Response (simplified):
```json
{
  "request_id": "bw-search-1",
  "status": "ok",
  "text": "{\"items\":[{\"title\":\"Playwright\",\"url\":\"https://playwright.dev/\",\"snippet\":\"...\",\"source\":\"google\"}],\"summary\":\"Found 5 search result(s) for \\\"Playwright browser automation\\\"\",\"diagnostics\":{\"fallback_used\":false}}",
  "error_text": null
}
```

### Example 3: open_extract partial failure

Request:
```json
{
  "request_id": "bw-open-2",
  "args": {
    "action": "open_extract",
    "urls": ["https://example.com", "https://blocked.example"],
    "fail_fast": false
  }
}
```

Response (simplified):
```json
{
  "request_id": "bw-open-2",
  "status": "ok",
  "text": "{\"items\":[{\"fetch_method\":\"browser\"},{\"fetch_method\":\"unavailable\",\"error_code\":\"BOT_BLOCKED\",\"error\":\"page appears blocked by anti-bot challenge\"}],\"summary\":\"Extracted 1 page(s); 1 page(s) failed browser extraction\"}",
  "error_text": null
}
```

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
