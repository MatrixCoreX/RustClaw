# Bounded Views and Resource Safety

The adaptive-limit inventory is a review ratchet, not a request to remove every
bound. Agent task budgets, result completeness, display previews, input
validation, and storage quotas have different owners. A bound must not silently
turn incomplete evidence into a complete answer.

## Reviewed Boundaries

| Owner | Boundary | Meaning and recovery | Verification |
| --- | --- | --- | --- |
| Teaching UI | `TaskEvidenceList` page slice | Display page only; every row remains reachable and exact references remain inspectable. | `UI/src/lib/task-evidence-list.test.tsx`, `scripts/browser_tests/task_evidence.mjs` |
| KB UI | `kb-maintenance` string projection | Bounds untrusted view fields; does not modify stored documents or search evidence. Full document management uses owner-scoped skill APIs. Invalid overlong authentication input cannot acquire another identity. | `UI/src/lib/kb-maintenance.test.ts` |
| AiAPP read views | `bounded_aipp_text` and sanitized activity preview | List/detail preview, not planner evidence or the canonical task result. Task reports and skill-owned exports remain separate. | `http/ui_routes/aipp_tests.rs`: bounded-copy, report, secret-removal and cursor tests |
| KB provenance | 16-character document digest prefix | Readable source identifier only. The full document digest and chunk identity remain in structured evidence; this prefix is not an integrity or authorization check. | `memory/retrieval` and KB provenance tests |
| Model logs | 16 MiB compressed-record decoding ceiling | Prevents decompression expansion; checks declared length and SHA-256. Larger original records remain uncompressed at encode time. Logs never drive execution or historical capability recall. | `providers/model_log_payload_tests.rs` |
| Resume scheduler | Candidate batch and per-pass scan clamp | Limits work per scheduler pass, not total task work. A rotating scan cursor advances beyond rejected prefixes and wraps after shrink/restart. Durable claims prevent duplicate execution. | `repo/tasks_resume_scan_fairness_tests.rs` |
| Code index | Cache and source byte ceilings | Rejects an oversized cache instead of allocating it; rebuilds from source. Large sources expose explicit partial status and bounded range handles. A file-scan summary does not claim complete symbol coverage. | `skills/builtin_code_index_tests.rs` |
| Code parser | Per-file AST entry/time ceilings | Produces `parse_budget_exceeded` or `index_budget_exceeded`, not a complete semantic index. References remain scope-labelled syntactic candidates. Narrow the source/path or use a bounded file range. | `skills/builtin_code_index/language_parsers_tests.rs` |
| Canonical task evidence | Entry count, object bytes and aggregate bytes | Publishes a digest-bound reference only after all objects and the manifest are written. Over quota or unavailable storage is explicit; a completed operation is not re-executed or falsely marked failed because archival failed. Reads verify size/digest and load only selected objects. | `task_context_archive_tests.rs`, `http/task_context_pagination_tests.rs` |
| Large JSON output | 8 KiB structured preview | Retains non-sensitive top-level scalar observations and long-string byte counts without presenting the view as complete. Original artifact bytes, digest and range handles remain authoritative; no preview prose is used for execution decisions. | `skill_output_artifact/structured_preview_tests.rs`, `skill_output_artifact_tests.rs` |
| KB ingestion | Per-run byte/file ceilings | Parameter validation is explicit. Accepted work checkpoints an unvisited frontier and resumes without resetting or duplicating prior documents. | `kb/src/main_tests.rs`: direct contract, persistent resume and chunk-budget tests |
| KB extraction | Extracted text and structural ceilings | Rejects/skips an over-budget document with a stable reason rather than indexing an unlabeled prefix. Stored source ownership is unchanged. | `kb/src/ingest_extract_tests.rs` |
| KB input | Namespace, query and path length | Rejects invalid input; never truncates a query to manufacture a different request. This is not language-specific routing. | `kb/src/main_tests.rs`: direct search/ingest/list contract tests |
| KB document list | Page clamp and lookahead removal | Reads one extra row to determine `has_more`, then returns an owner-scoped continuation. Removed lookahead is fetched on the next page, not discarded. | `kb/src/main_tests.rs::document_listing_uses_stable_cursor_pagination` |
| KB storage | Default database quota | Persistent host-owned storage policy, not an agent reasoning budget. Writes exceeding quotas fail transactionally and preserve prior data. | `kb/src/main_tests.rs::all_persistent_storage_quotas_reject_writes_transactionally` |

Paths in the last column are relative to `crates/clawd/src/` unless they begin
with `UI/`, `scripts/`, or `kb/`; `kb/` refers to `crates/skills/kb/`.

## Review Rules

- Keep explicit failed attempts and unavailable evidence. A smaller response is
  not proof of completeness, correctness, or successful delivery.
- Keep resource ceilings on untrusted input, archive objects, parser output and
  decompression. Do not replace them with unrestricted memory allocation.
- A page, digest prefix, or preview must not become runtime decision evidence.
- Add only individually reviewed baseline rows with boundary tests. Do not
  regenerate the entire baseline to accept unrelated additions.
- New capability coverage and deployment acceptance are separate from this
  inventory review. Passing the ratchet does not certify all live workflows.
