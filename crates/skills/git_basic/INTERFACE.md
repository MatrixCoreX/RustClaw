# git_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the git_basic implementation.

## Capability Summary
- `git_basic` exposes read-oriented Git repository inspection commands.
- It is designed for status/history/diff visibility without destructive history changes.
- Not a git repository: returns `status=error` and `error_text` (no silent ok).

## Actions
- `status` — working tree status (short + branch)
- `log` — oneline log
- `diff` — working tree diff
- `diff_cached` — staged (cached) diff
- `branch` — list all branches
- `current_branch` — current branch name
- `remote` — remote URLs (-v)
- `changed_files` — file names that differ from HEAD
- `show` — show commit/object (--stat)
- `show_file_at_rev` — show file content at revision (target + path)
- `rev_parse` — rev-parse HEAD

Action selection notes:
- Use `current_branch` when the requested output is the single current branch name.
- Use `branch` only when the requested output is the branch list.
- Do not invent plural or helper actions such as `branches`, `list_branches`, or `get_current_branch`; the runtime may normalize some aliases defensively, but planner output should use the declared action names.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported actions. |
| `log` | `n` | no | number | 20 | Number of history entries (capped 100). Alias: `limit`. |
| `show` | `target` | no | string | `HEAD` | Commit/object target to show. |
| `show_file_at_rev` | `target` | no | string | `HEAD` | Revision. |
| `show_file_at_rev` | `path` | yes | string | - | File path in repo. |
| others | none | no | - | - | Use defaults. |

## Error Contract
- Unsupported action names.
- Not a git repository: `status=error`, `error_text` set.
- Invalid target/revision/path; git command failures return readable stderr.
- Non-zero `git` command exit codes are returned as `status=error` with `error_text=git command failed: exit=<code>\n<stdout/stderr>`.
- Successful responses also mirror structured metadata into `extra`, including `action`, `subcommand`, `exit_code`, and `output`.

## Structured Evidence Contract
- Matrix admission status: built-in structured evidence only; `output` is legacy text evidence unless a stricter parser is explicitly registered.
- Successful response `extra` fields:
  - `action`: string action name; evidence role `status`.
  - `subcommand`: string Git subcommand used; evidence role `field_value`.
  - `exit_code`: integer Git exit code; evidence role `status`.
  - `target`, `path`, `n`, or `limit`: echoed typed inputs when applicable; evidence roles `field_value` and `path`.
  - `output`: string bounded Git observation; fallback evidence only.
- Sensitive fields: diffs and file-at-revision output can contain source or secrets. Provider-facing traces should prefer file lists, stats, excerpts, or hashes unless content was requested.
- Error responses include readable `error_text`; top-level `error_kind` should be used when available.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"status"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"exit=0\n## main","extra":{"action":"status","subcommand":"status","exit_code":0,"output":"exit=0\n## main"},"error_text":null}
```
### Example 2 (log with n or limit)
Request:
```json
{"request_id":"demo-2","args":{"action":"log","n":5}}
```
or `{"action":"log","limit":5}` (alias).
