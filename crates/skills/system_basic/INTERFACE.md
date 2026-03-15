# system_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the system_basic skill implementation.

## Capability Summary
- `system_basic` provides basic workspace/system operations, including info, directory listing, and file-level read/write/delete helpers.
- Read operations are preferred; mutating operations should be used only with explicit user intent.

## Actions
- `info`
- `list_dir`
- `make_dir`
- `read_file`
- `write_file`
- `remove_file`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `info` | none | no | - | - | Return basic host/runtime info. |
| `list_dir` | `path` | yes | string(path) | - | List directory entries. |
| `make_dir` | `path` | yes | string(path) | - | Create directory path. |
| `read_file` | `path` | yes | string(path) | - | Read file content. |
| `write_file` | `path` | yes | string(path) | - | Target file path. |
| `write_file` | `content` | yes | string | - | File content to write. |
| `remove_file` | `path` | yes | string(path) | - | Remove target file. |

## Error Contract
- Missing required fields for path/content operations.
- Invalid/non-existent paths should return readable filesystem errors.
- Permission failures should return explicit error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"read_file","path":"README.md"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"...file content...","error_text":null}
```
