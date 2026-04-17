# install_module Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the install_module implementation.

## Capability Summary
- `install_module` installs development/runtime modules in common ecosystems.
- It accepts single or multiple module names and optional ecosystem/version hints.

## Actions
- No explicit `action` is required.
- Install behavior is determined by provided module fields and optional ecosystem hints.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| install | `modules` or `module` | yes | array/string | - | At least one valid module name. |
| install | `ecosystem` | no | string | auto | One of `python|node|rust|go` when known. |
| install | `version` | no | string | latest | Optional version pin/range hint. |

## Error Contract
- Empty module list/name.
- Invalid/unsafe module tokens.
- Unsupported ecosystem value.
- Installation failures return readable command/tool errors.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"modules":["requests"],"ecosystem":"python"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"installed modules: requests","error_text":null}
```
