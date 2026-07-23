# browser_web Interface Spec

## Capability Summary

`browser_web` renders exact public HTTP(S) URLs in a headless browser and
returns bounded page evidence. It is the fetch half of the Web research
workflow:

1. Use `web_search_extract` to discover candidate URLs.
2. Select relevant candidates.
3. Use `browser.open_extract` only when browser rendering is needed.

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
- `screenshot_dir` (optional, string, default
  `skills_output/browser_web/screenshots`; must remain inside the workspace)
- `content_mode` (optional, string, default `clean`): `clean|raw`
- `max_text_chars` (optional, integer, default `12000`, range `100..200000`)
- `min_content_chars` (optional, integer, default `200`, range `20..10000`)
- `fail_fast` (optional, boolean, default `false`)
- `wait_map_path` (optional, string): existing workspace-local JSON file
- `domains_allow` (optional, string[], maximum 32 entries)
- `domains_deny` (optional, string[], maximum 32 entries)

At least one of `url` or `urls` is required. Duplicate normalized targets are
removed before execution.

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
- Browser page text is capped by `max_text_chars`; captured raw HTML is capped
  at 4 MiB and reports truncation explicitly.
- Runtime artifacts are writes, so the registry declares
  `filesystem_write=true` even though the external operation is observational.

## Output Contract

Successful protocol responses use outer `status=ok`. The JSON object serialized
in `text` is also mirrored into `extra`, where runtime policy and final
synthesis consume structured fields.

Top-level fields include:

- `schema_version`
- `source_skill`
- `status`
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

Each successful `items[]` entry includes:

- `url`, `final_url`, `title`, `text`, `content_excerpt`, `source`
- `fetch_method=browser`
- `response_status`, `extracted_at`, `latency_ms`
- `nav_wait_until`, `nav_attempts`, `nav_attempt_trace`
- `text_truncated`, `text_chars_before_limit`, `content_sha256`
- `screenshot_path`, `capture_artifacts`
- `provenance`, `trust`, `wait_strategy`, `runtime`

Each failed or partial item includes:

- `fetch_method=unavailable|browser_partial`
- `error_code`
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

- `INVALID_INPUT`
- `INVALID_ACTION`
- `URL_INVALID`
- `URL_SCHEME_BLOCKED`
- `URL_CREDENTIALS_BLOCKED`
- `DOMAIN_BLOCKED`
- `DOMAIN_NOT_ALLOWED`
- `DNS_RESOLUTION_FAILED`
- `PRIVATE_NETWORK_BLOCKED`
- `WORKSPACE_PATH_OUTSIDE`
- `DEPENDENCY_MISSING`
- `NAV_TIMEOUT`
- `BOT_BLOCKED`
- `SELECTOR_MISS`
- `BROWSER_OPERATION_FAILED`
- `RESPONSE_TOO_LARGE`
- `CONTENT_TYPE_BLOCKED`

Challenge detection uses HTTP status or DOM challenge signals. It does not
classify errors by matching natural-language exception or page text.

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
