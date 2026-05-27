# process_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the process_basic implementation.

## Capability Summary
- `process_basic` provides process inspection and targeted process control operations.
- It supports listing processes/ports, killing a PID, and tailing logs.
- Use `port_list` for local listening-port checks, including requests that ask whether a runtime such as `clawd` is listening on a specific port.
- `port_list` chooses OS-native probes first: Linux uses `ss` with `lsof`/`netstat` fallback; macOS uses `lsof` with `netstat` fallback. The successful response includes `extra.platform` and `extra.command_tool`.

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
| `ps` | `filter` / `query` / `name` | no | string | - | Case-insensitive process command filter. |
| `port_list` | `filter` / `query` / `port` | no | string | - | Optional substring filter, commonly a port number or process name. |
| `kill` | `pid` | yes | number | - | Target process id. |
| `kill` | `signal` | no | string | `TERM` | Signal name/number for termination. |
| `tail_log` | `path` | yes | string(path) | - | Log file path to tail. |
| `tail_log` | `n` | no | number | impl default | Number of trailing lines. |

## Error Contract
- Missing required `pid`/`path` for action-specific operations.
- Invalid PID/signal/path values.
- OS command failures are returned with readable error text.
- Non-zero subprocess exit codes are returned as `status=error` with `error_text=process command failed: exit=<code>\n<stdout/stderr>`.
- Successful responses also mirror structured metadata into `extra`, including fields like `action`, `exit_code`, `platform`, `command_tool` for `port_list`, and `output`.

## Structured Evidence Contract
- Matrix admission status: built-in structured evidence only; `output` remains legacy text evidence unless a stricter parser is explicitly registered.
- Successful response `extra` fields:
  - `action`: string action name; evidence role `status`.
  - `exit_code`: integer subprocess exit code; evidence role `status`.
  - `platform`: string OS/platform; evidence role `field_value`.
  - `command_tool`: string selected probe for `port_list`; evidence role `field_value`.
  - `limit`, `filter`, `pid`, `signal`, `path`, or `n`: echoed typed inputs when applicable; evidence roles `field_value` and `path`.
  - `output`: string bounded process/log observation; fallback evidence only.
- Sensitive fields: process command lines and log tails can contain secrets or user data. Provider-facing traces should prefer counts, selected fields, excerpts, or hashes.
- Error responses include readable `error_text`; top-level `error_kind` should be used when available.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"ps","limit":20}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"exit=0\nPID ...","extra":{"action":"ps","exit_code":0,"limit":20,"platform":"linux","output":"exit=0\nPID ..."},"error_text":null}
```
