# health_check Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the health_check implementation.

## Capability Summary
- `health_check` runs baseline diagnostics and status checks for environment/runtime health.
- It is read-only and should not perform mutating operations.

## Actions
- No explicit action is required for baseline diagnostics.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| check | none | no | - | - | Execute default health diagnostics. |
| check | `log_dir` | no | string(path) | impl default | Optional log source path override. |

## Error Contract
- Invalid log path should return clear filesystem errors.
- Diagnostic execution/runtime failures should return explicit error text.
- `log_dir` rejects `..` traversal and paths outside workspace.
- Successful responses also mirror the parsed diagnostic object into the optional `extra` field.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"{\"workspace_root\":\"/workspace\",\"log_dir\":\"/workspace/logs\",\"clawd_process_count\":1}","extra":{"workspace_root":"/workspace","log_dir":"/workspace/logs","clawd_process_count":1},"error_text":null}
```
