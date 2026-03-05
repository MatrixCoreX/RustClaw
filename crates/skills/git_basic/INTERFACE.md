# git_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the git_basic implementation.

## Capability Summary
- `git_basic` exposes read-oriented Git repository inspection commands.
- It is designed for status/history/diff visibility without destructive history changes.

## Actions
- `status`
- `log`
- `diff`
- `branch`
- `show`
- `rev_parse`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported actions. |
| `show` | `target` | no | string | `HEAD` | Commit/object target to show. |
| `log` | `n` | no | number | impl default | Number of history entries. |
| others | none | no | - | - | Use defaults for repository-root view. |

## Error Contract
- Unsupported action names.
- Invalid target/revision arguments.
- Git command failures should return readable stderr.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"status"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"On branch ...","error_text":null}
```
