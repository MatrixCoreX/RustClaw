# install_module Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the install_module implementation.

## Capability Summary
- `install_module` installs or previews development/runtime modules in common language ecosystems.
- It accepts single or multiple module names and optional ecosystem/version hints.
- Preview requests use `action=preview_install`; they return a structured installation plan and never execute installer commands.
- `action=install` performs the installation. Direct callers may still set `dry_run=true`, but planner-facing preview requests must use the dedicated preview action so policy remains machine-verifiable.

## Actions
- `preview_install`: produce a read-only command and ecosystem preview without installation.
- `install`: execute installation; this is a high-risk mutating action.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| preview_install/install | `modules` or `module` | yes | array/string | - | At least one valid module name. |
| preview_install/install | `ecosystem` | no | string | python | One of `python|node|rust|go` when known. |
| preview_install/install | `version` | no | string | latest | Optional version pin/range hint. |
| preview_install | `dry_run` | no | boolean | true | Forced to true even when a caller supplies false. |
| install | `dry_run` | no | boolean | false | Direct-call preview compatibility; planners should use `preview_install` instead. |

## Error Contract
- Empty module list/name.
- Invalid/unsafe module tokens.
- Unsupported ecosystem value.
- Installation failures return readable command/tool errors.

## Structured Evidence Contract
- Success responses include structured `extra`; downstream runtime must prefer `extra` over parsing user-visible `text`.
- Success `extra` fields:
  - `skill`: string, always `install_module`; evidence role `status`.
  - `action`: string, `preview_install` or `install`; evidence role `status`.
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
{"request_id":"demo-2","args":{"action":"preview_install","modules":["requests"],"ecosystem":"python"}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"skill=install_module\naction=preview_install\necosystem=python\ndry_run=true\ninstaller_available=true\nmodules=requests\nmodule=requests\ncommand_0=python3 -m pip install --user requests","extra":{"skill":"install_module","action":"preview_install","ecosystem":"python","module":"requests","modules":["requests"],"version":null,"dry_run":true,"installer_available":true,"commands":["python3 -m pip install --user requests"],"output":"skill=install_module\naction=preview_install\necosystem=python\ndry_run=true\ninstaller_available=true\nmodules=requests\nmodule=requests\ncommand_0=python3 -m pip install --user requests"},"error_text":null}
```
