# config_edit

## Capability Summary

`config_edit` is the structured config mutation tool. Use it when the user asks to change a RustClaw configuration value, enable or disable a skill switch, switch a selected model/vendor, update a bounded config field, validate the result, read the value back, or report whether restart is needed.

Use `config_basic` for read-only config queries. Use `config_edit` for config mutations. After `apply_config_change`, prefer `config_edit.read_back` for the edited field so the mutation proof stays in the same structured workflow.

The default `path` is `configs/config.toml` when the user does not specify a config file. For module-specific configs, inspect or infer the real config entry point first, then pass that file explicitly, for example `configs/audio.toml`.

Do not use natural-language phrase matching in code. The LLM should map user intent to a structured field mutation, and this tool enforces the structured contract.

## Config Entry Points

- Main RustClaw config: `configs/config.toml`.
- Audio/STT config: `configs/audio.toml`.
- Other module configs: inspect current registry/interface docs or config files first, then pass the concrete config file path.
- Environment variables and secrets are not edited by this tool.

## Actions

### `plan_config_change`

Preview one config field change without writing.

Required:
- `field_path`: dot path inside the config file.
- `value`: JSON typed target value.

Optional:
- `path`: config file path, default `configs/config.toml`.
- `format`: `toml` or `json`, default inferred from path.
- `operation`: currently only `set`.

### `apply_config_change`

Apply one structured config field change.

Required:
- `field_path`: dot path inside the config file.
- `value`: JSON typed target value.

Optional:
- `path`: config file path, default `configs/config.toml`.
- `format`: `toml` or `json`, default inferred from path.
- `operation`: currently only `set`.

This action mutates files and requires runtime confirmation.

### `validate_config`

Validate the config file syntax after a change.

Optional:
- `path`: config file path, default `configs/config.toml`.
- `format`: `toml` or `json`, default inferred from path.

### `guard_config`

Run a structured RustClaw config risk guard. It reports known risky fields such as real-looking secrets, sudo/path policy flags, and full-access file delivery flags.

Optional:
- `path`: config file path, default `configs/config.toml`.
- `format`: `toml` or `json`, default inferred from path.

### `read_back`

Read one config field after mutation to prove the resulting value.

Required:
- `field_path`: dot path inside the config file.

Optional:
- `path`: config file path, default `configs/config.toml`.
- `format`: `toml` or `json`, default inferred from path.

### `restart_if_requested`

Report restart status/recommendation. This first version does not restart services by itself. If `restart=true`, it returns a structured handoff telling the planner to use an approved restart workflow.

Optional:
- `restart`: boolean, default `false`.
- `reason`: short reason.

## Common Field Examples

- Enable a skill: `skills.skill_switches.photo_organize = true`
- Disable a skill: `skills.skill_switches.photo_organize = false`
- Select global model vendor: `llm.selected_vendor = "mimo"`
- Select global model: `llm.selected_model = "mimo-v2.5-pro"`
- Switch STT vendor in `configs/audio.toml`: `audio_transcribe.default_vendor = "custom"`
- Switch STT model in `configs/audio.toml`: `audio_transcribe.default_model = "local-whisper"`

These examples are config field examples, not language-trigger rules.

## Parameter Contract

| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `plan_config_change` | `field_path` | yes | string | none | Dot-path field to preview. |
| `plan_config_change` | `value` | yes | JSON value | none | Target typed value. |
| `plan_config_change` | `path` | no | string | `configs/config.toml` | Config file path. |
| `plan_config_change` | `format` | no | `toml` or `json` | inferred | Structured file format. |
| `plan_config_change` | `operation` | no | string | `set` | Only `set` is supported. |
| `apply_config_change` | `field_path` | yes | string | none | Dot-path field to change. |
| `apply_config_change` | `value` | yes | JSON value | none | Target typed value. |
| `apply_config_change` | `path` | no | string | `configs/config.toml` | Config file path. |
| `apply_config_change` | `format` | no | `toml` or `json` | inferred | Structured file format. |
| `apply_config_change` | `operation` | no | string | `set` | Only `set` is supported. |
| `validate_config` | `path` | no | string | `configs/config.toml` | Config file path. |
| `validate_config` | `format` | no | `toml` or `json` | inferred | Structured file format. |
| `guard_config` | `path` | no | string | `configs/config.toml` | Config file path. |
| `guard_config` | `format` | no | `toml` or `json` | inferred | Structured file format. |
| `read_back` | `field_path` | yes | string | none | Dot-path field to read. |
| `read_back` | `path` | no | string | `configs/config.toml` | Config file path. |
| `read_back` | `format` | no | `toml` or `json` | inferred | Structured file format. |
| `restart_if_requested` | `restart` | no | boolean | `false` | Whether restart was requested. |
| `restart_if_requested` | `reason` | no | string | `config changed` | Restart recommendation reason. |

## Request/Response Examples

### Request: Preview enabling a skill


```json
{"request_id":"demo-plan","user_id":1,"chat_id":1,"context":{"permissions":{"allow_path_outside_workspace":false}},"args":{"action":"plan_config_change","path":"configs/config.toml","field_path":"skills.skill_switches.photo_organize","value":true}}
```

### Request: Apply a model vendor change

```json
{"request_id":"demo-apply","user_id":1,"chat_id":1,"context":{"permissions":{"allow_path_outside_workspace":false}},"args":{"action":"apply_config_change","path":"configs/config.toml","field_path":"llm.selected_vendor","value":"mimo"}}
```

### Request: Validate and read back

```json
{"request_id":"demo-read-back","user_id":1,"chat_id":1,"context":{"permissions":{"allow_path_outside_workspace":false}},"args":{"action":"read_back","path":"configs/config.toml","field_path":"llm.selected_vendor"}}
```

### Response: Plan change

```json
{"request_id":"demo-plan","status":"ok","text":"{\"action\":\"plan_config_change\",\"path\":\"configs/config.toml\",\"field_path\":\"skills.skill_switches.photo_organize\",\"would_change\":true}","extra":{"action":"plan_config_change","path":"configs/config.toml","field_path":"skills.skill_switches.photo_organize","would_change":true},"error_text":null}
```

### Response: Apply change

```json
{"request_id":"demo-apply","status":"ok","text":"{\"action\":\"apply_config_change\",\"applied\":true,\"validated\":true}","extra":{"action":"apply_config_change","applied":true,"validated":true},"error_text":null}
```

### Response: Error

```json
{"request_id":"demo-error","status":"error","text":"","extra":{"operation":"read_config","path":"/workspace/configs/missing.toml"},"error_text":"read_config failed for /workspace/configs/missing.toml: No such file or directory (os error 2)","error_kind":"not_found","platform":"linux"}
```

## Error Contract

- `invalid_input`: missing required fields, unsupported operation, unsupported format.
- `invalid_data`: config parse/serialization failure.
- `path_denied`: path traversal or outside-workspace path.
- `not_found`: config file missing.
- `permission_denied`: OS permission failure.
- `unsupported_action`: unknown action.

## Structured Evidence Contract

- Matrix admission status: built-in structured evidence only; config mutation proof must come from `extra`, not natural-language `text`.
- `plan_config_change` success `extra` fields:
  - `action`: string, always `plan_config_change`; evidence role `status`.
  - `path`: string config path; evidence role `path`.
  - `field_path`: string structured field path; evidence role `field_value`.
  - `would_change`: boolean preview result; evidence role `status`.
- `apply_config_change` success `extra` fields:
  - `action`: string, always `apply_config_change`; evidence role `status`.
  - `applied`: boolean mutation result; evidence role `status`.
  - `validated`: boolean post-write syntax validation result; evidence role `status`.
  - `path`, `field_path`, and typed target value fields when present; evidence roles `path` and `field_value`.
- `validate_config` and `guard_config` success `extra` fields:
  - `action`: string action name; evidence role `status`.
  - `valid` or risk/count fields when present; evidence roles `status` and `count`.
- `read_back` success `extra` fields:
  - `action`: string, always `read_back`; evidence role `status`.
  - `path`: string config path; evidence role `path`.
  - `field_path`: string structured field path; evidence role `field_value`.
  - `value`, `value_type`, or `value_text`: observed typed value; evidence role `field_value`.
- Sensitive fields: secret-like values are redacted. Provider-facing traces should keep redaction and prefer field paths plus typed summaries.
- Error responses include top-level `error_kind` and contextual `extra` such as `operation` and `path` when available.

## Safety Notes

- `apply_config_change` mutates files and must go through runtime confirmation.
- Secret-like fields are redacted in output.
- Prefer `plan_config_change -> apply_config_change -> validate_config -> guard_config/read_back`.
- After `apply_config_change`, use this skill's `read_back` action for the edited field before the final answer.
- Keep final answers concise but include the execution process when reporting config changes.
