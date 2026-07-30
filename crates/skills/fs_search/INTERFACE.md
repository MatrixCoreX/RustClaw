# fs_search Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the fs_search implementation.

## Capability Summary
- `fs_search` performs filesystem-level search by name, extension, text, or images.
- For new planner-facing filesystem tasks, prefer the virtual `fs_basic` contract (`find_entries` / `grep_text`). `fs_search` remains the runtime backing and compatibility layer for bounded search actions.
- A known explicit path should bypass recursive discovery through `filesystem.stat_paths` or a bounded range read. Symbol definitions, references, tests, and impact should use `code_index` first; `grep_text` is literal/regex text evidence, not semantic code intelligence.
- It searches a focused root recursively without an implicit depth cap. Output pages and internal deadline/memory safeguards remain bounded.
- File inventory and eligible content queries use a verified ripgrep fast path with typed arguments and bounded structured parsing. Directory discovery, unsupported fast-path cases, and unavailable ripgrep use the same normalized contract through the in-process Rust backend.
- Entry discovery supports stable name or modification-time ordering. Text search enforces per-file and aggregate byte budgets in addition to file and result-count budgets.
- `find_name` can return directory names as well as file names; use `target_kind` to narrow when needed.
- For locating likely filenames, prompt names, module names, or path fragments, use `find_name`.
- `find_ext` may also take a name `pattern`/`patterns` filter when the request asks for files with a specific extension and a filename fragment.
- For discovering which config/docs/skill/prompt files are related to a topic, first search or enumerate candidate filenames/paths (`find_name`, `find_ext`, or directory inventory) before searching inside file contents.
- For searching inside file contents, use `grep_text`.
- `grep_text` never reinterprets a content miss as a filename match. Use `find_name`/`find_entries` when the selector is a path or basename.
- Do not invent alias actions such as `find_text` or `search_text`; unsupported action names fail at runtime.
- Prefer the narrowest known root and exact basename/kind filters. If a result is partial or broad, narrow the next structured request instead of increasing output size blindly.

## Config Entry Points
- `WORKSPACE_ROOT` selects the trusted workspace boundary and defaults to the process current directory.
- `SKILL_TIMEOUT_SECONDS` supplies the runtime deadline; the skill reserves a short response margin.
- Traversal hard ceilings are internal executor safeguards, not user configuration. Reaching one returns structured partial completeness.
- Runtime-provided private SQLite skill storage holds short-lived query/snapshot pages. It is not a user configuration entry point and never grants access to the main runtime database.

## Actions
- `find_name`
- `find_ext`
- `grep_text`
- `find_images`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported search actions. |
| `find_name` | `pattern` / `patterns` (or `name`/`keyword`/`query`) | conditional | string or string[] | - | Basename selector; required unless `glob`/`globs` supplies a path selector. |
| `find_name` | `exact` | no | boolean | `false` | Require an exact basename match instead of substring matching. |
| `find_name` / `find_ext` | `match_mode` | no | string | `contains` | `exact|prefix|suffix|contains|fuzzy|glob`; `fuzzy` tolerates small typos/transpositions and ranks by relevance, while `glob` applies the pattern as a basename glob. |
| `find_name` / `find_ext` / `grep_text` | `glob` / `globs` | no | string or string[] | none | Typed path globs such as `**/*.rs`; parsed as data, never shell flags. |
| `find_name` / `find_ext` / `grep_text` | `case_mode` | no | string | `smart` | `smart|sensitive|insensitive`; smart mode becomes sensitive when a selector contains uppercase characters. |
| `find_name` | `target_kind` | no | string | `any` | `any|file|dir`; narrow name search to files or directories. `files_only=true` and `dirs_only=true` are accepted aliases. |
| `find_name` / `find_ext` | `sort_by` | no | string | `name` | `name|name_desc|mtime_desc|mtime_asc|size_desc|size_asc`; ties are ordered by path. |
| `find_ext` | `ext` (or `extension`) | yes | string or string[] | - | One or more normalized extension selectors (for example `rs` or `["md","txt"]`). |
| `find_ext` | `pattern` / `patterns` (or `name`/`keyword`/`query`) | no | string or string[] | none | Optional basename fragment filter; simple wildcard and alternation patterns are accepted. |
| `grep_text` | `query` | yes | string | - | Text query for content search. |
| `grep_text` | `pattern` / `patterns` (or `name`/`filename`/`file_pattern`) | no | string or string[] | none | Optional filename/basename filter for content search; does not replace `query`. |
| `grep_text` | `file_match_mode` | no | string | `contains` | Match mode for the optional filename selector. |
| `grep_text` | `pattern_kind` | no | string | `literal` | `literal|regex`; regex uses a bounded linear-time engine/backend and is capped at 32 KiB. |
| `grep_text` | `output_mode` | no | string | `content` | `content` returns exact matches, `paths` returns unique matching paths, and `count` returns the aggregate observed match count without snippets. |
| `grep_text` | `multiline` | no | boolean | `false` | Permit the literal or regex selector to cross line boundaries within the configured byte ceilings. |
| `grep_text` | `context_before` / `context_after` | no | integer | `0` | Structured surrounding lines per match, independently clamped to `0..20`. |
| `grep_text` | `max_file_bytes` | no | integer | `8388608` | Maximum bytes read from one file, hard-capped at 64 MiB. |
| `grep_text` | `max_scan_bytes` | no | integer | `67108864` | Aggregate bytes read by one search, hard-capped at 512 MiB. |
| `find_images` | `exts` | no | string[] | common image extensions | Optional image-extension allowlist. |
| `find_images` | `max_dirs` | no | integer | `200` | Cap directory-count summaries to `1..2000`. |
| optional | `root` (or `path`/`dir`) | no | string(path) | workspace | Search root path. |
| optional | `max_results` | no | number | 100 | Page size, clamped to 1..1000. |
| optional | `cursor` | no | opaque string | none | Query- and snapshot-bound cursor returned by the prior page. A numeric `offset` remains a finite compatibility input. |
| optional | `max_depth` | no | number | none | Explicit shallow-scope selector; ordinary searches have no implicit depth cap. |
| optional | `include_hidden` | no | boolean | `false` | Include hidden entries. |
| optional | `respect_ignore` | no | boolean | `true` | Respect `.gitignore`, `.ignore`, and repository ignore rules. Set false only for an explicit trusted request. |
| `grep_text` | `max_line_chars` | no | number | 240 | Cap each matched line snippet length. |

## Error Contract
- Missing required query, unsupported action, invalid/workspace-external root, or runtime failure returns readable `error_text` plus stable `error_kind` when available.
- Invalid, query-mismatched, or out-of-range cursors fail structurally; a changed tree returns `stale_snapshot` plus a `new_snapshot` continuation.
- An opaque next-page cursor reuses a short-lived declared-skill snapshot when valid. TTL/capacity eviction returns `stale_snapshot` with a `new_snapshot` continuation instead of silently rescanning. Changes to the root, result/ancestor stamps, or applicable `.gitignore` / `.ignore` controls also invalidate the snapshot.
- Search never follows directory symlinks; roots are canonicalized inside the task's permitted workspace or authenticated administrator host boundary.
- `completeness` is one of `complete|partial_deadline|partial_hard_limit|partial_permission|stale_snapshot`. A `partial_*` result with zero matches is never authoritative absence.
- Successful JSON is mirrored into `extra`; it contains version/action/root/policy, authoritative page `results`, returned and known counts, completeness, continuation, scan evidence, and opaque query/snapshot cursors.
- `find_name` may return files and directories unless narrowed. `grep_text.matches[]` carries path/line/byte/text/context evidence; `find_images.images[]` carries path/MIME/size/mtime/dimensions.
- Final answers preserve every returned candidate unless the user requested top-N; never replace `results` with examples, `etc.`, inferred entries, or an unmarked sample.

## Structured Evidence Contract
- Matrix admission uses machine `extra`, never natural-language `text` parsing.
- Common evidence: `action` (status), `root`/`workspace_root` (path), `count`/`returned_count`/`known_match_count`/`total_count_is_complete` (count), `results`/`matches` (results/entries/path), and `page`/`completeness`/`continuation`/`snapshot_sha256` (status/provenance).
- Action-specific fields include `exts`, `patterns`, `globs`, `match_mode`, `case_mode`, `target_kind`, image metadata, and grep line/byte/context provenance.
- The trusted runner context may grant `permissions.allow_path_outside_workspace=true`; only then may an explicit absolute `root` search the full host scope visible to the agent-runtime service account. A caller-provided argument cannot grant this permission.
- `scan.backend`, backend version/fallback/elapsed fields, `cache_reused`, `cache_status`, and `observation_bytes` provide diagnostic provenance. They are executor evidence, not planner-selected backend controls.
- Content matches include exact `start_byte`/`end_byte`, `matched_text`, encoding/binary evidence, and a file-identity-bound `range_handle` for a later bounded read.
- Sensitive fields: `matches[].text` may include user data. Provider-facing traces should prefer short excerpts, hashes, line numbers, and paths unless the user requested matched content.

## Request/Response Examples
1. Request: `{"request_id":"demo-1","args":{"action":"find_ext","ext":"rs","root":"crates","max_results":20}}`
   Response excerpt: `{"status":"ok","extra":{"schema_version":2,"action":"find_ext","completeness":"complete","known_match_count":1,"total_count_is_complete":true,"results":["crates/a.rs"]}}`
2. Request: `{"request_id":"demo-2","args":{"action":"find_name","pattern":"photos","target_kind":"dir","exact":true}}`
   Response excerpt: `{"status":"ok","extra":{"completeness":"complete","results":["assets/photos"],"has_more":false}}`
3. Wrong-query cursor request: `{"request_id":"demo-3","args":{"action":"find_name","pattern":"other","cursor":"<opaque>"}}`
   Error excerpt: `{"status":"error","error_kind":"cursor_query_mismatch","error_text":"cursor does not match this query"}`
4. Regex path-only request: `{"request_id":"demo-4","args":{"action":"grep_text","query":"fn\\s+main","pattern_kind":"regex","output_mode":"paths","globs":["**/*.rs"]}}`
   Response excerpt: `{"status":"ok","extra":{"action":"grep_text","output_mode":"paths","results":["src/main.rs"],"matches":[]}}`
