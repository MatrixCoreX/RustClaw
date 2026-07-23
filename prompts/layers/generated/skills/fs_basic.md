## fs_basic — planner-facing filesystem tool

Prefer registry leaf capabilities such as `filesystem.write_text`, `filesystem.make_dir`, `filesystem.read_text_range`, and `artifact.read_range` through `{"type":"call_capability","capability":"<leaf>","args":{...}}`. A lower-level `{"type":"call_tool","tool":"fs_basic","args":{"action":"<canonical-action>",...}}` remains available when the plan already owns an exact canonical action. `fs_basic` is a virtual planner tool: runtime maps its actions to stable backing tools such as `system_basic`, `fs_search`, and file builtins.

## Capability
- Inspect explicit path facts.
- List directories with filters and caps.
- Count directory entries with filters.
- Read bounded text ranges from explicit files.
- Resume bounded byte ranges from runtime-owned output artifacts.
- Find filesystem entries by name or extension.
- Search text content under a bounded root.
- Compare explicit paths.
- Write or append text, create directories, or remove files/directories when confirmation permits.
- Apply/review/revert structured workspace patches with exact-context and checkpoint protection.
- Review and decide isolated child-task patches through parent-owned machine actions.

## Actions
- `stat_paths`
- `list_dir`
- `count_entries`
- `read_text_range`
- `read_artifact_range`
- `find_entries`
- `grep_text`
- `compare_paths`
- `write_text`
- `append_text`
- `make_dir`
- `remove_path`
- `apply_patch`
- `diff`
- `rewind`
- `review_child_patch`
- `apply_child_patch`
- `reject_child_patch`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `stat_paths` | `paths` | yes | string/string[] | - | Explicit paths to inspect. |
| `list_dir` | `path` | no | string(path) | `.` | Directory to list. |
| `list_dir` | `files_only` / `dirs_only` | no | bool | `false` | Narrow inventory to files or directories. |
| `list_dir` | `include_hidden` | no | bool | `false` | Include dot-prefixed entries. |
| `list_dir` | `names_only` | no | bool | `false` | Return only entry names when names are sufficient; runtime also exposes `names_by_kind` for grouped file/directory answers. |
| `list_dir` | `sort_by` | no | string | `name` | `name|name_desc|mtime_desc|mtime_asc|size_desc|size_asc`. |
| `list_dir` | `ext_filter` | no | string/string[] | - | Extension filter for files. |
| `list_dir` | `max_entries` | no | integer | impl default | Output cap. |
| `count_entries` | `path` | no | string(path) | `.` | Directory whose direct entries should be counted. |
| `count_entries` | `files_only` / `dirs_only` | no | bool | `false` | Count only files or only directories. |
| `count_entries` | `include_hidden` | no | bool | `false` | Include dot-prefixed entries. |
| `count_entries` | `ext_filter` | no | string/string[] | - | Count matching file extensions. |
| `read_text_range` | `path` | yes | string(path) | - | Text file to slice. |
| `read_text_range` | `mode` | no | string | `head` | `head|tail|range|last_non_empty`; the final mode returns `line_number`, `line_text`, and `exists`. |
| `read_text_range` | `n` / `start_line` / `end_line` | no | integer | action default | Bounded line controls. |
| `read_text_range` | `field_selector` | no | string | - | Use machine token `title` when the requested scalar is the document/markdown heading; runtime returns `field_value` when observed. |
| `read_artifact_range` | `path` | yes | string(path) | - | Runtime-owned file below `.rustclaw/artifacts`; regular workspace files are rejected. |
| `read_artifact_range` | `start_byte` / `cursor` | no | integer | `0` | Exact byte offset returned by an artifact range handle or prior `page.next_cursor`. |
| `read_artifact_range` | `max_bytes` | no | integer | `65536` | Bounded page size, clamped to `256..1048576`; binary pages return base64. |
| `find_entries` | `root` | no | string(path) | workspace | Bounded search root. |
| `find_entries` | `pattern` | conditional | string/string[] | - | Name/basename pattern for name search. |
| `find_entries` | `ext` | conditional | string/string[] | - | Extension selector for extension search. |
| `find_entries` | `target_kind` | no | string | `any` | `any|file|dir`. |
| `grep_text` | `query` | yes | string | - | Text query. |
| `grep_text` | `root` | no | string(path) | workspace | Search root or known file path. |
| `grep_text` | `pattern` | no | string/string[] | - | Optional filename filter. |
| `compare_paths` | `left_path`, `right_path` | yes | string(path) | - | Two explicit paths to compare. |
| `write_text` | `path`, `content` | yes | string(path), string | - | Replace/write text content. Requires confirmation. |
| `append_text` | `path`, `content` | yes | string(path), string | - | Append text content to an existing or new file. Include the requested newline in `content` when the user asks for a line append. Requires confirmation. |
| `make_dir` | `path` | yes | string(path) | - | Create directory. Requires confirmation. |
| `make_dir` | `parents` / `recursive` | no | bool | `true` | Create missing parent directories for mkdir-p style operations. |
| `remove_path` | `path` | yes | string(path) | - | Remove one file. Directory removal requires `target_kind="directory"` and `recursive=true`. Requires confirmation. |
| `apply_patch` | `patch` | yes | string(unified diff) | - | Apply a Git-compatible unified diff. Exact context must match; optional `precondition_hashes` maps paths to `sha256:<hex>` or `missing`. Requires confirmation. |
| `diff` | `checkpoint_id` / `paths` | no | string / string[] | current workspace | Return a checkpoint patch or a bounded current Git diff as machine evidence. |
| `rewind` | `checkpoint_id` | yes | string | - | Restore a patch checkpoint only when target hashes still match the recorded post-patch state. Requires confirmation. |
| `review_child_patch` | `child_task_id` | yes | string | - | Load a terminal child worktree patch after validating parent ownership, artifact hash, base commit, and preconditions. Optional `patch_ref` pins the expected artifact. |
| `apply_child_patch` | `child_task_id` | yes | string | - | Apply the validated child patch through the normal workspace checkpoint path, persist the parent disposition, and clean the isolated worktree. Optional `patch_ref` pins the expected artifact. Requires confirmation. |
| `reject_child_patch` | `child_task_id` | yes | string | - | Persist parent rejection and clean the isolated worktree without changing the primary workspace. Optional `patch_ref` pins the expected artifact. Requires confirmation. |

## Boundaries
- Known explicit path facts: use `stat_paths`, not search.
- Unknown candidate discovery: use `find_entries`, not guessed reads.
- Directories containing matching files: use `find_entries` to discover candidate files, then synthesize unique parent directories from returned paths.
- Directory inventory: use `list_dir`, not `grep_text`.
- File-name inventory is a file-only listing: prefer `filesystem.list_file_names` / `fs_basic.list_dir` with `files_only=true`, `dirs_only=false`, and `names_only=true`. Directory/folder-name inventory is directory-only with `dirs_only=true`, `files_only=false`. Use mixed file+directory inventory only for untyped entries/items/names.
- Grouped file-vs-directory inventory: use `list_dir` and preserve kind metadata (`entries` or `names_by_kind`); do not answer from a flat untyped name list when the contract is grouped.
- Directory counts: use `count_entries`, not `run_cmd` pipelines, unless shell behavior itself is the task.
- Content search or matching-line requests: use `grep_text`, not `read_text_range`. For a known single file, set `root` to that file and `query` to the requested content token, then answer from returned `matches` lines rather than the full file excerpt.
- Raw file excerpts: use `read_text_range`; semantic document understanding belongs to `doc_parse`.
- Truncated tool/skill output: follow its `range_handles` with `artifact.read_range`; never route a runtime artifact through unrestricted file reads or guess byte offsets.
- Last non-empty line of a known file: use `read_text_range` with `mode="last_non_empty"` and answer from observed `line_text`; do not replace this read-only operation with a shell pipeline.
- Document heading/title scalar from a known text/markdown file: use `read_text_range` with `field_selector="title"` and a bounded head read, then answer from observed `field_value` when present.
- File appends: use `append_text`, not `read_text_range` and not `run_cmd` redirection.
- Existing source edits: prefer `apply_patch` over whole-file `write_text`; keep whole-file writes for explicit small replacements or new files.
- Review and recovery: use `diff` and `rewind` checkpoint artifacts; never reconstruct a patch or checkpoint id from final-answer prose.
- Child patch decisions: call `review_child_patch` before `apply_child_patch`; use only observed `child_task_id` and `patch_ref` machine fields. Children never apply or merge directly into the primary workspace.
- Shell semantics, pipelines, or platform-specific commands belong to `run_cmd`.
- Legacy `read_file`, `write_file`, `list_dir`, `make_dir`, `remove_file`, `fs_search`, and `system_basic` remain accepted for compatibility, but prefer `fs_basic` when this contract covers the task.

## Evidence & Final Answer Contract
- For `find_entries`, the returned `results` array is the authoritative candidate set and `count` is the authoritative number of observed candidates.
- If the user asks to find/list candidates and the tool returns multiple `results`, the final answer must include every returned candidate unless the user asked for a top-N subset or the tool result explicitly says it was capped/truncated.
- Do not replace a full `results` array with a sample, examples, "etc.", "and others", or a smaller hand-picked subset.
- If `count` is larger than the number of visible `results`, say that the result is capped/truncated and report only the visible candidates plus the observed count. Do not invent the missing candidates.
- For strict scalar requests, derive the scalar from structured fields such as `exists`, `count`, `size_bytes`, `path`, or `value`; do not paste raw JSON when a scalar was requested.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- 中文里“查一下/看看/列一下/找一下/有没有”要按任务对象映射到上面的结构化 action，不要把这些词做成代码触发词。
- 如果用户给了明确路径，优先按路径事实、目录枚举或范围读取处理；如果路径不明确，先用有界候选搜索或澄清。
- 中文里“有哪些/列出/都有哪些/全部候选”如果对应 `results` 数组，最终回答要完整列出观察到的候选；不要只列几个示例。
### en
- For "find/list/show candidates" requests, treat `results` as the complete observed candidate list unless the result explicitly reports truncation.
