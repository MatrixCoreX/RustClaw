# package_manager Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the package_manager implementation.

## Capability Summary
- `package_manager` detects available system package managers, detects project/workspace package or build tools from manifest/lock files, and installs packages with optional dry-run/sudo controls.
- It supports direct manager-specific install and smart auto-detection install.
- Detection is platform-aware: macOS prefers Homebrew first, while Linux prefers the native distro managers before Homebrew fallback. When `detect.path` points at a project directory, project markers such as `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`, `Cargo.toml`, or `Cargo.lock` take precedence. Successful responses include `extra.platform`, `extra.available`, `extra.version_present`, and optional `extra.version`; `detect` also includes `extra.candidate_order`, and project detection includes `extra.manager_scope="project"` plus `extra.marker`.

## Actions
- `detect`
- `install`
- `smart_install`
- `uninstall`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of `detect|install|smart_install`. |
| `detect` | `path` / `root` / `project_path` / `workspace` | no | string(path) | - | Optional project directory or manifest path. If supplied, detect project package/build tool from marker files before falling back to system manager. |
| `install`/`smart_install`/`uninstall` | `packages` or `package` | yes | array/string | - | Non-empty package list. Prefer these canonical fields. Structured compatibility aliases `modules` and `module` are accepted but should not be emitted by new planners. |
| `install` | `manager` | no | string | auto | Explicit package manager override. |
| `install`/`smart_install`/`uninstall` | `dry_run` | no | boolean | impl default | Preview package operation without changes. |
| `install`/`smart_install`/`uninstall` | `use_sudo` | no | boolean | impl default | Use elevated package operation when needed. |

## Error Contract
- Missing or empty package list.
- Unsupported manager/action values.
- Install command failures return readable stderr/system errors.
- Non-zero install command exit codes are returned as `status=error` with `error_text=package install failed: exit=<code>\n<stdout/stderr>`.
- Successful responses also mirror structured metadata into `extra`, including `action`, `manager`, `platform`, `packages`, and `output`.

## Structured Evidence Contract
- Runtime evidence source: package manager results must come from structured `extra`, not from natural-language `text`.
- For an ordinary detection request, use `result_kind="none"`,
  `requires_content_evidence=true`, and model synthesis from the structured
  detection result.
- Only when the user explicitly asks for the exact manager token, use
  `result_kind="none"`, `response_shape="scalar"`, and
  `structured_field_selector="manager"`.
- `detect` success `extra` fields:
  - `action`: string, always `detect`; evidence role `status`.
  - `manager`: string detected manager; evidence role `field_value`.
  - `available`: boolean, true when a non-`unknown` manager was detected; evidence role `status`.
  - `version_present`: boolean, true when a bounded read-only version probe returned a version line; evidence role `status`.
  - `version`: string or null, first non-empty version line when available; evidence role `field_value`.
  - `platform`: string OS/platform; evidence role `field_value`.
  - `candidate_order`: string array candidate managers; evidence role `entries`.
  - `manager_scope`: string such as `project` or `system` when present; evidence role `field_value`.
  - `marker`: string project marker filename when present; evidence role `path`.
  - `output`: string observation summary; fallback evidence only.
- `install`, `smart_install`, and `uninstall` success `extra` fields:
  - `action`: string action name; evidence role `status`.
  - `manager`: string selected manager; evidence role `field_value`.
  - `platform`: string OS/platform; evidence role `field_value`.
  - `package`: string when exactly one package was requested; evidence role `field_value`.
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
{"request_id":"demo-1","status":"ok","text":"action=smart_install\nmanager=apt-get\ndry_run=true\npackages=jq\npackage=jq\ncommand=apt-get install -y jq","extra":{"action":"smart_install","manager":"apt-get","platform":"linux","package":"jq","packages":["jq"],"dry_run":true,"command":"apt-get install -y jq","output":"action=smart_install\nmanager=apt-get\ndry_run=true\npackages=jq\npackage=jq\ncommand=apt-get install -y jq"},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"action":"detect"}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"manager=apt-get available=true version_present=true","extra":{"action":"detect","manager":"apt-get","manager_scope":"system","available":true,"version_present":true,"version":"apt 2.7.14 (amd64)","platform":"linux","candidate_order":["apt-get","apt","dnf","yum","pacman","apk","zypper","brew"],"output":"manager=apt-get available=true version_present=true"},"error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"action":"uninstall","package":"jq","dry_run":true}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"action=uninstall\nmanager=apt-get\ndry_run=true\npackages=jq\npackage=jq\ncommand=apt-get remove -y jq","extra":{"action":"uninstall","manager":"apt-get","platform":"linux","package":"jq","packages":["jq"],"dry_run":true,"command":"apt-get remove -y jq","output":"action=uninstall\nmanager=apt-get\ndry_run=true\npackages=jq\npackage=jq\ncommand=apt-get remove -y jq"},"error_text":null}
```
