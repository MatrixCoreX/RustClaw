# browser_web Skill Interface

## Capability Summary

`browser_web` is a browser-layer utility skill for live web reading. It uses Node.js + Playwright + headless Chromium to:
- open one or more URLs and extract visible page text
- perform Google result-page search in a real browser session
- search first, then open top results and extract page content

This skill is for reading and extraction only. It does NOT submit forms, log into websites, click dangerous actions, or fabricate search/page content.

## Actions

### 1. `open_extract`

Open one or more URLs in a headless browser and extract visible page text.

**Parameters:**
- `action` (required, string): must be `"open_extract"`
- `url` (optional, string): single URL to extract
- `urls` (optional, string[]): list of URLs to extract
- `max_pages` (optional, integer, default `3`): must be between `1` and `10`
- `wait_until` (optional, string, default `domcontentloaded`): one of `domcontentloaded`, `load`, `networkidle`

At least one of `url` or `urls` is required.

**Output:**
- `items[]`
  - `url`
  - `final_url`
  - `title`
  - `text`
  - `content_excerpt`
  - `source` (page host/domain)
  - `published_at` (`null` when not available)
  - `fetch_method` (`browser` on success, `unavailable` on failure)
- `summary`
- `citations[]`

### 2. `search_page`

Use a real browser to open a search engine result page and extract result links/snippets.

**Parameters:**
- `action` (required, string): must be `"search_page"`
- `query` (required, string): non-empty search query
- `engine` (optional, string, default `google`): MVP currently supports only `google`
- `top_k` (optional, integer, default `5`): must be between `1` and `20`

**Output:**
- `items[]`
  - `title`
  - `url`
  - `snippet`
  - `source` (`google`)
- `summary`
- `citations[]`

### 3. `search_extract`

Search first, then extract content from the top result pages.

**Parameters:**
- `action` (required, string): must be `"search_extract"`
- `query` (required, string): non-empty search query
- `engine` (optional, string, default `google`)
- `top_k` (optional, integer, default `5`): must be between `1` and `20`
- `extract_top_n` (optional, integer, default `3`): must be between `1` and `10`
- `wait_until` (optional, string, default `domcontentloaded`)

**Output:**
- same shape as `open_extract`
- if search yields no results, returns empty `items[]` with a clear summary

## When To Use

Use `browser_web` when the user explicitly wants browser-based reading/search, for example:
- "go to Google and search ..."
- "open this URL and extract the article"
- "search first, then read the top pages"
- dynamic or JS-heavy page reading where a real browser is more suitable than plain HTTP fetch

## When Not To Use

Do NOT use `browser_web` for:
- local file parsing (`doc_parse` fits that better)
- structured array filtering/sorting/table conversion (`transform` fits that better)
- generic API-style search when a dedicated search backend exists (`web_search_extract`)
- dangerous website interaction (login, submit, purchase, form posting)

## Dependencies

This skill requires:
1. Node.js available in `PATH`
2. Playwright dependency installed in `crates/skills/browser_web`
   - `npm install`
3. Chromium installed for Playwright
   - `npx playwright install chromium`

If dependencies are missing, the skill returns a clear error instead of fabricating success.

## Error Contract

- non-object `args` -> `status=error`, `error_text="args must be object"`
- invalid or non-`http/https` URL -> clear error
- invalid parameter ranges -> clear error, no silent clamping
- unsupported engine -> clear error
- missing Node.js / Playwright / helper script -> clear error
- page/search extraction failures are reported explicitly; content is never fabricated
