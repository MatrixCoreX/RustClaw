# git_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the git_basic implementation.

## Capability Summary
- `git_basic` exposes bounded repository inspection plus confirmation-gated
  local stage, commit, branch creation, and safe branch checkout.
- Remote mutations (`push`, `pull`, remote branch deletion, tag publication)
  are not implemented.
- Local mutations use Git argument arrays, never shell command strings.
- Commit hooks and commit signing are disabled for agent-owned commits.
- Checkout requires a clean working tree and never uses force.
- Repository selection is workspace-bound. `repo` may be a workspace-relative
  repository directory or an absolute path whose canonical target remains
  inside `WORKSPACE_ROOT`; it cannot escape `WORKSPACE_ROOT`.
- Revision reads resolve one `target` / `ref` object expression to an exact Git
  object ID before executing the observation. `show` accepts Git's
  `<revision>:<repository-path>` file-object syntax; multi-object ranges are
  rejected.
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
- `show` — show one commit/object (`--stat`), including a file object selected
  with `<revision>:<repository-path>`
- `show_file_at_rev` — show file content at revision (target + path)
- `rev_parse` — rev-parse HEAD
- `stage` — stage only the explicit non-empty `paths` list
- `commit` — commit the current staged set with hooks/signing disabled
- `create_branch` — create a local branch without checkout
- `checkout_branch` — checkout an existing local branch only from a clean tree

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported actions. |
| all | `repo` | no | string | `.` | Workspace-relative repository directory, or a canonical workspace-contained absolute path. |
| `status`, `log`, `branch`, `remote`, `changed_files` | `cursor` | no | integer | 0 | Zero-based observation cursor. |
| `status`, `log`, `branch`, `remote`, `changed_files` | `limit` | no | integer | 20 | Page size, range 1..200. |
| `log` | `n` | no | integer | 20 | Alias for `limit`. |
| `diff`, `diff_cached`, `changed_files` | `path` | no | string | - | Repository-relative pathspec. |
| `show` | `target` | no | string | `HEAD` | Single commit/object expression to show; `<revision>:<repository-path>` is allowed. |
| `show_file_at_rev` | `target` | no | string | `HEAD` | Revision. |
| `show_file_at_rev` | `path` | yes | string | - | Repository-relative file path. |
| `rev_parse` | `ref` | no | string | `HEAD` | Revision expression to resolve. |
| `stage` | `paths` | yes | string[] | - | Explicit repository-relative paths; `.` and implicit add-all are rejected. |
| `commit` | `message` | yes | string | - | Commit message, 1..16384 UTF-8 bytes. |
| `create_branch` | `branch_name` | yes | string | - | Local branch name validated by Git. |
| `create_branch` | `start_point` | no | string | `HEAD` | Revision resolved to an exact object before branch creation. |
| `checkout_branch` | `branch_name` | yes | string | - | Existing local branch; dirty working trees are rejected. |

## Error Contract
- Unsupported action names.
- Not a git repository: `status=error`, `error_text` set.
- Invalid target/revision/path; git command failures return readable stderr.
- Option-like revisions, absolute file/pathspec values, parent traversal,
  repositories outside the workspace, and invalid argument types return stable
  `error_code` values. An absolute `repo` selector is accepted only when its
  canonical target remains inside `WORKSPACE_ROOT`.
- Non-zero `git` command exit codes are returned as `status=error` with `error_text=git command failed: exit=<code>\n<stdout/stderr>`.
- Local-write errors include `git_paths_missing`, `git_paths_invalid`,
  `git_stage_empty`, `git_commit_message_invalid`,
  `git_branch_name_invalid`, `git_branch_not_found`,
  `git_checkout_dirty_worktree`, and `git_unexpected_arg`.
- Successful responses also mirror structured metadata into `extra`, including `schema_version`, `action`, `subcommand`, `exit_code`, `output`, and action-specific machine fields.

## Structured Evidence Contract
- Success `extra` always includes `schema_version`, `action`, `subcommand`,
  `exit_code`, exact `output`, integrity fields, and Git CLI provenance.
- List actions include bounded `page`; observations expose typed
  `field_value`, `changed_files`, `commits`, `branches`, or `remotes`.
- Revision-bound reads include exact `target`, resolved `revision`, and `path`.
- Local-write success includes `status`, `effect=mutate`, `branch`,
  `commit_hash`, `staged_paths`, `changed_paths`, `worktree_state`,
  `hooks_enabled=false`, `signing_enabled=false`, and
  `remote_mutation=false`.
- Sensitive fields: diffs and file-at-revision output can contain source or secrets. Provider-facing traces should prefer file lists, stats, excerpts, or hashes unless content was requested; raw `diff`, `show`, and `show_file_at_rev` output remains conservative.
- Error responses include readable `error_text`; runtime decisions use
  `error_code` / `error_kind`, never parse `error_text`.

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
