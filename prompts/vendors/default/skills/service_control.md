<!-- AUTO-GENERATED: sync_skill_docs.py -->
## Role & Boundaries
- You are the `service_control` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `crates/skills/service_control/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
- `service_control` performs service lifecycle operations and diagnosis with **structured input/output**.
- **Managers implemented**: `rustclaw` (HTTP to clawd), `systemd`, `service`. Others return "not implemented".
- **Behavior**: Read-only first; high-risk (stop/restart) blocked for ambiguous targets; auto-verify after start/restart/reload; auto logs on failure.
- **Security**: Target validation; no arbitrary shell; RustClaw whitelist for HTTP path; safe unit names for systemd/service.

## Actions (from interface)
- TODO: list supported `action` values.

## Parameter Contract (from interface)
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| TODO | TODO | TODO | TODO | TODO | TODO |

## Error Contract (from interface)
- Missing or invalid `action` / unknown `target` → clear `failure_reason`.
- **No matching service** (0 candidates after discovery) → `failure_reason` and `next_step` "请提供更具体服务名，或确认该服务已在当前主机安装并可用。" Do not invent or guess service names.
- **Ambiguous match** (>1 candidates) → `failure_reason` "ambiguous: multiple matching services", `next_step` lists candidates for user to choose. Do not execute until exactly one target is specified.
- `clawd` → start/stop/restart return error (main daemon).
- Ambiguous target (vague wording) + stop/restart without `allow_risky` → refuse with `failure_reason` and `next_step`.
- Manager not implemented for the action → `failure_reason` and optional `next_step`.
- API 401 (rustclaw) → suggest RUSTCLAW_UI_KEY.
- **Permission denied**: On systemd/service, if the control command fails due to permission, the skill may retry with `sudo`. Success is returned without mentioning sudo. If sudo also fails, `failure_reason` is "无法通过 sudo 执行" and `next_step` suggests using a privileged account or configuring passwordless sudo.

## Request/Response Examples (from interface)
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

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
