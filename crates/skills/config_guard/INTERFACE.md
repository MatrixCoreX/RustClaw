# config_guard Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the config_guard implementation.

## Capability Summary
- `config_guard` provides controlled config read/validate/patch operations.
- It is designed for minimal, key-scoped config changes with safety checks.

## Actions
- Read/validate/patch style config operations (exact action names depend on implementation runtime).

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| reads | `path` | yes | string(path) | - | Target config file path. |
| validates | `path` | yes | string(path) | - | Target config file path. |
| writes/patches | `path` | yes | string(path) | - | Target config file path. |
| writes/patches | key path field | yes | string | - | Explicit key to patch. |
| writes/patches | value field | yes | any | - | Intended value for target key. |

## Error Contract
- Missing target path/key/value for write operations.
- Invalid path/key/value shape and parse failures.
- Safety violations (over-broad whole-file rewrite) should return explicit errors.
- Secret fields in outputs should be redacted.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"path":"configs/config.toml","key":"skills.skill_switches.crypto","value":true}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"config patch applied","error_text":null}
```
