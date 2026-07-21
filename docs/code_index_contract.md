# Code Index Contract

`code_index` is the planner-facing repository intelligence boundary. It keeps
an incremental machine index at `.rustclaw/index/repository-v1.json` and
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
| `find_references` | Return parser-observed exact symbol references. |
| `list_tests` | Return parser-observed tests, optionally constrained by path or referenced symbol. |
| `changed_impact` | Connect explicit or Git-observed changed paths to dependent files and tests. |
| `retrieve_context` | Return bounded source snippets selected by structured symbols or paths. |

## Parser And Cache Boundary

- Rust files are parsed with `syn`, including identifiers present inside macro
  token streams.
- Recognized non-Rust source files participate in the file index but do not
  claim symbol accuracy until an established parser adapter is added.
- Unchanged files are reused by size and nanosecond modification fingerprint;
  changed files receive a new SHA-256 and parser pass.
- Symlinks and generated/cache directories are excluded. Index paths are
  always workspace-relative.
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

Errors use machine `error_kind` and structured skill error envelopes. Visible
wording must not be selected by parsing error text.
