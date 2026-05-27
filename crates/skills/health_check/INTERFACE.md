# health_check Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the health_check implementation.

## Capability Summary
- `health_check` runs baseline diagnostics and status checks for environment/runtime health.
- It now returns both RustClaw runtime fields and a structured `system_health` block for the host OS.
- `system_health.os_family` explicitly distinguishes `linux` and `macos` so downstream logic can branch cleanly.
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

## Response Notes
- RustClaw runtime fields remain top-level for compatibility:
  - `clawd_process_count`
  - `telegramd_process_count`
  - `clawd_health_port_open`
  - `clawd_log`
  - `telegramd_log`
- Host OS diagnostics are grouped under `system_health`:
  - `os_family`, `arch`, `kernel_release`, `hostname`, `service_manager`
  - `cpu_count`, `uptime_seconds`
  - `load_avg_1m`, `load_avg_5m`, `load_avg_15m`
  - `memory_total_bytes`, `memory_available_bytes`
  - `disk_root_total_bytes`, `disk_root_available_bytes`
  - `warnings` (`disk_root_low`, `memory_available_low`, `load_high`)

## Structured Evidence Contract
- Matrix admission status: built-in structured evidence only; use `extra` fields for strict health/status evidence.
- Success `extra` fields:
  - `workspace_root`: string path; evidence role `path`.
  - `log_dir`: string path; evidence role `path`.
  - `clawd_process_count`, `telegramd_process_count`: integer counts; evidence role `count`.
  - `clawd_health_port_open`: boolean; evidence role `status`.
  - `clawd_log`, `telegramd_log`: object or scalar log observations; evidence role `field_value`.
  - `system_health`: object containing OS, CPU, uptime, load, memory, disk, and warning fields; evidence roles `field_value`, `count`, and `status`.
- Sensitive fields: log observations can include user data. Provider-facing traces should prefer warnings, counts, selected keys, excerpts, or hashes.
- Error responses include readable `error_text`; `extra.error_kind` should be used when implementation-specific context is available.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"{\"workspace_root\":\"/workspace\",\"log_dir\":\"/workspace/logs\",\"clawd_process_count\":1,\"system_health\":{\"os_family\":\"linux\",\"service_manager\":\"systemd\",\"warnings\":[]}}","extra":{"workspace_root":"/workspace","log_dir":"/workspace/logs","clawd_process_count":1,"system_health":{"os_family":"linux","service_manager":"systemd","warnings":[]}},"error_text":null}
```
