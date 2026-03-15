# service_control Interface Spec

> This file is the source of truth for the skill implementation and for sync_skill_docs.py.

## Capability Summary

- `service_control` performs service lifecycle operations and diagnosis with **structured input/output**.
- **Managers implemented**: `rustclaw` (HTTP to clawd), `systemd`, `service`. Others return "not implemented".
- **Behavior**: Read-only first; high-risk (stop/restart) blocked for ambiguous targets; auto-verify after start/restart/reload; auto logs on failure.
- **Security**: Target validation; no arbitrary shell; RustClaw whitelist for HTTP path; safe unit names for systemd/service.

## Supported Actions

- `status` — Get running state (one or all for rustclaw).
- `start`, `stop`, `restart`, `reload` — Lifecycle (reload → restart for rustclaw).
- `logs` — Bounded recent logs (rustclaw: fixed log files; systemd: journalctl).
- `verify` — Explicit post-check (running/stopped/unknown).
- `diagnose_start_failure`, `diagnose_unhealthy_state` — status + logs + evidence summary.

## Manager Types

- **rustclaw** — Auto when target is in whitelist: `clawd`, `telegramd`, `whatsappd`, `whatsapp_webd`, `feishud`, `larkd`. Uses `/v1/health` and `/v1/services/{service}/{action}`.
- **systemd** — Explicit `manager_type: "systemd"`; target = unit name; uses `systemctl` / `journalctl`.
- **service** — Explicit `manager_type: "service"`; target = service name; uses `service <name> status/start/stop/restart/reload`.
- **docker_compose**, **docker_container**, **supervisor**, **process_only**, **unknown** — Recognized in input; status/logs may return "not implemented" for lifecycle.

## Input Contract

| Param         | Required | Type   | Default | Description |
|---------------|----------|--------|---------|-------------|
| `action`      | yes      | string | -       | One of: `status`, `start`, `stop`, `restart`, `reload`, `logs`, `verify`, `diagnose_start_failure`, `diagnose_unhealthy_state`. |
| `target`      | yes*     | string | -       | Service/unit name. *Optional for `status` (all services when manager is rustclaw). |
| `service`     | no       | string | -       | Alias for `target` (backward compatible). |
| `manager_type`| no       | string | -       | One of: `systemd`, `service`, `docker_compose`, `docker_container`, `supervisor`, `process_only`, `rustclaw`, `unknown`. Auto when target in rustclaw whitelist. |
| `tail_lines`  | no       | number | 100     | Max 500. For `logs` and for auto-logs on failure. |
| `lines`       | no       | number | 100     | Alias for `tail_lines`. |
| `verify`      | no       | bool   | true    | After start/restart/reload, run verify step. |
| `allow_risky` | no       | bool   | false   | If true, allow stop/restart even when target is ambiguous (not recommended). |

- **Target missing**: Required for all actions except `status` without target; returns structured error with `failure_reason` and `next_step`.
- **Ambiguous target**: Values like "后端", "服务们", "all", "*" block high-risk actions unless `allow_risky` is true.

## Output Contract (structured JSON in `text`)

The skill always returns a JSON object (in the runner response `text` field) with at least:

- `status` — `"ok"` or `"error"`.
- `service_name` — Target name.
- `manager_type` — Resolved or specified manager.
- `requested_action` — Requested action.
- `executed_actions` — List of steps actually run (e.g. `["status","restart","verify"]`).
- `pre_state`, `post_state` — Observed state before/after.
- `verified` — Whether post-action verification passed.
- `key_evidence` — Array of short evidence strings (status output, log summary).
- `failure_reason` — Non-empty on failure.
- `next_step` — Suggestion when applicable.
- `summary` — Short human-readable summary.

Failure responses must include `failure_reason`; when logs were inspected, key evidence is in `key_evidence`.

## Log Paths (rustclaw only)

- `logs/clawd.log`, `logs/telegramd.log`, `logs/whatsappd.log`, `logs/whatsapp_webd.log`, `logs/feishud.log`, `logs/larkd.log`.

## Error Contract

- Missing or invalid `action` / unknown `target` → clear `failure_reason`.
- `clawd` → start/stop/restart return error (main daemon).
- Ambiguous target + stop/restart without `allow_risky` → refuse with `failure_reason` and `next_step`.
- Manager not implemented for the action → `failure_reason` and optional `next_step`.
- API 401 (rustclaw) → suggest RUSTCLAW_UI_KEY.

## Request/Response Examples

### status (all, rustclaw)

Request:
```json
{"request_id":"r1","args":{"action":"status"}}
```

### status (one service)

```json
{"request_id":"r2","args":{"action":"status","target":"telegramd"}}
```

### start with verify

```json
{"request_id":"r3","args":{"action":"start","target":"telegramd","verify":true}}
```

### logs

```json
{"request_id":"r4","args":{"action":"logs","target":"clawd","tail_lines":100}}
```

### systemd restart

```json
{"request_id":"r5","args":{"action":"restart","target":"nginx","manager_type":"systemd"}}
```

Response (concept): skill returns `{"request_id":"...","status":"ok","text":"{...structured output...}","error_text":null}` where `text` is the JSON output contract above.
