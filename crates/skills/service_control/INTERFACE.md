# service_control Interface Spec

> This file is the source of truth for the skill implementation and for sync_skill_docs.py.

## Capability Summary

- `service_control` performs service lifecycle operations and diagnosis with **structured input/output**.
- **Managers implemented**: `rustclaw` (HTTP to clawd), `systemd`, `service`, `brew_services`, `launchd`.
- **Behavior**: Read-only first; high-risk (stop/restart) blocked for ambiguous targets; auto-verify after start/restart/reload; auto logs on failure.
- **Status questions**: Runtime status must come from `status` / `verify` (or another real runtime check), not from binary-file existence.
- **Security**: Target validation; no arbitrary shell; RustClaw whitelist for HTTP path; safe unit names for systemd/service/brew services/launchd.

## Actions

- `status` — Get running state. When `target` is omitted, default to RustClaw aggregate status for the built-in RustClaw services.
- `start`, `stop`, `restart`, `reload` — Lifecycle (reload → restart for rustclaw).
- `logs` — Bounded recent logs (rustclaw: fixed log files; systemd: journalctl; macOS managers provide bounded diagnostic guidance/evidence).
- `verify` — Explicit post-check (running/stopped/unknown).
- `diagnose_start_failure`, `diagnose_unhealthy_state` — status + logs + evidence summary.

## Manager Types

- **rustclaw** — Auto when target is in whitelist: `clawd`, `telegramd`, `whatsappd`, `whatsapp_webd`, `feishud`, `larkd`. Uses `/v1/health` and `/v1/services/{service}/{action}`.
- **systemd** — Explicit `manager_type: "systemd"`; target = unit name; uses `systemctl` / `journalctl`.
- **service** — Explicit `manager_type: "service"`; target = service name; uses `service <name> status/start/stop/restart/reload`.
- **brew_services** — Explicit or auto-discovered on macOS/Homebrew; target = Homebrew service name; uses `brew services list` and `brew services start/stop/restart`.
- **launchd** — Explicit or auto-discovered on macOS; target = launchd label; status/verify supported via `launchctl list`; lifecycle control is intentionally limited, with guidance to prefer `brew_services` when applicable.
- **docker_compose**, **docker_container**, **supervisor**, **process_only**, **unknown** — Recognized in input; status/logs may return "not implemented" for lifecycle.

## Parameter Contract

| Param         | Required | Type   | Default | Description |
|---------------|----------|--------|---------|-------------|
| `action`      | yes      | string | -       | One of: `status`, `start`, `stop`, `restart`, `reload`, `logs`, `verify`, `diagnose_start_failure`, `diagnose_unhealthy_state`. |
| `target`      | yes*     | string | -       | Service/unit name. *Optional for `status`; omitted target checks RustClaw aggregate status for built-in RustClaw services by default. |
| `service`     | no       | string | -       | Alias for `target` (backward compatible). |
| `manager_type`| no       | string | -       | One of: `brew_services`, `launchd`, `systemd`, `service`, `docker_compose`, `docker_container`, `supervisor`, `process_only`, `rustclaw`, `unknown`. Auto when target in rustclaw whitelist or resolved through service discovery. |
| `tail_lines`  | no       | number | 100     | Max 500. For `logs` and for auto-logs on failure. |
| `lines`       | no       | number | 100     | Alias for `tail_lines`. |
| `verify`      | no       | bool   | true    | After start/restart/reload, run verify step. |
| `allow_risky` | no       | bool   | false   | If true, allow stop/restart even when target is ambiguous (not recommended). |

- **Target missing**: Required for all actions except `status` without target. `status` without target defaults to RustClaw aggregate status; other missing-target actions return structured error with `failure_reason` and `next_step`.
- **Target aliases (skill-internal)**: The skill normalizes common names before discovery: e.g. `mysql`/`mysqld` → `mysql`, `redis`/`redis-server` → `redis`, `postgres`/`postgresql` → `postgresql`, `docker`/`dockerd` → `docker`, `ssh`/`sshd` → `sshd`, `cron`/`crond` → `cron`, `nginx` → `nginx`, `caddy` → `caddy`. Trailing "服务" / " service" suffix is stripped for lookup. Only the target name is affected; `action` is unchanged.
- **Service discovery**: Before executing control (when manager is not explicitly set), the skill discovers candidates via Homebrew services, launchd, systemd, and `service --status-all` where those tools are available. Exact match > prefix > contains; candidate count is limited. If **0 candidates**: returns error with `next_step` suggesting "请提供更具体服务名" (do not invent a service name). If **>1 candidates**: returns error with `failure_reason` "ambiguous: multiple matching services" and `next_step` listing the candidates so the user can pick one. Only when exactly **1 candidate** is the control command executed. When `manager_type` is explicitly set, discovery is skipped and the normalized target is used as given.
- **Ambiguous target (vague wording)**: Values like "后端", "服务们", "all", "*" block high-risk actions unless `allow_risky` is true.

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
- `error_kind` — Non-empty on failure. Stable values include `missing_input`, `invalid_input`, `not_found`, `ambiguous_target`, `permission_denied`, `service_inactive`, `service_failed`, `service_control_failed`, `dependency_error`, `unsupported_action`, `unsupported_platform`, `skill_execution_failed`, and `unknown`.
- `failure_reason` — Non-empty on failure.
- `next_step` — Suggestion when applicable.
- `summary` — Short human-readable summary.

Failure responses must include `failure_reason` and should include stable `error_kind`; when logs were inspected, key evidence is in `key_evidence`. Runner-level failure responses also expose top-level `error_kind` and `platform` so `clawd` can classify failure without parsing OS/systemd text.

## Log Paths (rustclaw only)

- `logs/clawd.log`, `logs/telegramd.log`, `logs/whatsappd.log`, `logs/whatsapp_webd.log`, `logs/feishud.log`, `logs/larkd.log`.

## Error Contract

- Missing or invalid `action` / unknown `target` → clear `failure_reason`.
- **No matching service** (0 candidates after discovery) → `error_kind=not_found`, `failure_reason`, and `next_step` "请提供更具体服务名，或确认该服务已在当前主机安装并可用。" Do not invent or guess service names.
- **Ambiguous match** (>1 candidates) → `error_kind=ambiguous_target`, `failure_reason` "ambiguous: multiple matching services", `next_step` lists candidates for user to choose. Do not execute until exactly one target is specified.
- `clawd` → start/stop/restart return error (main daemon).
- Ambiguous target (vague wording) + stop/restart without `allow_risky` → refuse with `error_kind=ambiguous_target`, `failure_reason`, and `next_step`.
- Manager not implemented for the action → `error_kind=unsupported_platform` or `unsupported_action`, `failure_reason`, and optional `next_step`.
- API 401 (rustclaw) → `error_kind=permission_denied`; suggest RUSTCLAW_UI_KEY.
- **Permission denied**: On systemd/service, if the control command fails due to permission, the skill may retry with `sudo`. Success is returned without mentioning sudo. If sudo also fails, `error_kind=permission_denied`, `failure_reason` is "无法通过 sudo 执行", and `next_step` suggests using a privileged account or configuring passwordless sudo.

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

Response (concept): skill returns `{"request_id":"...","status":"ok","text":"{...structured output...}","error_text":null}` where `text` is the JSON output contract above. On failure, runner response includes top-level `error_kind` and `platform` in addition to the structured JSON in `text`.

## Addendum (2026-03)

- Optional input: `suggest_once` (bool, default true). Legacy `llm_suggest_once` is still accepted.
- Optional input: `suggested_params` (object). Generic suggestion payload for cross-skill reuse.
- `service_control` reads suggestion target from `suggested_params.target` / `service` / `service_name` / `candidate_target`.
- Legacy `llm_suggested_target` is still accepted for compatibility.
- Suggestions are advisory only: the skill still requires discovery to resolve exactly one candidate before execution.
- Permission handling for `systemd`/`service`/`brew_services`: first try normal command; on permission-like failure, retry with `sudo` where the manager supports it.
- If `sudo` succeeds, return success and do not emit sudo-failure messages.
- If `sudo` fails, return failure with clear privileged-account guidance in `next_step`.
