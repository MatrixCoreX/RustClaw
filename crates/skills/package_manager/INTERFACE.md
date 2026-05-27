# package_manager Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the package_manager implementation.

## Capability Summary
- `package_manager` detects available system package managers, detects project/workspace package or build tools from manifest/lock files, and installs packages with optional dry-run/sudo controls.
- It supports direct manager-specific install and smart auto-detection install.
- Detection is platform-aware: macOS prefers Homebrew first, while Linux prefers the native distro managers before Homebrew fallback. When `detect.path` points at a project directory, project markers such as `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`, `Cargo.toml`, or `Cargo.lock` take precedence. Successful responses include `extra.platform`; `detect` also includes `extra.candidate_order`, and project detection includes `extra.manager_scope="project"` plus `extra.marker`.

## Actions
- `detect`
- `install`
- `smart_install`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of `detect|install|smart_install`. |
| `detect` | `path` / `root` / `project_path` / `workspace` | no | string(path) | - | Optional project directory or manifest path. If supplied, detect project package/build tool from marker files before falling back to system manager. |
| `install`/`smart_install` | `packages` or `package` | yes | array/string | - | Non-empty package list. |
| `install` | `manager` | no | string | auto | Explicit package manager override. |
| `install`/`smart_install` | `dry_run` | no | boolean | impl default | Preview install without changes. |
| `install`/`smart_install` | `use_sudo` | no | boolean | impl default | Use elevated install when needed. |

## Error Contract
- Missing or empty package list.
- Unsupported manager/action values.
- Install command failures return readable stderr/system errors.
- Non-zero install command exit codes are returned as `status=error` with `error_text=package install failed: exit=<code>\n<stdout/stderr>`.
- Successful responses also mirror structured metadata into `extra`, including `action`, `manager`, `platform`, `packages`, and `output`.

## Structured Evidence Contract
- Matrix admission status: built-in structured evidence only; package manager selection must come from `extra`, not from natural-language `text`.
- `detect` success `extra` fields:
  - `action`: string, always `detect`; evidence role `status`.
  - `manager`: string detected manager; evidence role `field_value`.
  - `platform`: string OS/platform; evidence role `field_value`.
  - `candidate_order`: string array candidate managers; evidence role `entries`.
  - `manager_scope`: string such as `project` or `system` when present; evidence role `field_value`.
  - `marker`: string project marker filename when present; evidence role `path`.
  - `output`: string observation summary; fallback evidence only.
- `install` and `smart_install` success `extra` fields:
  - `action`: string action name; evidence role `status`.
  - `manager`: string selected manager; evidence role `field_value`.
  - `platform`: string OS/platform; evidence role `field_value`.
  - `packages`: string array requested packages; evidence role `entries`.
  - `dry_run`: boolean preview flag; evidence role `status`.
  - `command`: string command preview/executed command; evidence role `field_value`.
  - `output`: string bounded install observation; fallback evidence only.
- Sensitive fields: package names and command strings are usually low sensitivity, but provider-facing traces should still avoid full stderr dumps unless needed.
- Error responses include readable `error_text`; top-level or `extra.error_kind` should be preferred over matching error text when present.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"smart_install","packages":["jq"],"dry_run":true}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"dry_run=1 command: apt-get install -y jq","extra":{"action":"smart_install","manager":"apt-get","platform":"linux","packages":["jq"],"dry_run":true,"command":"apt-get install -y jq","output":"dry_run=1 command: apt-get install -y jq"},"error_text":null}
```
