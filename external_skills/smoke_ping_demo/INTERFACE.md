# smoke_ping_demo Interface Spec

> This file was scaffolded by `extension_manager`.
> Keep it aligned with `external_skills/smoke_ping_demo/src/main.rs`.

## Capability Summary
- Return a short success text for action ping.
- This scaffold is intentionally generated in a disabled state; registration and enablement must be explicit.

## Config Entry Points
- No dedicated config file, environment variable, local database, API session, or external dependency is required.

## Actions
- `ping`: TODO: describe what this action should do.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `ping` | `action` | yes | string | `ping` | Fixed action selector. |

## Error Contract
- Return `status=error` with readable `error_text` when required params are missing.
- Return `unsupported action: <name>` for unknown actions.
- Error responses include `extra.error_kind` with one of `invalid_args`, `invalid_input`, `missing_action`, or `unsupported_action`.
- Keep request/response payloads as single-line JSON objects over stdin/stdout.

## Structured Evidence Contract
- Matrix admission status: example only; do not mark registry `matrix_admission.eligible=true` until registration validation verifies these fields.
- `ping` success `extra` fields:
  - `action`: stable action string, always `ping`.
  - `ok`: boolean success flag, always `true`.
  - `message`: stable machine-readable string, currently `pong`.
- `ping` error `extra` fields:
  - `error_kind`: stable machine-readable error kind.
- Sensitive fields: none.
- Strict evidence eligibility:
  - `extra.ok` may satisfy a boolean/status evidence field only after admission validation.
  - `extra.message` may satisfy a scalar field only after admission validation.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","context":null,"user_id":1,"chat_id":1,"args":{"action":"ping"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"pong","extra":{"action":"ping","ok":true,"message":"pong"},"error_text":null}
```
