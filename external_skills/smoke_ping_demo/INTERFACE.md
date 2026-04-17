# smoke_ping_demo Interface Spec

> This file was scaffolded by `extension_manager`.
> Keep it aligned with `external_skills/smoke_ping_demo/src/main.rs`.

## Capability Summary
- Return a short success text for action ping.
- This scaffold is intentionally generated in a disabled state; registration and enablement must be explicit.

## Actions
- `ping`: TODO: describe what this action should do.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `ping` | `action` | yes | string | `ping` | Fixed action selector. |

## Error Contract
- Return `status=error` with readable `error_text` when required params are missing.
- Return `unsupported action: <name>` for unknown actions.
- Keep request/response payloads as single-line JSON objects over stdin/stdout.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","context":null,"user_id":1,"chat_id":1,"args":{"action":"ping"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"TODO","extra":{"action":"ping"},"error_text":null}
```
