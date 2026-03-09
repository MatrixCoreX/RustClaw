# docker_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the docker_basic implementation.

## Capability Summary
- `docker_basic` provides common Docker inspection and container lifecycle helpers.
- It focuses on targeted container actions and avoids broad destructive cleanup.

## Actions
- `ps`
- `images`
- `logs`
- `restart`
- `start`
- `stop`
- `inspect`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported Docker actions. |
| `logs` | `container` | yes | string | - | Target container name/id. |
| `logs` | `tail` | no | number | impl default | Number of log lines to show. |
| `restart`/`start`/`stop`/`inspect` | `container` | yes | string | - | Target container name/id. |
| `ps`/`images` | none | no | - | - | List containers/images. |

## Error Contract
- Missing required `container` for container-specific actions.
- Unsupported action names.
- Docker daemon/CLI errors must be returned with readable output.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"logs","container":"clawd","tail":100}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"...container logs...","error_text":null}
```
