# kb Interface Spec

## Capability Summary

`kb` is a local namespace-based knowledge retrieval layer for user-managed documents.

The package opts into versioned progress frames. A valid action emits a machine-only
`kb.operation.starting` record before the final response. The host owns localization and
display; the frame is progress/stall evidence only and does not determine success.

Use it when the user wants the agent runtime to:
- build a searchable knowledge base from local files or directories
- add/update indexed materials under a named namespace
- search previously indexed documents by natural-language query
- view which namespaces already exist
- inspect basic namespace/library statistics

Current runtime notes:
- Normalized namespace, document, chunk, FTS/vector retrieval, and revision rows
  live only in the KB-owned SQLite
  database resolved from `database.skill_data_root` (normally
  `data/skills/kb/state.db`).
- `clawd` may query the KB-owned retrieval index during recall, but KB rows are
  not copied into the main runtime database.
- KB data is isolated by `user_key`; it is not tied to a single chat.
- On upgrade, legacy `data/kb/**/*.json` snapshots and legacy main-database
  `kb_doc` rows are migrated once, verified, and removed from their legacy
  location. New writes never use those compatibility paths.

Natural-language intent mapping:
- Requests that semantically mean "add documents to an indexed knowledge base" should use `ingest` when required args are available; examples are illustrative only.
- Requests that semantically mean "retrieve from an indexed knowledge base" should use `search` when the namespace is known or uniquely bound; examples are illustrative only.
- Requests that ask to inspect available knowledge-base namespaces, namespace catalog entries, or global KB counts should use `list_namespaces` or `stats`, not filesystem directory listing. Runtime storage paths are implementation details and are not a substitute for KB actions.
- `kb` is for indexed retrieval over previously ingested local content, not for direct file reading, ad hoc filesystem search, or open-ended chat.

## Actions

- `ingest`: build/update namespace index from local files
  - best for requests like `把 docs/ 建成知识库`、`把这批文档收录到 faq 库`
  - caller should provide an explicit `namespace`
  - if the user names a target folder but not a namespace, prefer a short namespace derived from that folder name only when it is obvious and unambiguous; otherwise ask a concise clarification
- `search`: deterministic hybrid retrieval over the KB-private FTS and local vector projections, with filters applied before candidate limits
  - best for requests like `去 docs 知识库里搜部署步骤`、`在 faq 库里查退款`
  - caller should provide both `namespace` and `query`
  - if the user asks to search a knowledge base but does not identify which namespace to use, ask a concise clarification unless current context already binds exactly one namespace
- `list_namespaces`: inspect which namespaces are already available
  - best for requests like `看看现在有哪些知识库`、`列出所有资料库`
  - does not require `namespace`
  - response JSON includes `namespaces` plus machine fields `names`, `count`, and `namespace_count`
  - each namespace summary includes parser/chunker/embedding versions and the current `active_vector_generation`; user-facing screens keep these maintenance fields behind progressive disclosure
- `list_documents`: list typed document metadata for one namespace
- `remove_documents`: remove selected document paths and their chunks/retrieval rows in one transaction
- `delete_namespace`: delete one owned namespace and all private rows; `deleted=true` plus `cleanup_status=cleaned` is terminal evidence, so do not probe the deleted namespace
- `reindex`: rebuild an existing namespace from its recorded source paths
- `resume_ingest`: continue a persisted ingest/reindex job from its last atomic checkpoint
- `ingest_job_status`: inspect persisted progress without changing the job
- `cancel_ingest`: stop a waiting/running ingest job; already committed documents remain searchable
- `start_vector_reindex`: create a shadow vector generation and process its first bounded batch
- `resume_vector_reindex`: continue one shadow generation from its persisted chunk cursor
- `vector_reindex_status`: inspect generation progress, digests, and the active pointer without mutation
- `cancel_vector_reindex`: discard an unfinished shadow projection without changing the active generation
- `stats`: inspect namespace-level or global KB stats
  - best for requests like `看 docs 知识库的统计`、`看看知识库现在一共有多少库`
  - `namespace` is optional; omitted means global KB stats
  - namespace-scoped response JSON includes top-level `namespace`, `document_count`, and `chunk_count`; nested `stats` also keeps `docs/chunks` compatibility fields
- Returned `text` is a JSON string payload describing the inner skill result; `extra` carries the same machine object.
- Search hits use `RetrievalHitV2`, are always marked `instruction_authority=none`, and expose neutral source references rather than absolute paths.
- Search score components are explicit (`final`, `lexical`, `semantic`, `freshness`, `authority`, `rerank`). Knowledge evidence requires lexical or semantic relevance; freshness is reported as `0` and never admits an otherwise unrelated document.
- Up to three adjacent matching chunks from one document may be returned as one evidence group with ordered `chunk_ids` and start/end offsets. Top-k selection takes the strongest group from each document before filling remaining slots, so one document cannot crowd out all others.

## Parameter Contract

| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `ingest` | `action` | yes | string | - | Must be `ingest`. |
| `ingest` | `namespace` | yes | string | - | Namespace to build/update. |
| `ingest` | `paths` | yes | string[] | - | File or directory paths to index. |
| `ingest` | `path` | no | string | - | Single-path compatibility alias; normalized as `paths: [path]` by the skill. |
| `ingest` | `chunk_size` | no | integer | `1200` | Chunk size for splitting documents. |
| `ingest` | `chunk_overlap` | no | integer | `180` | Overlap between adjacent chunks. |
| `ingest` | `overwrite` | no | bool | `false` | Rebuild namespace from scratch. |
| `ingest` | `file_types` | no | string[] | - | Extension whitelist such as `["md","txt","json"]`. |
| `ingest` | `max_file_size` | no | integer | `2097152` | Skip files larger than this many bytes; hard maximum `67108864`. |
| `ingest`/`reindex` | `max_files_per_run` | no | integer | `500` (`64` constrained) | Per-call file budget, maximum `5000`; incomplete work returns a continuation. |
| `ingest`/`reindex` | `max_bytes_per_run` | no | integer | `67108864` (`16777216` constrained) | Per-call source-byte budget, maximum `268435456`. |
| `ingest`/`reindex` | `max_chunks_per_run` | no | integer | `10000` (`2048` constrained) | Per-call produced-chunk transaction budget and hard maximum. |
| `ingest`/`reindex` | `max_depth` | no | integer | `64` | Directory traversal depth budget; skipped deeper directories are reported. |
| `ingest`/`reindex` | `max_run_seconds` | no | integer | `20` (`10` constrained) | Cooperative wall-clock budget below the runner timeout. |
| `search` | `action` | yes | string | - | Must be `search`. |
| `search` | `namespace` | yes | string | - | Namespace to search. |
| `search` | `query` | yes | string | - | Search query. |
| `search` | `top_k` | no | integer | `5` | Max number of hits. |
| `search` | `filters` | no | object | - | Optional grouped filters object. |
| `search` | `filters.path_prefix` or `path_prefix` | no | string | - | Keep only indexed chunks whose source path starts with this prefix. |
| `search` | `filters.file_type` or `file_type` | no | string | - | Filter by normalized file extension, such as `md` or `json`. |
| `search` | `filters.time_from` or `time_from` | no | integer/string | - | Inclusive lower bound for source file `mtime_epoch`. |
| `search` | `filters.time_to` or `time_to` | no | integer/string | - | Inclusive upper bound for source file `mtime_epoch`. |
| `search` | `min_score` | no | float | `0` | Minimum retrieval score. |
| `list_namespaces` | `action` | yes | string | - | Must be `list_namespaces`. |
| `list_documents` | `namespace` | yes | string | - | Namespace whose documents should be listed. |
| `list_documents` | `limit` | no | integer | `100` | Page size from 1 to 500. |
| `list_documents` | `after_path` | no | string | - | Stable path cursor for the next page. |
| `list_documents` | `continuation.after_path` | no | string | - | Preferred structured cursor returned by the previous page. |
| `remove_documents` | `namespace` | yes | string | - | Namespace to mutate. |
| `remove_documents` | `paths` or `path` | yes | string[]/string | - | Recorded source paths to remove. |
| `delete_namespace` | `namespace` | yes | string | - | Namespace to delete. |
| `reindex` | `namespace` | yes | string | - | Existing namespace to rebuild from source paths. |
| `resume_ingest` | `job_id` | yes | string | - | Persisted job returned by `ingest` or `reindex`. |
| `resume_ingest` | `next_file_index` | yes | nonnegative integer | - | Copy the cursor from the latest continuation; each cursor identifies one bounded batch. |
| `ingest_job_status` | `job_id` | yes | string | - | Job to inspect. |
| `cancel_ingest` | `job_id` | yes | string | - | Job to cancel. |
| `start_vector_reindex` | `namespace` | yes | string | - | Namespace whose canonical chunks should be projected into a new shadow generation. |
| `start_vector_reindex` | `batch_size` | no | integer | `32` (`8` constrained) | Bounded first batch, from 1 to 256 chunks. |
| `resume_vector_reindex` | `namespace` | yes | string | - | Namespace owning the generation. |
| `resume_vector_reindex` | `generation` | yes | positive integer | - | Generation returned by start/status. |
| `resume_vector_reindex` | `batch_size` | no | integer | `32` (`8` constrained) | Bounded continuation batch, from 1 to 256 chunks. |
| `vector_reindex_status` | `namespace` | yes | string | - | Namespace to inspect. |
| `vector_reindex_status` | `generation` | no | positive integer | latest | Exact generation to inspect. |
| `cancel_vector_reindex` | `namespace` | yes | string | - | Namespace owning the unfinished generation. |
| `cancel_vector_reindex` | `generation` | yes | positive integer | - | Unfinished generation to cancel. |
| `stats` | `action` | yes | string | - | Must be `stats`. |
| `stats` | `namespace` | no | string | - | Namespace to inspect; omit for global KB stats. |

- `overwrite=true`: rebuild namespace from scratch
- `overwrite=false`: incremental update by path + full content SHA-256 + parser/chunker version; mtime and size alone never prove identity
- `paths` may be a single string or an array. Prefer `paths[]` in new plans; `path` exists only as a protocol compatibility alias for one source path.
- The host-owned `skill_storage.database_resource_profile` selects standard or constrained defaults. Constrained defaults reduce per-invocation work and persist continuations more frequently; callers may still provide any valid explicit budget, so the profile does not remove supported content or lower correctness.
- Ingestion and vector reindex share one owner-level active-job budget and cannot write the same namespace concurrently. Capacity and concurrency rejection use structured `storage_quota_exceeded` or `storage_concurrency_limit` errors.
- per-doc metadata: `path`, `file_type`, `mtime_epoch`, `size`, `chunks`, `content_sha256`, `parser_version`, `chunker_version`
- per-chunk metadata: `chunk_id`, `offset`, `path`, `file_type`, `mtime_epoch`,
  `text_sha256`, `content_kind`, and typed `location` (`line_start`, `line_end`,
  `section_path`, `page`, `sheet`, or `slide` when available)
- `ingest` chunks each typed extraction block independently, preserving structural
  boundaries and source locations before falling back to bounded overlapping windows.
- KB-owned manifests/checkpoints atomically commit each bounded batch with its next-file cursor, making restart/resume safe.
- Ingestion first produces a versioned `DocumentExtractionResult` with media type,
  parser version, typed text blocks, source locations, and structured warnings.
  The current typed extractors support UTF-8 text/Markdown, normalized JSON,
  per-line validated JSONL/NDJSON, row-preserving CSV/TSV, visible HTML, PDF
  text layers, and DOCX/XLSX/PPTX structure.
  HTML comments plus `script`/`style` content are discarded without executing
  embedded code or loading remote resources; common and numeric entities are
  decoded. PDF extraction uses the shared typed `doc_parse` adapter and requires
  Poppler's `pdfinfo`/`pdftotext`; a missing dependency or malformed source
  returns a stable machine error. Office extraction uses the shared pure-parser
  `office_workspace` adapter, rejects active-content packages, and never executes
  macros or loads external resources. Binary, invalid UTF-8, empty, oversized,
  malformed, or unsupported input is skipped with a stable machine error code.
- Markdown headings, paragraphs, list items, table rows, and fenced code blocks;
  HTML headings, list items, table rows, and code blocks; JSONL records; delimited
  rows; Office paragraphs/rows/slides; and PDF pages are independent typed blocks.
  Chunking never crosses those block boundaries and prefers line/sentence boundaries
  for an oversized block. Scanned PDFs without a text layer are not reported as OCR;
  they return an empty-content machine status so an explicit OCR capability can be
  selected by the agent when appropriate.
- Text input is bounded by line and character limits. PDF input is bounded by source
  bytes, selected page count, and extracted characters. OOXML input is bounded by
  ZIP members, member/expanded bytes, expansion ratio, block/table-row/cell counts,
  and never recursively opens embedded objects.
- Parser or chunker version changes invalidate the active vector generation. Search
  remains lexical/degraded until a resumable replacement generation verifies and is
  atomically activated; vectors from two extraction contracts are never mixed.
- Incomplete work returns `complete=false`, `job_status=waiting`, and a `resume_ingest` continuation with `job_id` and `next_file_index`. Copy both fields for each next batch; do not repeat the previous cursor.
- A stale cursor or concurrent checkpoint change returns structured `error_code=checkpoint_conflict` with the current `job` status and continuation, without committing that batch. Inspect this evidence before retrying. Completed/cancelled jobs remain no-ops.
- Vector reindex keeps the old generation active while bounded batches build a shadow projection. It atomically changes `active_generation` only after namespace revision, source digest, row count, profile, dimensions, and vector payloads verify. A source change marks the shadow generation `stale`; cancellation removes only unfinished projection rows.
- Vector-reindex checkpoints contain only `generation`, `cursor_chunk_id`, `source_digest`, and `projection_digest`; they never persist document text or credentials.
- `search` accepts filter fields either nested under `filters` or as top-level aliases; the nested value is checked first.
- Successful `ingest` responses expose `stats.retrieval_index_synced` and
  `stats.retrieval_index_rows`; they never expose or mutate the main runtime
  database.
- Search merges bounded FTS5 and current-profile local-vector candidates after
  SQL ownership/path/type/time filtering. It reports `stats.retrieval_mode`,
  `corpus_chunks`, `total_candidates`, `retrieval_candidates`,
  `vector_candidate_backend`, and `vector_rows_scored`.
- Filtered vector sets up to 4,096 rows in the standard profile or 1,024 rows
  in the constrained profile use exact cosine ranking. Larger sets use the
  versioned `lsh-hyperplane-v2` ANN projection (eight 12-bit bands) to select at most 4,096 or
  1,024 coarse candidates respectively, followed by exact cosine reranking.
  Search never falls back to scanning an unbounded vector set.
- The ANN table is a disposable projection of the active immutable vector
  generation. Deleting it does not delete canonical documents, chunks, or
  vectors; `start_vector_reindex` rebuilds it and verifies one projection row
  per configured band for every vector before activation.
- Every search hit carries a runtime-generated `evidence_id` bound to its
  source reference, document digest, and namespace revision. Downstream model
  synthesis may select only IDs present in the current evidence map; document
  text cannot mint or authorize evidence IDs.
- If the active vector or ANN projection is absent or stale, search remains read-only
  and returns a structured `maintenance.suggested_action` for
  `start_vector_reindex`; the agent loop may schedule that mutation explicitly.
- The built-in `local-hash-v3` vector projection is deterministic and useful
  for same-token similarity, but it does not claim general cross-language
  semantic understanding. Its tokenizer applies Unicode NFKC normalization,
  preserves first-occurrence order, deduplicates tokens, and records bounded
  projection truncation metadata. A future remote/learned profile must declare
  cross-language capability explicitly before use.
- Non-admin source paths are confined to the canonical workspace root. A
  verified `host_policy_grant` runner context may index absolute paths visible
  to the agent-runtime operating-system user; request arguments cannot grant this.

## Config Entry Points

- The runtime resolves the private database from
  `[database].skill_data_root` in `configs/config.toml`; the default is
  `data/skills`.
- The `kb` registry entry declares
  `storage = { kind = "sqlite", schema_version = 7, migration_owner = "kb" }`.
- `skill-runner` supplies the resolved `context.skill_storage` descriptor.
  Callers and the model must not invent a database path or reuse
  `[database].sqlite_path`.
- Direct protocol tests must pass a descriptor whose `skill_name` is `kb`,
  `storage_kind` is `sqlite`, and `schema_version` is `7`.
- No third-party account, API key, or external database is required.

## Error Contract

- Return explicit error when `namespace`, `paths`, or `query` is missing for the selected action.
- If namespace is missing during `search`, return explicit error rather than empty success.
- Indexing and retrieval failures must be surfaced with readable error text.

## Request/Response Examples

### Example 1

Request:
```json
{
  "request_id": "kb-1",
  "args": {
    "action": "search",
    "namespace": "docs",
    "query": "deployment steps",
    "top_k": 3
  }
}
```

Response:
```json
{
  "request_id": "kb-1",
  "status": "ok",
  "text": "{\"status\":\"ok\",\"hits\":[{\"schema_version\":2,\"source_ref\":\"kb://docs/0123456789abcdef/docs-1\",\"namespace\":\"docs\",\"namespace_revision\":3,\"document_id\":\"0123456789abcdef\",\"document_digest\":\"0123456789abcdef...\",\"chunk_id\":\"docs-1\",\"location\":{\"path\":\"README.md\",\"offset\":0},\"score\":{\"final\":0.91,\"lexical\":0.95,\"semantic\":0.86,\"rerank\":0.91},\"retrieval_modes\":[\"lexical\",\"semantic\"],\"instruction_authority\":\"none\",\"text\":\"...\"}],\"summary\":\"1 hit\",\"stats\":{\"top_k\":3}}",
  "error_text": null
}
```

- Retrieval scores expose stable lexical, semantic, and final components in the range 0 to 1.
- Results are fully traceable to source file/chunk metadata.
- If namespace is missing, returns explicit error.

### Example 2

Request:
```json
{
  "request_id": "kb-2",
  "args": {
    "action": "ingest",
    "namespace": "docs",
    "paths": ["docs", "README.md"],
    "file_types": ["md", "txt"],
    "overwrite": false
  }
}
```

Response:
```json
{
  "request_id": "kb-2",
  "status": "ok",
  "text": "{\"status\":\"ok\",\"namespace\":\"docs\",\"indexed_files\":12,\"updated_files\":3,\"skipped_files\":1,\"summary\":\"docs namespace updated\"}",
  "error_text": null
}
```

### Example 3

Natural-language mapping examples:
- `把 docs/ 建成知识库，命名为 docs` -> `{"action":"ingest","namespace":"docs","paths":["docs/"]}`
- `在 docs 知识库里搜索 telegram 按钮` -> `{"action":"search","namespace":"docs","query":"telegram 按钮"}`
- `列出现在所有知识库` -> `{"action":"list_namespaces"}`
- `看 docs 知识库统计` -> `{"action":"stats","namespace":"docs"}`
- `帮我去知识库里查部署步骤` -> if namespace is not uniquely known, ask a concise clarification for the namespace instead of guessing
