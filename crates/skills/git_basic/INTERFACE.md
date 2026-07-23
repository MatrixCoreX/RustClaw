# git_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the git_basic implementation.

## Capability Summary
- `git_basic` exposes read-oriented Git repository inspection commands.
- It is designed for status/history/diff visibility without destructive history changes.
- Repository selection is workspace-bound. `repo` is a workspace-relative repository directory; it cannot escape `WORKSPACE_ROOT`.
- Revision reads resolve `target` / `ref` to an exact Git object ID before executing the observation.
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
| all | `repo` | no | string | `.` | Workspace-relative repository directory. |
| `status`, `log`, `branch`, `remote`, `changed_files` | `cursor` | no | integer | 0 | Zero-based observation cursor. |
| `status`, `log`, `branch`, `remote`, `changed_files` | `limit` | no | integer | 20 | Page size, range 1..200. |
| `log` | `n` | no | integer | 20 | Alias for `limit`. |
| `diff`, `diff_cached`, `changed_files` | `path` | no | string | - | Repository-relative pathspec. |
| `show` | `target` | no | string | `HEAD` | Commit/object target to show. |
| `show_file_at_rev` | `target` | no | string | `HEAD` | Revision. |
| `show_file_at_rev` | `path` | yes | string | - | Repository-relative file path. |
| `rev_parse` | `ref` | no | string | `HEAD` | Revision expression to resolve. |

## Error Contract
- Unsupported action names.
- Not a git repository: `status=error`, `error_text` set.
- Invalid target/revision/path; git command failures return readable stderr.
- Option-like revisions, absolute paths, parent traversal, repositories outside the workspace, and invalid argument types return stable `error_code` values.
- Non-zero `git` command exit codes are returned as `status=error` with `error_text=git command failed: exit=<code>\n<stdout/stderr>`.
- Successful responses also mirror structured metadata into `extra`, including `schema_version`, `action`, `subcommand`, `exit_code`, `output`, and action-specific machine fields.

## Structured Evidence Contract
- Matrix admission status: built-in structured evidence only; `output` is legacy text evidence unless a stricter parser is explicitly registered.
- Successful response `extra` fields:
  - `schema_version`: number, currently `1`.
  - `action`: string action name; evidence role `status`.
  - `subcommand`: string Git subcommand used; evidence role `field_value`.
  - `exit_code`: integer Git exit code; evidence role `status`.
  - `target`, `revision`, `path`, `cursor`, or `limit`: echoed typed inputs when applicable; evidence roles `field_value` and `path`.
  - `output`: exact Git observation. Large output is preserved for the runtime artifact spill path rather than silently cut inside the skill.
  - `output_bytes`, `output_sha256`, `truncated`: exact observation integrity fields.
  - `provenance`: `source=git_cli`, exact repository root, observed HEAD revision, observation time, and read-only operation class.
  - `page`: stable `cursor`, `limit`, `returned_count`, `total_count`, `has_more`, `next_cursor`, and `previous_cursor` for list actions. `log.total_count` is null because Git history is fetched incrementally.
  - `field_value`: object with stable action-specific evidence:
    - `status`: `branch`, `current_branch`, `upstream`, `ahead`, `behind`, `clean`, `worktree_state`, `changed_count`, `staged_count`, `unstaged_count`, `untracked_count`.
    - `current_branch`: `branch`, `current_branch`.
    - `changed_files`: `changed_count`.
    - `log`: `commit_count`.
    - `rev_parse`: `revision`.
    - `branch`: `branch_count`, `current_branch` when available.
    - `remote`: `remote_count`.
    - `show_file_at_rev`: `source`, `source_kind`, `target`, `revision`, `path`, `content_excerpt`, `content_line_count`, `content_bytes`.
  - Top-level action-specific arrays/objects:
    - `changed_files`: array of changed paths for `status` / `changed_files`.
    - `subject`: first commit subject for exact latest-subject selection; `commits` is an array of `{sha, subject}` for `log`, and `subjects` is a compact string array.
    - `branches`: array of `{name, current}` for `branch`.
    - `remotes`: array of `{name, url, direction}` for `remote`.
    - `show_file_at_rev`: stable `source="git_show_file_at_rev"` and `source_kind="git_revision_file"` so revision-bound content requests can be attributed to Git evidence before filesystem fallback.
- Sensitive fields: diffs and file-at-revision output can contain source or secrets. Provider-facing traces should prefer file lists, stats, excerpts, or hashes unless content was requested; raw `diff`, `show`, and `show_file_at_rev` output remains conservative.
- Error responses include readable `error_text`; runtime decisions must use `error_code` / `error_kind`, never parse `error_text`.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"status"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"exit=0\n## main","extra":{"schema_version":1,"action":"status","subcommand":"status","exit_code":0,"branch":"main","current_branch":"main","clean":true,"worktree_state":"clean","changed_count":0,"changed_files":[],"field_value":{"action":"status","exit_code":0,"branch":"main","current_branch":"main","clean":true,"worktree_state":"clean","changed_count":0},"output":"exit=0\n## main"},"error_text":null}
```
### Example 2 (log with n or limit)
Request:
```json
{"request_id":"demo-2","args":{"action":"log","n":5}}
```
or `{"action":"log","limit":5}` (alias).
