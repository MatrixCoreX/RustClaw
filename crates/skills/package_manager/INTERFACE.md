# package_manager Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the package_manager implementation.

## Capability Summary
- `package_manager` detects available package managers and installs packages with optional dry-run/sudo controls.
- It supports direct manager-specific install and smart auto-detection install.

## Actions
- `detect`
- `install`
- `smart_install`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of `detect|install|smart_install`. |
| `detect` | none | no | - | - | Detect package manager and environment support. |
| `install`/`smart_install` | `packages` or `package` | yes | array/string | - | Non-empty package list. |
| `install` | `manager` | no | string | auto | Explicit package manager override. |
| `install`/`smart_install` | `dry_run` | no | boolean | impl default | Preview install without changes. |
| `install`/`smart_install` | `use_sudo` | no | boolean | impl default | Use elevated install when needed. |

## Error Contract
- Missing or empty package list.
- Unsupported manager/action values.
- Install command failures return readable stderr/system errors.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"smart_install","packages":["jq"],"dry_run":true}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"dry-run install plan: ...","error_text":null}
```
