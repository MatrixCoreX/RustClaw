# http_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the http_basic implementation.

## Capability Summary
- `http_basic` performs simple HTTP requests for fetch and JSON post use cases.
- It is intended for lightweight API calls with explicit URL and optional headers/body.
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
| `post_json` | `body` | no | object/array/scalar | - | JSON payload for POST request. |

## Error Contract
- Missing/invalid URL or unsupported action.
- Network, timeout, or response-read failures should return readable error text.
- Invalid JSON body serialization errors should be surfaced explicitly.
- HTTP responses with non-2xx status codes are successful observations, not transport failures.
- Received responses mirror structured metadata into `extra`, including `action`, `url`, `status_code`, `success_status`, and `body_preview`.

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
