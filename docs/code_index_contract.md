# Code Index Contract

`code_index` is the planner-facing repository intelligence boundary. It keeps
an incremental machine index at `.agent-runtime/index/repository-v1.json` and
returns structured definitions, references, tests, changed-file impact, and
bounded source ranges.

## Ownership

- The model decides when repository intelligence is relevant and supplies
  explicit `symbol`, `symbols`, `path`, or `paths` fields.
- Runtime validates those machine fields, confines paths to the workspace,
  refreshes the index, and returns parser-derived evidence.
- Runtime must not inspect task prose to infer symbols, paths, search mode, or
  the requested answer.
- Ordinary user-visible explanations are synthesized by the model from the
  generic capability result.

## Actions

| Action | Purpose |
| --- | --- |
| `refresh` | Incrementally refresh source-file fingerprints and parser data. |
| `search_symbols` | Search symbol names with explicit `exact`, `prefix`, or `contains` mode. |
| `find_definitions` | Return exact symbol definitions and source range handles. |
| `find_references` | Return exact-name syntactic reference candidates, with lexical scopes. |
| `list_tests` | Return parser-observed tests, optionally constrained by path or referenced symbol. |
| `changed_impact` | Connect explicit or Git-observed changed paths to dependent files and tests. |
| `retrieve_context` | Return bounded source snippets selected by structured symbols or paths. |

## Parser And Cache Boundary

- Rust files are parsed with `syn`, including identifiers present inside macro
  token streams.
- Python, JavaScript/JSX, and TypeScript/TSX use pinned upstream Tree-sitter
  grammars. Definitions include named functions, classes, methods, variables,
  import aliases, and parameters; TypeScript also includes interfaces and types.
  Python test functions and JavaScript test-call declarations are discoverable.
  References carry lexical `scope` and `resolution=syntactic_candidate`; neither
  same-name references nor changed-file impact claim language-server resolution.
- Other recognized languages remain file-only. Syntax errors report
  `parsed_with_errors`; parse/traversal budget exhaustion reports an explicit
  partial status. No code is imported or executed to build the index.
- `summary.complete` concerns the source-file scan, as declared by
  `completeness_scope=source_file_scan`; `symbol_coverage_complete` separately
  reports whether every scanned file has full parser coverage. Missing candidates
  in a partial index are not proof that a symbol does not exist.
- AST input is bounded to 2 MiB per source. Larger non-Rust files are hashed in
  chunks without building a syntax tree; large Rust files retain the explicit
  range-index fallback. Each Tree-sitter parse and traversal has a 250ms budget
  and a 50,000-entry ceiling; callers can use source-range tools for omitted work.
- Large Rust fallback hashes the complete file once through the same descriptor.
  It retains at most 24 KiB per line, 2 MiB of symbol-name bytes and 50,000
  candidates. Oversized/invalid UTF-8 lines are skipped as whole lines; subsequent
  line numbers remain accurate and `range_index_budget_exceeded` discloses the
  partial result. It never treats a retained line prefix as a complete symbol.
- Unchanged files are reused by size and nanosecond modification fingerprint;
  changed files receive a new SHA-256 and parser pass. Every retained cached
  path is re-admitted through the current host filesystem policy before reuse,
  including entries outside the current incremental scan page. Historical
  parser data never grants current access to its source.
- Symlinks and generated/cache directories are excluded. Index paths are
  always workspace-relative. Source/cache access and cache publication use
  descriptor-relative no-follow filesystem operations on Linux and macOS.
- Protected directories are excluded without disabling public queries.
  `summary.policy_excluded_entries`, `cache_rejected_files` and `skipped_files`
  disclose exclusion/rejection checks, not counts of distinct source files.
  Excluded or unreadable scan entries make `scan_complete=false`; an empty
  result from that partial scope is not proof of repository-wide absence.
  Explicit context paths and protected cache access are refused rather than
  read through a cached definition. Observe-only queries leave the cache file
  unchanged; refresh publishes through one admitted parent directory.
- Cache parser revisions invalidate older parser results without changing the
  public schema. Cache reads are bounded to 128 MiB; observe-only queries do not
  persist a rebuilt index. Only explicit `refresh` writes the cache.
- Every source location includes a
  `filesystem.read_text_range` machine handle. Callers should request a
  narrower follow-up range instead of loading broad files.
- Implicit Git change discovery uses a read-only, network-denied process with
  the same host private-directory exclusions and shared explicit environment.
  Query execution uses an ephemeral repository with a detached HEAD and bounded
  copies of the index/shared index and local ignore/attribute data. Only validated
  declarative core settings are projected; repository/global executable configuration,
  clean/process filters, hooks, fsmonitor, external diff and text conversion are
  not loaded. Object data remains read-only, including linked-worktree stores.
  `changed_paths_semantics=without_executable_repository_config` is conservative:
  raw files can differ from their filter-normalized blobs without a user edit.
  NUL-delimited paths preserve newlines, whitespace and literal backslashes;
  non-UTF-8/unrepresentable paths produce incomplete evidence, not lossy aliases.
  Protected cached Git filenames are filtered; `data.policy_excluded_paths`,
  `git_query_failures` and `changed_paths_complete` distinguish partial discovery
  from a verified empty change set. An unavailable sandbox never falls back to
  direct execution. Git does not spawn nested status commands. Populated
  submodules use the same isolated observer, with at most 32 repositories,
  8 nested levels and 128 MiB of copied metadata per query. Index/shared-index
  copies stream with a 64 MiB per-file cap; local ignore/attribute files are
  capped at 1 MiB. Gitlink HEAD changes and inspected nested file changes remain
  observable. Uninitialized/cyclic/budget-limited submodules remain explicit:
  `git_submodule_worktrees_uninspected>0` makes
  `changed_paths_complete=false`. This preparation boundary does not establish complete
  process-owner/cancellation/lifetime admission or activate authority storage.

## Result Boundary

All actions return JSON with:

- `schema_version`
- `kind`
- `action`
- `status_code`
- index/refresh `summary`
- action-specific `data`

Errors use machine `error_code` and structured skill error envelopes. Visible
wording must not be selected by parsing error text.

Parser sources: [Tree-sitter Rust API](https://docs.rs/tree-sitter/0.25.10/tree_sitter/),
[Python grammar](https://github.com/tree-sitter/tree-sitter-python),
[JavaScript grammar](https://github.com/tree-sitter/tree-sitter-javascript), and
[TypeScript/TSX grammars](https://github.com/tree-sitter/tree-sitter-typescript).
