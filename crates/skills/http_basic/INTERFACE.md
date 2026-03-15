# http_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the http_basic implementation.

## Capability Summary
- `http_basic` performs simple HTTP requests for fetch and JSON post use cases.
- It is intended for lightweight API calls with explicit URL and optional headers/body.

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
| `post_json` | `body` | no | object/array/scalar | - | JSON payload for POST request. |

## Error Contract
- Missing/invalid URL or unsupported action.
- Network/timeouts/HTTP errors should return readable error text.
- Invalid JSON body serialization errors should be surfaced explicitly.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"get","url":"https://example.com/api/ping"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"{\"ok\":true}","error_text":null}
```
