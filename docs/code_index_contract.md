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
  changed files receive a new SHA-256 and parser pass.
- Symlinks and generated/cache directories are excluded. Index paths are
  always workspace-relative. Source/cache access and cache publication use
  descriptor-relative no-follow filesystem operations on Linux and macOS.
- Cache parser revisions invalidate older parser results without changing the
  public schema. Cache reads are bounded to 128 MiB; observe-only queries do not
  persist a rebuilt index. Only explicit `refresh` writes the cache.
- Every source location includes a
  `filesystem.read_text_range` machine handle. Callers should request a
  narrower follow-up range instead of loading broad files.

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
