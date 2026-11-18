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

## Read-only availability

The availability response keeps `capacity_decision` for automatic background
generation and adds a separate versioned `manual_growth` observation. UI labels
must not equate these: unrelated main-database growth can pause automatic work
while an explicit canonical edit still fits the memory-domain budget.

The observation and transactional write admission use one typed evaluator,
including the distinct before-growth and after-growth allocation boundaries.
Missing, invalid and unavailable configuration are machine states, not parsed
error prose or read-side repair triggers. An observation is never a permission
grant; mutation still rechecks the original actor, policy and actual effects.

Ordinary callers receive the manual decision but not global allocation or
configured-limit numbers. Administrator measurements remain separate from the
actual filesystem-free observation. Owner, scope, settings revision and request
ordering continue to bind the entire response. Clients connected to a node
without the new observation show it as unavailable, not implicitly allowed.

Deletion/export and no-new-content effects remain usable when growth is blocked.
Source restoration still has its own source row/byte checks. Displaying an
available storage budget does not promise model, network or provider readiness.
