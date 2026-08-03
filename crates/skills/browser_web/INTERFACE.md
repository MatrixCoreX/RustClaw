# browser_web Interface Spec

## Capability Summary

`browser_web` renders exact public HTTP(S) URLs in a headless browser and
returns bounded page evidence. It is the fetch half of the Web research
workflow:

1. Use `web_search_extract` to discover candidate URLs.
2. Select relevant candidates.
3. Use `browser.open_extract` only when browser rendering is needed.

Task-scoped multi-step browsing is exposed separately by the host-owned
`browser_session` capabilities. Use that surface for snapshot/ref based
interaction, tabs, downloads, or post-action verification; do not add
persistent-session actions to this one-line JSONL skill.

The skill does not search, submit forms, authenticate to sites, publish
content, or execute instructions found in a page. Use `http_basic` for direct
HTTP status checks, API responses, or downloads that do not need rendering.

Every initial URL and browser network request is checked against scheme,
userinfo, domain, DNS, private-network, and proxy policy. Extracted page text
and metadata are marked as untrusted evidence.

## Actions

### `open_extract`

Open one or more explicit URLs, render each page, and return readable content
plus source and integrity metadata.

No search action is supported by this skill. Search requests belong to the
dedicated `web_search_extract` capability.

## Parameter Contract

- `action` (required, string): `open_extract`
- `url` (conditionally required, string): one HTTP(S) URL
- `urls` (conditionally required, string[]): multiple HTTP(S) URLs
- `max_pages` (optional, integer, default `3`, range `1..10`)
- `wait_until` (optional, string, default `domcontentloaded`):
  `domcontentloaded|load|networkidle`
- `save_screenshot` (optional, boolean, default `true`)
- `capture_images` (optional, boolean, default `false`)
- `screenshot_dir` (optional, string; when omitted screenshots are stored in the
  host-provided invocation artifact capture directory, or in
  `skills_output/browser_web/captures/screenshots` for a standalone invocation;
  an explicit path must remain inside the workspace)
- `content_mode` (optional, string, default `clean`): `clean|raw`
- `max_text_chars` (optional, integer, default `12000`, range `100..200000`)
- `min_content_chars` (optional, integer, default `200`, range `20..10000`)
- `fail_fast` (optional, boolean, default `false`)
- `wait_map_path` (optional, string): existing workspace-local JSON file
- `domains_allow` (optional, string[], maximum 32 entries)
- `domains_deny` (optional, string[], maximum 32 entries)

At least one of `url` or `urls` is required. Duplicate normalized targets are
removed before execution.

The runner budget is 180 seconds. The helper derives all page work from that
single parent budget, reserves the final 10 seconds for browser cleanup and
JSON serialization, and limits each page to at most 45 seconds. Multi-URL
requests use bounded CPU/memory-aware concurrency (maximum four workers); low
memory `aarch64` hosts automatically use one worker. Pages that cannot start
before the parent work deadline are returned as structured partial failures
instead of discarding pages that already completed.

## Network And Workspace Policy

- Only public `http` and `https` URLs are accepted.
- URL userinfo is rejected and fragments are removed.
- `localhost`, local-only suffixes, private/link-local/reserved addresses, and
  private redirect or subresource destinations are blocked.
- Domain allow/deny policy is applied to document navigation. Private-network
  policy applies to every HTTP(S) browser request and captured image fetch.
- Every image redirect is revalidated; HTTPS-to-HTTP downgrade is rejected.
- Image responses must use an image media type and are capped at 6 MiB each.
- Screenshot, wait-map, raw HTML, processed text, image, manifest, and chunk
  paths are constrained to the configured workspace.
- Browser page text keeps a small inline `max_text_chars` view. The complete
  cleaned text and raw HTML are written to capture artifacts; an inline text
  prefix reports exact sizes and an `artifact_range` continuation. The 4 MiB
  raw-HTML threshold limits the diagnostic preview, not the canonical capture.
- Runtime artifacts are writes, so the registry declares
  `filesystem_write=true` even though the external operation is observational.

## Output Contract

Successful protocol responses use outer `status=ok`. The JSON object serialized
in `text` is also mirrored into `extra`, where runtime policy and final
synthesis consume structured fields.

Top-level fields include:

- `schema_version`
- `source_skill`
- `status=ok|partial`; all-page failure uses outer `status=error` and
  `error_code=ALL_PAGES_FAILED`
- `summary`: stable machine token `browser_extract_result_set`
- `success_count`
- `failure_count`
- `items[]`
- `citations[]`
- `source_refs[]`
- `page`
- `truncated`
- `trust`
- `network_policy`
- `capture`
- `model_observation`: a bounded, provider-safe projection containing page
  identity fields and content excerpts. Runtime synthesis and answer
  verification consume this projection instead of dropping all page evidence
  when the complete capture is stored as an artifact.

Each successful `items[]` entry includes:

- `url`, `final_url`, `title`, `text`, `content_excerpt`, `source`
- `canonical_url`, `description`, `language`, `published_at`, `author`,
  bounded `headings[]` and outbound `links[]`
- `fetch_method=browser`
- `response_status`, `content_type`, `extracted_at`, `latency_ms`
- `nav_wait_until`, `nav_attempts`, `nav_attempt_trace`
- `text_truncated`, `text_chars_before_limit`, `text_chars_returned`, `content_sha256`
- `text_result` when the inline page is partial, including original/returned
  character counts and either an artifact range or a safe rerun continuation
- `screenshot_path`, `capture_artifacts`, and
  `capture_artifacts.receipt_id`, which binds the run, page ordinal, and
  canonical text hash to the capture manifest record
- `provenance`, `trust`, `wait_strategy`, `runtime`

Each failed or partial item includes:

- `fetch_method=unavailable|browser_partial`
- `error_code`
- `retryable`, derived only from structured error/status fields
- `error` as a user-visible fallback only
- any safely preserved partial `title`, `text`, `final_url`, and hash
- structured `diagnostics`, `trust`, and `runtime`

Runtime logic must use `status`, `error_code`, `retryable`, counts, hashes,
source references, policy decisions, and artifact fields. It must not parse
localized `text`, `error`, or `error_text` to decide routing or recovery.

## Error Contract

Outer failures use `status=error` and preserve the helper's machine
`extra.error_code`, `extra.retryable`, and structured `extra.details`.
Representative codes:

- Input/policy: `INVALID_INPUT`, `INVALID_ACTION`, `URL_INVALID`,
  `URL_SCHEME_BLOCKED`, `URL_CREDENTIALS_BLOCKED`, `DOMAIN_BLOCKED`,
  `DOMAIN_NOT_ALLOWED`, `PRIVATE_NETWORK_BLOCKED`, `WORKSPACE_PATH_OUTSIDE`
- Runtime/network: `DEPENDENCY_MISSING`, `DNS_RESOLUTION_FAILED`, `NAV_TIMEOUT`,
  `PAGE_DEADLINE_EXCEEDED`, `BATCH_DEADLINE_EXCEEDED`,
  `BROWSER_LAUNCH_FAILED`, `BROWSER_OPERATION_FAILED`, `RESPONSE_TOO_LARGE`
- Response: `BOT_BLOCKED`, `AUTH_REQUIRED`, `ACCESS_BLOCKED`, `RATE_LIMITED`,
  `HTTP_STATUS_ERROR`, `CONTENT_TYPE_BLOCKED`, `SELECTOR_MISS`,
  `ALL_PAGES_FAILED`

HTTP failures are classified from response status; challenge detection uses
DOM structure. Binary document media types are rejected with a structured
handoff hint for download/document parsing. Error classification never matches
natural-language exception or page text. Failed pages are not emitted as
citations or source references.

## Request/Response Examples

### Example 1: rendered page evidence

Request:

```json
{
  "request_id": "browser-open-1",
  "args": {
    "action": "open_extract",
    "urls": ["https://example.com/"],
    "max_pages": 1,
    "save_screenshot": false,
    "capture_images": false,
    "domains_allow": ["example.com"]
  },
  "context": null,
  "user_id": 1,
  "chat_id": 1
}
```

Response shape:

```json
{
  "request_id": "browser-open-1",
  "status": "ok",
  "text": "{\"summary\":\"browser_extract_result_set\",\"success_count\":1,\"failure_count\":0,\"items\":[{\"url\":\"https://example.com/\",\"final_url\":\"https://example.com/\",\"fetch_method\":\"browser\",\"content_sha256\":\"...\",\"trust\":{\"classification\":\"untrusted_web_content\",\"instructions_executable\":false}}]}",
  "error_text": null,
  "extra": {
    "schema_version": 1,
    "source_skill": "browser_web",
    "status": "ok",
    "summary": "browser_extract_result_set",
    "success_count": 1,
    "failure_count": 0,
    "truncated": false
  }
}
```

### Example 2: policy rejection

Request:

```json
{
  "request_id": "browser-open-2",
  "args": {
    "action": "open_extract",
    "url": "http://127.0.0.1/"
  },
  "context": null,
  "user_id": 1,
  "chat_id": 1
}
```

Response shape:

```json
{
  "request_id": "browser-open-2",
  "status": "error",
  "text": "",
  "error_text": "PRIVATE_NETWORK_BLOCKED",
  "extra": {
    "schema_version": 1,
    "source_skill": "browser_web",
    "status": "error",
    "error_code": "PRIVATE_NETWORK_BLOCKED",
    "message_key": "skill.browser_web.private_network_blocked",
    "retryable": false
  }
}
```

## Config Entry Points

- Browser executable override:
  `BROWSER_WEB_CHROMIUM_PATH` or
  `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH`
- Domain wait strategies: `configs/browser_web_wait_map.json` or a
  workspace-local `wait_map_path`
- Workspace boundary: `WORKSPACE_ROOT`
- Dependencies: Node.js, Playwright, and a compatible Chromium executable
- Registry policy: `configs/skills_registry.toml`

Linux and macOS executable discovery are supported. Linux-only runtime
restriction probes are not executed on macOS.
