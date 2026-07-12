## fs_basic — planner-facing filesystem tool

Use `{"type":"call_tool","tool":"fs_basic","args":{...}}` for filesystem tasks that match the structured actions below. `fs_basic` is a virtual planner tool: runtime maps its actions to stable backing tools such as `system_basic`, `fs_search`, and file builtins.

## Capability
- Inspect explicit path facts.
- List directories with filters and caps.
- Count directory entries with filters.
- Read bounded text ranges from explicit files.
- Find filesystem entries by name or extension.
- Search text content under a bounded root.
- Compare explicit paths.
- Write or append text, create directories, or remove files/directories when confirmation permits.

## Actions
- `stat_paths`
- `list_dir`
- `count_entries`
- `read_text_range`
- `find_entries`
- `grep_text`
- `compare_paths`
- `write_text`
- `append_text`
- `make_dir`
- `remove_path`

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
| `read_text_range` | `mode` | no | string | `head` | `head|tail|range`. |
| `read_text_range` | `n` / `start_line` / `end_line` | no | integer | action default | Bounded line controls. |
| `read_text_range` | `field_selector` | no | string | - | Use machine token `title` when the requested scalar is the document/markdown heading; runtime returns `field_value` when observed. |
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

## Boundaries
- Known explicit path facts: use `stat_paths`, not search.
- Unknown candidate discovery: use `find_entries`, not guessed reads.
- Directories containing matching files: use `find_entries` to discover candidate files, then synthesize unique parent directories from returned paths.
- Directory inventory: use `list_dir`, not `grep_text`.
- Grouped file-vs-directory inventory: use `list_dir` and preserve kind metadata (`entries` or `names_by_kind`); do not answer from a flat untyped name list when the contract is grouped.
- Directory counts: use `count_entries`, not `run_cmd` pipelines, unless shell behavior itself is the task.
- Content search or matching-line requests: use `grep_text`, not `read_text_range`. For a known single file, set `root` to that file and `query` to the requested content token, then answer from returned `matches` lines rather than the full file excerpt.
- Raw file excerpts: use `read_text_range`; semantic document understanding belongs to `doc_parse`.
- Document heading/title scalar from a known text/markdown file: use `read_text_range` with `field_selector="title"` and a bounded head read, then answer from observed `field_value` when present.
- File appends: use `append_text`, not `read_text_range` and not `run_cmd` redirection.
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
