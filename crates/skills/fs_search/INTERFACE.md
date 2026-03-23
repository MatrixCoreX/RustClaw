# fs_search Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the fs_search implementation.

## Capability Summary
- `fs_search` performs filesystem-level search by name, extension, text, or images.
- It is intended for bounded queries with optional root scoping and result caps.
- `find_name` can return directory names as well as file names; use `target_kind` to narrow when needed.

## Actions
- `find_name`
- `find_ext`
- `grep_text`
- `find_images`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported search actions. |
| `find_name` | `pattern` (or `name`/`keyword`) | yes | string | - | Name pattern/keyword; matches basename contains. |
| `find_name` | `target_kind` | no | string | `any` | `any|file|dir`; narrow name search to files or directories. |
| `find_ext` | `ext` (or `extension`) | yes | string | - | Extension selector (e.g. `rs`). |
| `grep_text` | `query` | yes | string | - | Text/regex query for content search. |
| optional | `root` | no | string(path) | workspace | Search root path. |
| optional | `max_results` | no | number | impl default | Cap result volume. |

## Error Contract
- Missing required query key for selected action.
- Invalid root path.
- Unsupported action names.
- Search runtime errors return readable filesystem/tool errors.
- `find_name` may return both files and directories unless `target_kind` is provided.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"find_ext","ext":"rs","root":"crates","max_results":20}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"crates/...","error_text":null}
```
