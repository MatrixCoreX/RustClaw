# Memory write admission

The existing `memory.storage_soft_limit_bytes` configuration is the sole input.
There is no second runtime default. Its existing minimum-one-byte normalization
is retained. Startup projects that configured value into a versioned SQLite
record and publishes its binding to the pressure observation in one transaction.
Periodic retention refreshes the same projection using the active configuration.
This record is configuration evidence, not consent, authorization or enablement.

The measurements have separate meanings:

- Whole-main-database allocated pages continue to protect background work.
- Memory-domain DBSTAT btrees include source/canonical tables, private history,
  import previews, indexes and FTS shadows. Explicit content growth is checked
  against this actual allocation before mutation and before commit.
- Filesystem available space remains a separate observation, not a
  descriptor-pinned guarantee or a substitute for either measurement.

An explicit stop without a matching producer observation/configuration revision
is conservative. A missing or corrupt observation, configuration record or
measurement cannot authorize new content. Read/admission paths never create
schemas, reconstruct policy, normalize damaged records or perform cleanup.
Startup refuses a missing migration marker for an already-owned table.

Corrections retain private revision history and therefore count as growth.
New nonempty previews, actual import insertions, restoring deleted content and
preference undo to a larger byte value also require admission. Rejection rolls
back canonical records, history, derived projections, quota changes and receipt
consumption together, including when composed inside a caller-owned transaction.

Export, deletion, suppression, metadata-only/duplicate/reused previews and
confirmations, fact-correction undo and preference undo to a smaller or equal
byte value remain available. A failed confirmation keeps its preview retryable.
These exemptions describe content effects, not a promise of zero SQLite IO.

Per-principal row/byte quotas still count only source transcripts in `memories`.
Restoring such a source checks actual rows and UTF-8 BLOB bytes, then refreshes
usage atomically. Facts, preferences, indexes and history do not consume a source
quota. Background logical-work accounting remains a separate contract.

DBSTAT allocation excludes freelists, WAL/SHM and other files and is not physical
per-principal usage. This is not a complete disk quota. SQLite/IO failure still
requires rollback and a reported failure; metadata exemptions must not be
misrepresented as unlimited disk capacity.
