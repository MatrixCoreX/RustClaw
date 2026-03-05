# process_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the process_basic implementation.

## Capability Summary
- `process_basic` provides process inspection and targeted process control operations.
- It supports listing processes/ports, killing a PID, and tailing logs.

## Actions
- `ps`
- `port_list`
- `kill`
- `tail_log`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported actions. |
| `ps` | `limit` | no | number | impl default | Max number of process rows. |
| `port_list` | none | no | - | - | List listening/used ports. |
| `kill` | `pid` | yes | number | - | Target process id. |
| `kill` | `signal` | no | string | `TERM` | Signal name/number for termination. |
| `tail_log` | `path` | yes | string(path) | - | Log file path to tail. |
| `tail_log` | `n` | no | number | impl default | Number of trailing lines. |

## Error Contract
- Missing required `pid`/`path` for action-specific operations.
- Invalid PID/signal/path values.
- OS command failures are returned with readable error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"ps","limit":20}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"PID ...","error_text":null}
```
