# system_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the system_basic skill implementation.

## Capability Summary
- `system_basic` provides system/runtime introspection plus higher-level read-only query helpers.
- It does **not** replace standalone base skills for raw file, directory, or command operations.
- It is intended for complex composed queries where builtin primitives alone are too low-level for stable planning.

## Actions
- `info`
- `inventory_dir`
- `count_inventory`
- `workspace_glance`
- `tree_summary`
- `dir_compare`
- `extract_field`
- `extract_fields`
- `structured_keys`
- `find_path`
- `read_range`
- `compare_paths`
- `path_batch_facts`
- `diagnose_runtime`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `info` | none | no | - | - | Return host/runtime introspection as JSON: hostname, timestamps, uptime, process RSS, pid, cwd, workspace root, executable path, OS and arch. |
| `inventory_dir` | `path` | no | string(path) | `.` | Target directory inside workspace. |
| `inventory_dir` | `files_only` | no | bool | `false` | Keep only files. |
| `inventory_dir` | `dirs_only` | no | bool | `false` | Keep only directories. |
| `inventory_dir` | `include_hidden` | no | bool | `false` | Include dot-prefixed entries. |
| `inventory_dir` | `names_only` | no | bool | `false` | Return names array and suppress detailed `entries`. |
| `inventory_dir` | `sort_by` | no | string | `name` | `name|mtime_desc|mtime_asc|size_desc|size_asc`. |
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
| `extract_field` | `field_path` | yes | string | - | Dot path like `package.name` or `dependencies.0.name`. |
| `extract_field` | `format` | no | string | auto | `json|toml|yaml`, auto-detected from extension when omitted. |
| `extract_fields` | `path` | yes | string(path) | - | Local JSON/TOML/YAML file path. |
| `extract_fields` | `field_paths` | yes | string[]/string | - | Multiple dot paths to extract in one pass. |
| `extract_fields` | `format` | no | string | auto | `json|toml|yaml`, auto-detected from extension when omitted. |
| `structured_keys` | `path` | yes | string(path) | - | Local JSON/TOML/YAML file path. |
| `structured_keys` | `field_path` | no | string | root | Optional dot path to an object/array inside the parsed document. |
| `structured_keys` | `format` | no | string | auto | `json|toml|yaml`, auto-detected from extension when omitted. |
| `structured_keys` | `max_keys` | no | integer | `200` | Cap for returned object keys preview, clamped to `1..1000`. |
| `find_path` | `root` | no | string(path) | `.` | Search root inside workspace. |
| `find_path` | `name`/`pattern` | yes | string | - | Name or pattern to match. |
| `find_path` | `match_mode` | no | string | `contains` | `contains|exact|starts_with|ends_with`. |
| `find_path` | `target_kind` | no | string | `any` | `any|file|dir`. |
| `find_path` | `max_results` | no | integer | `20` | Output cap, clamped to `1..200`. |
| `read_range` | `path` | yes | string(path) | - | Text file to slice. |
| `read_range` | `mode` | no | string | `head` | `head|tail|range`. |
| `read_range` | `n` | no | integer | `20` | Number of lines for `head`/`tail`, or fallback window for `range`. |
| `read_range` | `start_line` | no | integer | `1` | Range start for `mode=range`. |
| `read_range` | `end_line` | no | integer | auto | Range end for `mode=range`; derived from `n` when omitted. |
| `compare_paths` | `left_path` | yes | string(path) | - | First path to compare. |
| `compare_paths` | `right_path` | yes | string(path) | - | Second path to compare. |
| `path_batch_facts` | `paths` | yes | string[]/string | - | Explicit paths to inspect in batch. |
| `path_batch_facts` | `include_missing` | no | bool | `true` | Keep missing-path records instead of failing on not found. |
| `diagnose_runtime` | `include_process` | no | bool | `false` | Include top process snapshot. |
| `diagnose_runtime` | `include_ports` | no | bool | `false` | Include listening ports snapshot when available. |
| `diagnose_runtime` | `include_env_summary` | no | bool | `false` | Include selected environment summary. |

## Error Contract
- Unsupported actions return a readable error and list allowed actions.
- Workspace paths reject `..` traversal and paths outside workspace.
- `extract_field` / `extract_fields` return explicit parse errors for unsupported/invalid JSON, TOML, or YAML.
- `dir_compare` requires both target paths to be directories and reports summary diffs instead of a full recursive listing.
- `read_range` and `compare_paths` return explicit read/metadata errors for missing or unreadable target paths.
- `tree_summary` intentionally truncates deep/wide trees and reports truncation metadata instead of dumping the full directory.
- Runtime data collection should degrade gracefully where possible (for example, missing `/proc` fields produce fallback values instead of fabricated data).

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"info"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"{\"hostname\":\"host\",\"pid\":1234,\"cwd\":\"/workspace\",\"workspace_root\":\"/workspace\",\"os\":\"linux\",\"arch\":\"x86_64\"}","error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"action":"inventory_dir","path":"document","names_only":true,"include_hidden":false}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"{\"action\":\"inventory_dir\",\"names\":[\"a.txt\",\"b.md\"],\"counts\":{\"total\":2}}","error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"action":"extract_field","path":"Cargo.toml","format":"toml","field_path":"workspace.package.version"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"{\"action\":\"extract_field\",\"exists\":true,\"value_type\":\"string\",\"value_text\":\"0.1.3\"}","error_text":null}
```

### Example 4
Request:
```json
{"request_id":"demo-4","args":{"action":"read_range","path":"configs/config.toml","mode":"range","start_line":1,"end_line":8}}
```
Response:
```json
{"request_id":"demo-4","status":"ok","text":"{\"action\":\"read_range\",\"start_line\":1,\"end_line\":8,\"excerpt\":\"1|[server]\\n2|port = 8080\"}","error_text":null}
```

### Example 5
Request:
```json
{"request_id":"demo-5","args":{"action":"extract_fields","path":"Cargo.toml","format":"toml","field_paths":["workspace.package.version","workspace.members.0"]}}
```
Response:
```json
{"request_id":"demo-5","status":"ok","text":"{\"action\":\"extract_fields\",\"count\":2,\"results\":[{\"field_path\":\"workspace.package.version\",\"exists\":true},{\"field_path\":\"workspace.members.0\",\"exists\":true}]}","error_text":null}
```

### Example 6
Request:
```json
{"request_id":"demo-6","args":{"action":"dir_compare","left_path":"configs","right_path":"docker/config","recursive":false}}
```
Response:
```json
{"request_id":"demo-6","status":"ok","text":"{\"action\":\"dir_compare\",\"counts\":{\"common\":8,\"left_only\":3,\"right_only\":1},\"left_only\":[\"channels\"],\"right_only\":[\"example.toml\"]}","error_text":null}
```
