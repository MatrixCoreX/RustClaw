# install_module Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the install_module implementation.

## Capability Summary

- Installs or previews language dependencies without implicit host-global writes.
- `scope=project` updates a detected project manifest/lockfile or a project-local isolated Python dependency directory.
- `scope=tool_cache` installs a standalone tool into the runtime's versioned `data/tool-cache/modules` tree.
- Preview returns exact scope, argv, working directory, target files and the confirmation requirement without creating directories or running an installer.

## Actions

- `preview_install`: read-only installation plan; always forces `dry_run=true`.
- `install`: confirmed mutating installation with bounded command output and a structured operation receipt.

## Parameter Contract

| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `modules` or `module` | yes | array/string | - | One or more ecosystem-valid package names. |
| all | `ecosystem` | no | string | python | `python`, `node`, `rust`, or `go` (registry aliases are accepted). |
| all | `version` | no | string | latest | Optional version selector. |
| all | `scope` | no | enum | auto | `project` or `tool_cache`; auto selects project only when its ecosystem manifest exists. |
| all | `project_path` | no | string | `.` | Project directory; non-admin requests remain workspace-confined and verified admin context may select an external directory. |
| preview_install | `dry_run` | no | boolean | true | Always true even if the caller supplies false. |
| install | `dry_run` | no | boolean | false | Compatibility preview; planners should call `preview_install`. |

## Scope And Platform Contract

- Project Node installs use local `npm install --save`; Rust uses `cargo add`; Go uses `go get`.
- Python uses `uv add` or `poetry add` when the corresponding lockfile exists, otherwise an isolated project `.agent-runtime/dependencies/python` target.
- Tool-cache Python uses `pip --target`, Node uses `npm --prefix`, Rust uses `cargo install --root`, and Go uses a cache-specific `GOBIN`.
- Ordinary module installation never emits Python `--user`, npm `-g`, or an unscoped Cargo/Go install. Native OS packages belong to `package_manager`.

## Structured Evidence Contract

Success `extra` includes `action`, `ecosystem`, `scope`, `modules`, `version`, `dry_run`, `would_write`, `installer_available`, `commands`, `command_argv`, `working_directories`, `target_files`, and `output`. Preview also includes `confirmation_required_for_install=true`. Actual installation includes bounded `output_results` and `operation_receipt.artifacts` with SDK digests for resulting files.

Errors use `extra.error_code` plus `message_key`, `retryable`, and readable `error_text`.

## Request/Response Examples

Project preview request:

```json
{"request_id":"demo-1","args":{"action":"preview_install","module":"typescript","ecosystem":"node","scope":"project","project_path":"."}}
```

Project preview response shape:

```json
{"request_id":"demo-1","status":"ok","text":"skill=install_module\naction=preview_install\necosystem=node\nscope=project\ndry_run=true","extra":{"action":"preview_install","ecosystem":"node","scope":"project","dry_run":true,"would_write":false,"confirmation_required_for_install":true,"command_argv":[["npm","install","--save","typescript"]],"target_files":["/workspace/package.json","/workspace/package-lock.json"]},"error_text":null}
```

Tool-cache preview request:

```json
{"request_id":"demo-2","args":{"action":"preview_install","module":"ripgrep","ecosystem":"rust","scope":"tool_cache","version":"14.1.1"}}
```

The returned command uses `cargo install --root <workspace>/data/tool-cache/modules/rust/ripgrep/14.1.1`, and preview does not create that directory.
