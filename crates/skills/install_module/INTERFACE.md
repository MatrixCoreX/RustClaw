# install_module Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the install_module implementation.

## Capability Summary
- `install_module` installs or previews development/runtime modules in common language ecosystems.
- It accepts single or multiple module names and optional ecosystem/version hints.
- Dry-run requests must set `dry_run=true`; dry-run returns a structured installation plan and does not execute installer commands.

## Actions
- No explicit `action` is required.
- Install behavior is determined by provided module fields and optional ecosystem hints.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| install | `modules` or `module` | yes | array/string | - | At least one valid module name. |
| install | `ecosystem` | no | string | auto | One of `python|node|rust|go` when known. |
| install | `version` | no | string | latest | Optional version pin/range hint. |
| install | `dry_run` | no | boolean | false | When true, return command preview and ecosystem evidence without installing. |

## Error Contract
- Empty module list/name.
- Invalid/unsafe module tokens.
- Unsupported ecosystem value.
- Installation failures return readable command/tool errors.

## Structured Evidence Contract
- Success responses include structured `extra`; downstream runtime must prefer `extra` over parsing user-visible `text`.
- Success `extra` fields:
  - `skill`: string, always `install_module`; evidence role `status`.
  - `action`: string, always `install`; evidence role `status`.
  - `ecosystem`: string selected ecosystem; evidence role `field_value`.
  - `module`: string when exactly one module was requested; evidence role `field_value`.
  - `modules`: string array requested or installed modules; evidence role `entries`.
  - `version`: string or null; evidence role `field_value`.
  - `dry_run`: boolean preview flag; evidence role `status`.
  - `installer_available`: boolean read-only installer availability probe; evidence role `status`.
  - `commands`: string array command previews or executed commands; evidence role `entries`.
  - `output`: string machine-field summary; fallback evidence only.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"modules":["requests"],"ecosystem":"python"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"skill=install_module\naction=install\necosystem=python\ndry_run=false\ninstaller_available=true\nmodules=requests\nmodule=requests\ncommand_0=python3 -m pip install --user requests","extra":{"skill":"install_module","action":"install","ecosystem":"python","module":"requests","modules":["requests"],"version":null,"dry_run":false,"installer_available":true,"commands":["python3 -m pip install --user requests"],"output":"skill=install_module\naction=install\necosystem=python\ndry_run=false\ninstaller_available=true\nmodules=requests\nmodule=requests\ncommand_0=python3 -m pip install --user requests"},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"modules":["requests"],"ecosystem":"python","dry_run":true}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"skill=install_module\naction=install\necosystem=python\ndry_run=true\ninstaller_available=true\nmodules=requests\nmodule=requests\ncommand_0=python3 -m pip install --user requests","extra":{"skill":"install_module","action":"install","ecosystem":"python","module":"requests","modules":["requests"],"version":null,"dry_run":true,"installer_available":true,"commands":["python3 -m pip install --user requests"],"output":"skill=install_module\naction=install\necosystem=python\ndry_run=true\ninstaller_available=true\nmodules=requests\nmodule=requests\ncommand_0=python3 -m pip install --user requests"},"error_text":null}
```
