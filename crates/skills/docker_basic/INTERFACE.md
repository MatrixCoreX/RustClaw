# docker_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the docker_basic implementation.

## Capability Summary
- `docker_basic` provides common Docker inspection and container lifecycle helpers.
- It focuses on targeted container actions and avoids broad destructive cleanup.

## Actions
- `ps`
- `images`
- `version`
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
| `ps`/`images`/`version` | none | no | - | - | List containers/images or inspect Docker version availability. |

## Error Contract
- Missing required `container` for container-specific actions.
- Unsupported action names.
- Read-only inspection actions (`ps`, `images`, `version`) return `status=ok` with `available=false` and readable output when the Docker CLI or daemon is unavailable, because that is still an environment observation.
- Container-specific lifecycle/log/inspect actions return Docker daemon/CLI errors with readable output.
- For mutating/container-specific actions, non-zero `docker` command exit codes are returned as `status=error` with `error_text=docker command failed: exit=<code>\n<stdout/stderr>`.
- Successful responses also mirror structured metadata into `extra`, including `action`, `exit_code`, `docker_args`, and `output`.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"logs","container":"clawd","tail":100}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"exit=0\n...container logs...","extra":{"action":"logs","exit_code":0,"docker_args":["logs","--tail","100","clawd"],"output":"exit=0\n...container logs..."},"error_text":null}
```
