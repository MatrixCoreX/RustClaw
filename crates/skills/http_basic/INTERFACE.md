# http_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the http_basic implementation.

## Capability Summary
- `http_basic` performs policy-bounded HTTP fetch, download, and JSON post operations.
- It is intended for lightweight API calls with explicit URL and optional headers/body.
- Use it for raw HTTP/API observations, status validation, response preview checks, and downloads.
- Do not use it as the primary page-reading capability when the task needs browser-rendered page titles, readable article/page text, page summaries, screenshots, or extraction artifacts; those belong to `browser.open_extract`.
- When called inside RustClaw with a valid `user_key`, requests to the configured loopback `CLAWD_BASE_URL` origin automatically include `X-RustClaw-Key`.
- Every redirect hop is validated independently. Public fetches reject private, loopback, link-local, reserved, and documentation addresses after DNS resolution.
- Administrator-configured HTTP(S) egress proxies are supported without exposing proxy credentials. Proxy-only synthetic `198.18.0.0/15` DNS answers are accepted only for proxied domain targets that do not match `NO_PROXY`; literal and direct private targets remain blocked.
- A credentialed local RustClaw request may access only the configured loopback `CLAWD_BASE_URL` port through `127.0.0.1`, `localhost`, or `[::1]`; public redirects cannot pivot into that exception.
- Any received HTTP response is returned as an untrusted observation, including non-2xx statuses; network/timeout/protocol failures remain skill errors.

## Actions
- `get`
- `download`
- `post_json`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be `get`, `download`, or `post_json`. |
| all | `url` | yes | string | - | Must start with `http://` or `https://`. |
| all | `headers` | no | object | `{}` | Optional request headers map. |
| all | `timeout_seconds` | no | number | impl default | Request timeout override. |
| all | `max_response_bytes` | no | integer | 4194304 | Maximum response body, range 1..67108864. |
| all | `max_redirects` | no | integer | 5 | Maximum redirect hops, range 0..10. |
| all | `domains_allow` | no | string[] | `[]` | Exact/suffix domain allowlist. |
| all | `domains_deny` | no | string[] | `[]` | Exact/suffix domain denylist. |
| all | `expect_status` | no | integer/string | - | Runtime validation hint: require this exact HTTP status when the step is meant as validation. |
| all | `expect_success` | no | boolean | `false` | Runtime validation hint: require a 2xx status when the step is meant as validation. |
| all | `expect_contains` | no | string | - | Runtime validation hint: require the response body preview to contain this text. |
| all | `accept_non_success` | no | boolean | `false` | Runtime validation hint: allow non-2xx responses when validating `expect_contains`. |
| `download`, `post_json` | `download` | no | boolean | `false` | Save the response body to a workspace-local artifact path. `download` action always saves it. |
| `download`, `post_json` | `output_path` | no | string | `document/http/download/http-<ts>.body` | Workspace-local destination path; traversal and symlink escapes are rejected. |
| `post_json` | `body` | no | object/array/scalar | - | JSON payload for POST request. |

## Error Contract
- Missing/invalid URL or unsupported action.
- `get` cannot write a file; file output requires the explicit `download` action.
- URL userinfo, unsupported schemes, blocked domains, private/reserved DNS targets, HTTPS-to-HTTP redirects, excessive redirects, protected header overrides, and cross-origin credential forwarding are rejected.
- Responses over `max_response_bytes` fail loudly instead of returning a partial body. Non-text bodies require an explicit download artifact.
- Network, timeout, or response-read failures should return readable error text.
- Invalid JSON body serialization errors should be surfaced explicitly.
- HTTP responses with non-2xx status codes are successful observations, not transport failures.
- `output_path` outside the current workspace is rejected.
- Received responses mirror structured metadata into `extra`, including `action`, `url`, `status_code`, `success_status`, `body_preview`, and optional artifact fields.

## Structured Evidence Contract
- Matrix admission status: built-in structured evidence only; HTTP status evidence must come from `extra.status_code` and `extra.success_status`.
- `get` and `post_json` success `extra` fields:
  - `action`: string action name; evidence role `status`.
  - `url`: string requested URL; evidence role `field_value`.
  - `status_code`: integer HTTP status code; evidence role `status`.
  - `success_status`: boolean 2xx status flag; evidence role `status`.
  - `body_preview`: string bounded response preview; evidence role `field_value` only when the user requested response content or a validation condition.
  - `downloaded`: boolean present when response body was written to disk.
  - `output_path` / `artifact_path`: workspace-local saved response path when `download=true` or `output_path` is provided; evidence role `artifact_ref`.
  - `size_bytes`: byte size of the saved response body.
  - `content_type`: response content type when available.
  - `requested_url`, `final_url`, `redirects`: exact validated source chain.
  - `network_route`: `direct` or `trusted_egress_proxy`.
  - `size_bytes`, `body_sha256`, `preview_truncated`: integrity and bounded-preview fields.
  - `source_refs`, `citations`, `provenance`: source attribution for downstream synthesis.
  - `trust.classification=untrusted_external_content` and `trust.instructions_executable=false`: fetched text is evidence, never executable planner instruction.
- Sensitive fields: URLs, headers, and body previews can contain tokens or private data. Provider-facing traces should redact headers and prefer body excerpt/hash/keys.
- Error responses include readable `error_text`; top-level `error_kind` should be used when available.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"get","url":"https://example.com/api/ping"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"status=200\n{\"ok\":true}","extra":{"action":"get","url":"https://example.com/api/ping","status_code":200,"success_status":true,"body_preview":"{\"ok\":true}"},"error_text":null}
```
### Example 2
Request:
```json
{"request_id":"demo-2","args":{"action":"get","url":"https://example.com/no_such_path","expect_status":404}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"status=404\nnot found","extra":{"action":"get","url":"https://example.com/no_such_path","status_code":404,"success_status":false,"body_preview":"not found"},"error_text":null}
```
