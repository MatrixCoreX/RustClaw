# config_guard Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the config_guard implementation.

## Capability Summary
- `config_guard` performs a read-only safety scan of RustClaw TOML configuration.
- For new planner-facing config tasks, prefer `config_basic.guard_rustclaw_config`; `config_guard` remains the runtime backing and compatibility entry.
- It detects risky settings and likely real secrets in selected known locations, returning a compact JSON summary.
- It does **not** patch or write configuration in the current implementation.

## Actions
- No explicit `action` is required. The current implementation always performs the read-only config risk scan.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| scan | `path` | no | string(path) | discovered `configs/config.toml` | Target TOML config file path. Relative paths resolve from the skill process working directory / workspace root. |

The skill currently checks:
- `telegram.bot_token`
- `llm.openai.api_key`, `llm.google.api_key`, `llm.anthropic.api_key`, `llm.grok.api_key`
- `tools.allow_sudo`
- `tools.allow_path_outside_workspace`
- `telegram.sendfile.full_access`

## Error Contract
- Read failures return `read config failed: ...`.
- TOML parse failures return `parse toml failed: ...`.
- The skill does not echo secret values; it only reports that a key appears real.
- Patch/write requests are outside the current implementation and should not be planned as `config_guard` calls.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"path":"configs/config.toml"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"{\"path\":\"configs/config.toml\",\"risk_count\":1,\"risks\":[\"tools.allow_sudo=true\"]}","error_text":null}
```
