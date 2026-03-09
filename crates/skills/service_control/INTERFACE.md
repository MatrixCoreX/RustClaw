# service_control Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the service_control implementation.

## Capability Summary
- `service_control` performs service lifecycle operations for managed services.
- It supports status checks and start/stop/restart control.

## Actions
- `status`
- `start`
- `stop`
- `restart`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of `status|start|stop|restart`. |
| all | service selector fields | implementation-defined | string | - | Service target identity according to runtime adapter. |

## Error Contract
- Unsupported `action` names must return clear errors.
- Missing/unknown service target should return readable error text.
- Runtime control failures must include command/system error details.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"status"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"service status: ...","error_text":null}
```
