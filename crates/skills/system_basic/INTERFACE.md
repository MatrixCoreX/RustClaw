# system_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the system_basic skill implementation.

## Capability Summary
- `system_basic` provides system/runtime introspection plus higher-level read-only query helpers.
- For new planner-facing filesystem tasks, prefer the virtual `fs_basic` contract. For new structured config field/key tasks, prefer the virtual `config_basic` contract. `system_basic` remains the runtime backing and compatibility layer for many read-only helpers.
- It does **not** replace standalone base skills for raw file, directory, or command operations.
- It is intended for complex composed queries where builtin primitives alone are too low-level for stable planning.
- It is not the semantic document parser. For supported local documents that need key-point extraction, section understanding, excerpt judgment, or grounded summarization, prefer `doc_parse` when that skill is enabled; use `read_range` only for exact bounded line slices or raw text previews.
- For directory inventory with filename or extension filtering, use `inventory_dir` with `files_only=true` and `ext_filter`; do not use `extract_field` / `extract_fields` unless the user explicitly asks for fields, keys, or values inside a specific structured document.
- For directory count requests that need separate component counts, use `count_inventory` and make the requested dimensions explicit (`count_files=true`, `count_dirs=true`, or the relevant `kind_filter`/`ext_filter`) so the final answer can preserve each requested dimension instead of returning only `counts.total`.
- When the user asks to list files and then briefly explain their purpose, first collect the file names with `inventory_dir`; the final explanation should be synthesized from the names and known project conventions, not from missing structured fields.
- For recent/latest/last-modified directory inventory, use `inventory_dir` with `sort_by="mtime_desc"` exactly. If the request asks for files, set `files_only=true`; use `max_entries` for the requested count. Do not emit unsupported values such as `mtime`.
- `extract_field` and `extract_fields` operate on exactly one structured file per call: use `path` plus `field_path`/`field_paths`. Do not pass `paths`, `targets`, or other multi-file arrays to these actions; for multiple files, call the action once per file.
- `extract_field` / `extract_fields` first resolve exact dot/bracket paths. If a caller supplies one bare field key and exact root-level lookup misses, the skill may resolve it to a unique nested key in that structured document and report `resolved_field_path`, `match_strategy`, and `match_count`; ambiguous bare keys remain missing instead of guessing.
- When a request asks for a value from the same JSON/TOML/YAML array item or TOML `[[array_table]]` block where another field equals a specified value, encode that relationship in `field_path` with an array filter selector, for example `items[?(@.id=='abc')].status` or `skills.[name=run_cmd].planner_kind`. Do not flatten it to `items.status` / `skills.planner_kind`; that drops the row/block condition.
- For file metadata checks or comparisons, use `compare_paths` for two paths or `path_batch_facts` for multiple explicit paths. Do not model filesystem metadata such as size, modified time, path type, or content equality as `extract_field` / `extract_fields` document fields.

## Actions
- `info`
- `runtime_status`
- `inventory_dir`
- `count_inventory`
- `workspace_glance`
- `tree_summary`
- `dir_compare`
- `extract_field`
- `extract_fields`
- `structured_keys`
- `validate_structured`
- `find_path`
- `read_range`
- `compare_paths`
- `path_batch_facts`
- `diagnose_runtime`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `info` | none | no | - | - | Return host/runtime introspection as JSON: hostname, current_user, kernel_release, timestamps, uptime, process RSS, pid, cwd, workspace root, executable path, OS and arch. |
| `runtime_status` | `kind` | no | string | - | Runtime scalar to return. Supported: `current_user`, `host_name`, `kernel_release`, `current_time`, `current_working_directory`. Aliases such as `whoami`, `hostname`, `uname_r`, `system_time`, and `pwd` are normalized. |
| `inventory_dir` | `path` | no | string(path) | `.` | Target directory inside workspace. |
| `inventory_dir` | `files_only` | no | bool | `false` | Keep only files. |
| `inventory_dir` | `dirs_only` | no | bool | `false` | Keep only directories. |
| `inventory_dir` | `include_hidden` | no | bool | `false` | Include dot-prefixed entries. |
| `inventory_dir` | `names_only` | no | bool | `false` | Return names array and suppress detailed `entries`. |
| `inventory_dir` | `sort_by` | no | string | `name` | `name|name_desc|mtime_desc|mtime_asc|size_desc|size_asc`. |
| `inventory_dir` | `ext_filter` | no | string/string[] | - | File extension filter such as `md` or `[\"md\",\"txt\"]`. |
| `inventory_dir` | `max_entries` | no | integer | `200` | Output cap, clamped to `1..1000`. |
| `count_inventory` | `path` | no | string(path) | `.` | Target directory inside workspace. |
| `count_inventory` | `recursive` | no | bool | `false` | Recurse into subdirectories when true. |
| `count_inventory` | `include_hidden` | no | bool | `false` | Include dot-prefixed entries. |
| `count_inventory` | `kind_filter` | no | string | `any` | `any|file|dir`; narrows which entries are counted. |
| `count_inventory` | `count_files` | no | bool | `true` | Whether file counts should be included. |
| `count_inventory` | `count_dirs` | no | bool | `true` | Whether directory counts should be included. |
| `count_inventory` | `ext_filter` | no | string/string[] | - | File extension filter applied to file counts only. |
| `workspace_glance` | `path` | no | string(path) | `.` | Target directory for a top-level workspace-like summary. |
| `workspace_glance` | `include_hidden` | no | bool | `false` | Include dot-prefixed entries. |
| `workspace_glance` | `max_entries` | no | integer | `20` | Preview cap for returned direct entries, clamped to `1..100`. |
| `tree_summary` | `path` | no | string(path) | `.` | Root path to summarize as a bounded tree. |
| `tree_summary` | `include_hidden` | no | bool | `false` | Include dot-prefixed entries. |
| `tree_summary` | `max_depth` | no | integer | `2` | Max directory depth included in the returned tree, clamped to `1..6`. |
| `tree_summary` | `max_children_per_dir` | no | integer | `12` | Per-directory preview cap, clamped to `1..50`. |
| `tree_summary` | `max_nodes` | no | integer | `200` | Global output cap, clamped to `20..1000`. |
| `dir_compare` | `left_path` | yes | string(path) | - | Left directory to compare. |
| `dir_compare` | `right_path` | yes | string(path) | - | Right directory to compare. |
| `dir_compare` | `recursive` | no | bool | `false` | Compare only direct children by default; recurse when true. |
| `dir_compare` | `include_hidden` | no | bool | `false` | Include dot-prefixed entries. |
| `dir_compare` | `max_diffs` | no | integer | `100` | Preview cap for reported left-only/right-only diffs, clamped to `1..500`. |
| `extract_field` | `path` | yes | string(path) | - | Local JSON/TOML/YAML file path. |
| `extract_field` | `field_path` | yes | string | - | Dot/bracket path like `package.name`, `dependencies.0.name`, `items[0].name`, or `skills[?(@.name=='run_cmd')].planner_kind` for array item lookup by field value. The shorter LLM-friendly selector form `skills.[name=run_cmd].planner_kind` is also accepted. |
| `extract_field` | `format` | no | string | auto | `json|toml|yaml`, auto-detected from extension when omitted. |
| `extract_fields` | `path` | yes | string(path) | - | Local JSON/TOML/YAML file path. |
| `extract_fields` | `field_paths` | yes | string[]/string | - | Multiple dot/bracket paths to extract in one pass; supports the same array index/filter syntax as `field_path`. |
| `extract_fields` | `format` | no | string | auto | `json|toml|yaml`, auto-detected from extension when omitted. |
| `structured_keys` | `path` | yes | string(path) | - | Local JSON/TOML/YAML file path. |
| `structured_keys` | `field_path` | no | string | root | Optional dot path to an object/array inside the parsed document. |
| `structured_keys` | `format` | no | string | auto | `json|toml|yaml`, auto-detected from extension when omitted. |
| `structured_keys` | `max_keys` | no | integer | `200` | Cap for returned object keys preview, clamped to `1..1000`. |
| `validate_structured` | `path` | yes | string(path) | - | Local JSON/TOML/YAML file path to parse. |
| `validate_structured` | `format` | no | string | auto | `json|toml|yaml`, auto-detected from extension when omitted. |
| `find_path` | `root` | no | string(path) | `.` | Search root inside workspace. |
| `find_path` | `name`/`pattern` | yes | string | - | Name or pattern to match. |
| `find_path` | `match_mode` | no | string | `contains` | `contains|exact|starts_with|ends_with`. |
| `find_path` | `target_kind` | no | string | `any` | `any|file|dir`. |
| `find_path` | `max_results` | no | integer | `20` | Output cap, clamped to `1..200`. |
| `read_range` | `path` | yes | string(path) | - | Text file to slice. |
| `read_range` | `mode` | no | string | `head` | `head|tail|range|last_non_empty`; the final mode returns `line_number`, `line_text`, and `exists`. |
| `read_range` | `n` | no | integer | `20` | Number of lines for `head`/`tail`, or fallback window for `range`. |
| `read_range` | `start_line` | no | integer | `1` | Range start for `mode=range`. |
| `read_range` | `end_line` | no | integer | auto | Range end for `mode=range`; derived from `n` when omitted. |
| `read_range` | `field_selector` | no | string | - | Use machine token `title` to project the first markdown/document heading into `field_value`. Prefer the virtual `fs_basic.read_text_range` contract for new plans. |
| `compare_paths` | `left_path` | yes | string(path) | - | First path to compare. |
| `compare_paths` | `right_path` | yes | string(path) | - | Second path to compare. |
| `path_batch_facts` | `paths` | yes | string[]/string | - | Explicit paths to inspect in batch. |
| `path_batch_facts` | `include_missing` | no | bool | `true` | Keep missing-path records instead of failing on not found; missing records use machine-readable `exists=false`, `kind="missing"`, and `error_code="path_not_found"`. |
| `path_batch_facts` | `fields` | no | string[]/string | none | Optional requested metadata field names (for example `exists`, `size`, `kind`, `modified`); echoed back so callers can preserve requested metadata in the final answer. |
| `diagnose_runtime` | `include_process` | no | bool | `false` | Include top process snapshot. |
| `diagnose_runtime` | `include_ports` | no | bool | `false` | Include listening ports snapshot when available. |
| `diagnose_runtime` | `include_env_summary` | no | bool | `false` | Include selected environment summary. |

## Error Contract
- Unsupported actions return a readable error and list allowed actions.
- Paths reject `..` traversal.
- Relative paths resolve under workspace.
- Explicit absolute paths are allowed for these read-only actions and are resolved as provided.
- Error responses include structured `error_kind` and `platform` fields, with `error_text` kept as the human-readable explanation. Callers should use `error_kind` for recovery and routing instead of matching OS-specific error text.
- Common `error_kind` values include `invalid_input`, `path_denied`, `not_found`, `permission_denied`, `not_a_directory`, `is_directory`, `invalid_data`, `unsupported_action`, and `io_error`.
- `extract_field` / `extract_fields` return explicit parse errors for unsupported/invalid JSON, TOML, or YAML.
- `dir_compare` requires both target paths to be directories and reports summary diffs instead of a full recursive listing.
- `read_range` and `compare_paths` return explicit read/metadata errors for missing or unreadable target paths.
- `tree_summary` intentionally truncates deep/wide trees and reports truncation metadata instead of dumping the full directory. Success output includes `summary_rows` mirrored as `results` and `candidates`; each directory row carries machine fields such as `path`, `name`, `file_count`, `dir_count`, `child_count`, `omitted_children`, and `truncated`.
- Runtime data collection should degrade gracefully where possible (for example, missing `/proc` fields produce fallback values instead of fabricated data).
- Successful responses that already use JSON text are also mirrored into the optional `extra` field for machine-readable consumers.

## Structured Evidence Contract
- Matrix admission status: built-in structured evidence only; strict filesystem/config/runtime evidence must come from `extra` fields.
- `info` success `extra` fields:
  - `hostname`, `current_user`, `kernel_release`, `now_ts`, `now_rfc3339`, `pid`, `cwd`, `workspace_root`, `os`, `arch`, timestamps, uptime, RSS, and executable path; evidence roles `field_value`, `count`, and `path`.
- `runtime_status` success `extra` fields:
  - `action`, `kind`, `value`, `field_value`, and `command_output`; evidence roles `field_value` and `command_output`.
- `inventory_dir` success `extra` fields:
  - `action`, `path`, `resolved_path`, `names`, `entries`, `counts`, `size_summary`, and truncation/cap metadata; evidence roles `path`, `entries`, `results`, `field_value`, and `count`.
  - `size_summary` is language-neutral machine evidence with `matched_file_count`, `total_file_size_bytes`, `largest_file`, and `smallest_file`; use it for size comparisons instead of inferring from natural-language listing text.
- `count_inventory` success `extra` fields:
  - `action`, `path`, `resolved_path`, `counts`, filters, and recursion flags; evidence roles `path` and `count`.
- `workspace_glance`, `tree_summary`, and `dir_compare` success `extra` fields:
  - structured path/count/entry/diff fields; evidence roles `path`, `entries`, `results`, `candidates`, and `count`.
- `extract_field` success `extra` fields:
  - `action`, `path`, `field_path`, `exists`, `value_type`, `value_text`, `value`, `resolved_field_path`, `match_strategy`, and `match_count`; evidence roles `field_value`, `status`, and `count`.
- `extract_fields` success `extra` fields:
  - `action`, `path`, `count`, and `results[]` objects with field path, existence, value type, and value fields; evidence roles `results`, `field_value`, and `count`.
- `structured_keys` success `extra` fields:
  - `action`, `path`, `field_path`, `keys`, `count`, and truncation metadata; evidence roles `entries` and `count`.
- `validate_structured` success `extra` fields:
  - `action`, `path`, `valid`, format, and parse details; evidence role `status`.
- `find_path` success `extra` fields:
  - `action`, `root`, `count`, and `results`; evidence roles `path`, `results`, and `count`.
- `read_range` success `extra` fields:
  - `action`, `path`, `start_line`, `end_line`, `total_lines`, `line_count` (stable alias of total file lines), `excerpt`, optional `first_line` when the observed slice includes line 1, optional `field_selector`, optional `field_value` / `value_text` for selector projections; evidence roles `path`, `field_value`, and `count`.
- `compare_paths` success `extra` fields:
  - `left` and `right` structured path fact objects with `exists`, `kind`, `size`, and modified time fields.
  - `comparison` structured comparison fields such as `same_path`, `same_kind`, `same_name`, `same_size`, `size_delta_bytes`, `left_newer`, and `same_content`.
  - `field_value` mirrors the machine comparison facts needed by scalar/verdict contracts, including `same_path`, `left_exists`, and `right_exists`; evidence roles `path`, `status`, and `field_value`.
- `path_batch_facts` success `extra` fields:
  - structured path fact objects with `exists`, `kind`, `size`, modified time, and requested fields; a single result also exposes top-level `basename` (or `null` when missing); missing facts include `error_code="path_not_found"`; evidence roles `path`, `status`, `field_value`, and `count`.
- `diagnose_runtime` success `extra` fields:
  - selected process, port, and environment summary fields; evidence roles `status`, `entries`, and `count`.
- Sensitive fields: excerpts, process snapshots, environment summaries, and structured values may include user data. Provider-facing traces should prefer keys, counts, paths, excerpts, or hashes rather than full values unless the user requested the value.
- Error responses include top-level `error_kind` and `platform`; contextual `extra.error_kind` appears for IO/path errors.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"info"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"{\"hostname\":\"host\",\"current_user\":\"runner\",\"pid\":1234,\"cwd\":\"/workspace\",\"workspace_root\":\"/workspace\",\"os\":\"linux\",\"arch\":\"x86_64\"}","extra":{"hostname":"host","current_user":"runner","pid":1234,"cwd":"/workspace","workspace_root":"/workspace","os":"linux","arch":"x86_64"},"error_text":null}
```

Runtime scalar example:

```json
{"request_id":"demo-1b","args":{"action":"runtime_status","kind":"current_user"}}
```

```json
{"request_id":"demo-1b","status":"ok","text":"{\"action\":\"runtime_status\",\"kind\":\"current_user\",\"value\":\"runner\",\"field_value\":\"runner\",\"command_output\":\"runner\"}","extra":{"action":"runtime_status","kind":"current_user","value":"runner","field_value":"runner","command_output":"runner"},"error_text":null}
```

```json
{"request_id":"demo-1c","args":{"action":"runtime_status","kind":"current_time"}}
```

```json
{"request_id":"demo-1c","status":"ok","text":"{\"action\":\"runtime_status\",\"kind\":\"current_time\",\"value\":\"2026-06-21T08:00:00Z\",\"field_value\":\"2026-06-21T08:00:00Z\",\"command_output\":\"2026-06-21T08:00:00Z\"}","extra":{"action":"runtime_status","kind":"current_time","value":"2026-06-21T08:00:00Z","field_value":"2026-06-21T08:00:00Z","command_output":"2026-06-21T08:00:00Z"},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"action":"inventory_dir","path":"document","names_only":true,"include_hidden":false}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"{\"action\":\"inventory_dir\",\"names\":[\"a.txt\",\"b.md\"],\"counts\":{\"total\":2}}","extra":{"action":"inventory_dir","names":["a.txt","b.md"],"counts":{"total":2}},"error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"action":"extract_field","path":"Cargo.toml","format":"toml","field_path":"workspace.package.version"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"{\"action\":\"extract_field\",\"exists\":true,\"value_type\":\"string\",\"value_text\":\"0.1.3\"}","extra":{"action":"extract_field","exists":true,"value_type":"string","value_text":"0.1.3"},"error_text":null}
```

Array table lookup example:

```json
{"request_id":"demo-3b","args":{"action":"extract_field","path":"configs/skills_registry.toml","format":"toml","field_path":"skills[?(@.name=='run_cmd')].planner_kind"}}
```

```json
{"request_id":"demo-3b","status":"ok","text":"{\"action\":\"extract_field\",\"exists\":true,\"value_type\":\"string\",\"value_text\":\"tool\"}","extra":{"action":"extract_field","exists":true,"value_type":"string","value_text":"tool"},"error_text":null}
```

### Example 4
Request:
```json
{"request_id":"demo-4","args":{"action":"read_range","path":"configs/config.toml","mode":"range","start_line":1,"end_line":8}}
```
Response:
```json
{"request_id":"demo-4","status":"ok","text":"{\"action\":\"read_range\",\"start_line\":1,\"end_line\":8,\"line_count\":12,\"first_line\":\"[server]\",\"excerpt\":\"1|[server]\\n2|port = 8080\"}","extra":{"action":"read_range","start_line":1,"end_line":8,"total_lines":12,"line_count":12,"first_line":"[server]","excerpt":"1|[server]\n2|port = 8080"},"error_text":null}
```

### Example 5
Request:
```json
{"request_id":"demo-5","args":{"action":"extract_fields","path":"Cargo.toml","format":"toml","field_paths":["workspace.package.version","workspace.members.0"]}}
```
Response:
```json
{"request_id":"demo-5","status":"ok","text":"{\"action\":\"extract_fields\",\"count\":2,\"results\":[{\"field_path\":\"workspace.package.version\",\"exists\":true},{\"field_path\":\"workspace.members.0\",\"exists\":true}]}","extra":{"action":"extract_fields","count":2,"results":[{"field_path":"workspace.package.version","exists":true},{"field_path":"workspace.members.0","exists":true}]},"error_text":null}
```

### Example 6
Request:
```json
{"request_id":"demo-6","args":{"action":"dir_compare","left_path":"configs","right_path":"docker/config","recursive":false}}
```
Response:
```json
{"request_id":"demo-6","status":"ok","text":"{\"action\":\"dir_compare\",\"counts\":{\"common\":8,\"left_only\":3,\"right_only\":1},\"left_only\":[\"channels\"],\"right_only\":[\"example.toml\"]}","extra":{"action":"dir_compare","counts":{"common":8,"left_only":3,"right_only":1},"left_only":["channels"],"right_only":["example.toml"]},"error_text":null}
```

### Example 7
Request:
```json
{"request_id":"demo-7","args":{"action":"inventory_dir","path":".","files_only":true,"ext_filter":"toml","names_only":true}}
```
Response:
```json
{"request_id":"demo-7","status":"ok","text":"{\"action\":\"inventory_dir\",\"names\":[\"Cargo.toml\",\"rustfmt.toml\"],\"counts\":{\"total\":2}}","extra":{"action":"inventory_dir","names":["Cargo.toml","rustfmt.toml"],"counts":{"total":2}},"error_text":null}
```

### Error Example
Request:
```json
{"request_id":"demo-error","args":{"action":"read_range","path":"."}}
```
Response:
```json
{"request_id":"demo-error","status":"error","text":"","extra":null,"error_text":"read_range requires a file, but target is a directory: /workspace","error_kind":"is_directory","platform":"linux"}
```
