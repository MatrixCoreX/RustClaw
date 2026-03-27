<!-- AUTO-GENERATED: sync_skill_docs.py -->
## Role & Boundaries
- You are the `archive_basic` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `crates/skills/archive_basic/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
- `archive_basic` provides archive operations for listing archive contents, packing files/folders into an archive, and unpacking archives into a destination directory.
- Supported archive types are `zip` and `tar.gz`/`tgz`.
- Relative paths resolve from `WORKSPACE_ROOT`; explicit absolute paths are also allowed when the user already supplied them exactly.
- All input paths reject `..` traversal.

## Actions (from interface)
- `list`: list entries in an archive file.
- `pack`: create an archive from a source path.
- `unpack`: extract an archive into a destination directory.

## Parameter Contract (from interface)
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `list` | `action` | yes | string | - | Must be `list`. |
| `list` | `archive` | yes | string(path) | - | Archive file path (relative to workspace or explicit absolute path). |
| `pack` | `action` | yes | string | - | Must be `pack`. |
| `pack` | `source` | yes | string(path) | - | Source file or directory to archive. |
| `pack` | `archive` | yes | string(path) | - | Output archive file path. Parent dir is auto-created. |
| `pack` | `format` | no | string | `zip` | Supported: `zip`, `tar.gz`, `tgz` (`tgz` handled as `tar.gz`). |
| `unpack` | `action` | yes | string | - | Must be `unpack`. |
| `unpack` | `archive` | yes | string(path) | - | Input archive file path. |
| `unpack` | `dest` | yes | string(path) | - | Extraction destination directory (auto-created; relative to workspace or explicit absolute path). |

## Error Contract (from interface)
- Input/shape errors:
  - `args must be object`
  - `<key> is required` (for missing required string args, e.g. `archive is required`)
- Action/format errors:
  - `unsupported action; use list|pack|unpack`
  - `unsupported format; use zip|tar.gz`
  - `unsupported archive format for list`
  - `unsupported archive format for unpack`
- Path safety errors:
  - `path with '..' is not allowed`
- Runtime/system errors:
  - `mkdir failed: <error>`
  - `run <bin> failed: <error>`
  - `archive command failed: exit=<code>\n<stdout/stderr>`
  - On malformed stdin JSON request: `invalid input: <serde error>`
- Successful command execution output is returned in `text` as `exit=<code>\n<stdout/stderr>`.
- Non-zero archive command exit codes are returned as `status=error` with `error_text=archive command failed: exit=<code>\n<stdout/stderr>`.
- Successful responses also mirror structured metadata into `extra`, including `action`, relevant paths, and `output`.

## Request/Response Examples (from interface)
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"list","archive":"tmp/sample.zip"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"exit=0\nArchive: ...","extra":{"action":"list","archive":"/workspace/tmp/sample.zip","output":"exit=0\nArchive: ..."},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"action":"pack","source":"tmp/data","archive":"tmp/data.zip","format":"zip"}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"exit=0\n  adding: ...","extra":{"action":"pack","format":"zip","source":"/workspace/tmp/data","archive":"/workspace/tmp/data.zip","output":"exit=0\n  adding: ..."},"error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"action":"unpack","archive":"tmp/data.tgz","dest":"tmp/out"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"exit=0\n...","extra":{"action":"unpack","archive":"/workspace/tmp/data.tgz","dest":"/workspace/tmp/out","output":"exit=0\n..."},"error_text":null}
```

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
