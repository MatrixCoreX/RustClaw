# fs_search Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the fs_search implementation.

## Capability Summary
- `fs_search` performs filesystem-level search by name, extension, text, or images.
- For new planner-facing filesystem tasks, prefer the virtual `fs_basic` contract (`find_entries` / `grep_text`). `fs_search` remains the runtime backing and compatibility layer for bounded search actions.
- It is intended for bounded queries with optional root scoping and result caps.
- `find_name` can return directory names as well as file names; use `target_kind` to narrow when needed.
- For locating likely filenames, prompt names, module names, or path fragments, use `find_name`.
- `find_ext` may also take a name `pattern`/`patterns` filter when the request asks for files with a specific extension and a filename fragment.
- For discovering which config/docs/skill/prompt files are related to a topic, first search or enumerate candidate filenames/paths (`find_name`, `find_ext`, or directory inventory) before searching inside file contents.
- For searching inside file contents, use `grep_text`.
- Do not invent alias actions such as `find_text` or `search_text`; unsupported action names fail at runtime.

## Config Entry Points
- Optional environment variables:
  - `RUSTCLAW_FS_SEARCH_MAX_DEPTH`: default traversal depth for this skill.
  - `RUSTCLAW_FS_SEARCH_MAX_FILES`: default scanned-file cap for this skill.
- If those are unset, `fs_search` may read locator scan env values as a lower-bound compatibility source, but it keeps deeper defaults suitable for explicit search tasks.

## Actions
- `find_name`
- `find_ext`
- `grep_text`
- `find_images`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported search actions. |
| `find_name` | `pattern` / `patterns` (or `name`/`keyword`/`query`) | yes | string or string[] | - | Name pattern/keyword; matches basename contains; simple wildcard and alternation patterns are accepted. |
| `find_name` | `exact` | no | boolean | `false` | Require an exact basename match instead of substring matching. |
| `find_name` | `target_kind` | no | string | `any` | `any|file|dir`; narrow name search to files or directories. `files_only=true` and `dirs_only=true` are accepted aliases. |
| `find_ext` | `ext` (or `extension`) | yes | string | - | Extension selector (e.g. `rs`). |
| `find_ext` | `pattern` / `patterns` (or `name`/`keyword`/`query`) | no | string or string[] | none | Optional basename fragment filter; simple wildcard and alternation patterns are accepted. |
| `grep_text` | `query` | yes | string | - | Text query for content search. |
| `grep_text` | `pattern` / `patterns` (or `name`/`filename`/`file_pattern`) | no | string or string[] | none | Optional filename/basename filter for content search; does not replace `query`. |
| optional | `root` (or `path`/`dir`) | no | string(path) | workspace | Search root path. |
| optional | `max_results` | no | number | impl default | Cap result volume. |
| optional | `max_depth` | no | number | env/default | Traversal depth cap. |
| optional | `max_files` | no | number | env/default | Scanned-file cap. |
| `grep_text` | `max_line_chars` | no | number | 240 | Cap each matched line snippet length. |

## Error Contract
- Missing required query key for selected action.
- Invalid root path.
- Unsupported action names.
- Search runtime errors return readable filesystem/tool errors.
- `find_name` may return both files and directories unless `target_kind` is provided.
- Successful responses are returned as JSON text with stable top-level fields like `action`, `root`, `workspace_root`, `count`, and `results`.
- For `find_name` / `find_ext` / candidate discovery, `results` is the authoritative observed candidate list and `count` is the authoritative observed count.
- If the caller asks to list or report candidates, the final answer should include every returned `results` item unless the user requested a top-N subset or the result is explicitly capped/truncated.
- Do not replace a returned `results` array with only examples, a smaller sample, `etc.`, or inferred candidates.
- `grep_text` also returns `patterns`, `match_count`, and `matches` items with `path`, `line`, and `text` so callers can answer content-check questions without reading whole files.
- Successful responses also mirror that parsed JSON into the optional `extra` field for machine-readable consumers.

## Structured Evidence Contract
- Matrix admission status: built-in structured evidence only; strict search evidence must come from `extra.results`, `extra.matches`, and count fields.
- `find_name`, `find_ext`, and `find_images` success `extra` fields:
  - `action`: string action name; evidence role `status`.
  - `root`: workspace-relative bounded search root; evidence role `path`.
  - `workspace_root`: absolute canonical workspace root used to resolve relative result paths; evidence role `path`.
  - `count`: integer number of returned results; evidence role `count`.
  - `results`: string array candidate paths; evidence roles `results`, `entries`, and `path`.
  - `ext`, `patterns`, `target_kind`, and cap fields when present; evidence role `field_value`.
- `grep_text` success `extra` fields:
  - `action`: string, always `grep_text`; evidence role `status`.
  - `root`: workspace-relative search root; evidence role `path`.
  - `workspace_root`: absolute canonical workspace root used to resolve relative match paths; evidence role `path`.
  - `patterns`: string array filename filters; evidence role `entries`.
  - `match_count`: integer match count; evidence role `count`.
  - `matches`: array of objects with `path`, `line`, and `text`; evidence roles `results`, `path`, `table_cell`, and `field_value`.
- Sensitive fields: `matches[].text` may include user data. Provider-facing traces should prefer short excerpts, hashes, line numbers, and paths unless the user requested matched content.
- Error responses include readable `error_text`; top-level `error_kind` should be used when available.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"find_ext","ext":"rs","root":"crates","max_results":20}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"{\"action\":\"find_ext\",\"root\":\"crates\",\"workspace_root\":\"/workspace/rustclaw\",\"ext\":\"rs\",\"count\":20,\"results\":[\"crates/a.rs\"]}","extra":{"action":"find_ext","root":"crates","workspace_root":"/workspace/rustclaw","ext":"rs","count":20,"results":["crates/a.rs"]},"error_text":null}
```
