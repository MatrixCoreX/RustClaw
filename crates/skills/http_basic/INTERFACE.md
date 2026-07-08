# http_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the http_basic implementation.

## Capability Summary
- `http_basic` performs simple HTTP requests for fetch and JSON post use cases.
- It is intended for lightweight API calls with explicit URL and optional headers/body.
- Use it for raw HTTP/API observations, status validation, response preview checks, and downloads.
- Do not use it as the primary page-reading capability when the task needs browser-rendered page titles, readable article/page text, page summaries, screenshots, or extraction artifacts; those belong to `browser_web.open_extract`.
- When called inside RustClaw with a valid `user_key`, requests to local RustClaw API endpoints on `http://127.0.0.1:8787/` automatically include `X-RustClaw-Key`.
- Any received HTTP response is returned as an observation, including non-2xx statuses; network/timeout/protocol failures remain skill errors.

## Actions
- `get`
- `post_json`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be `get` or `post_json`. |
| all | `url` | yes | string | - | Must start with `http://` or `https://`. |
| all | `headers` | no | object | `{}` | Optional request headers map. |
| all | `timeout_seconds` | no | number | impl default | Request timeout override. |
| all | `expect_status` | no | integer/string | - | Runtime validation hint: require this exact HTTP status when the step is meant as validation. |
| all | `expect_success` | no | boolean | `false` | Runtime validation hint: require a 2xx status when the step is meant as validation. |
| all | `expect_contains` | no | string | - | Runtime validation hint: require the response body preview to contain this text. |
| all | `accept_non_success` | no | boolean | `false` | Runtime validation hint: allow non-2xx responses when validating `expect_contains`. |
| all | `download` | no | boolean | `false` | Save the response body to a workspace-local artifact path. |
| all | `output_path` | no | string | `document/http/download/http-<ts>.body` | Workspace-local destination path; absolute paths must still stay inside the current workspace. |
| `post_json` | `body` | no | object/array/scalar | - | JSON payload for POST request. |

## Error Contract
- Missing/invalid URL or unsupported action.
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
