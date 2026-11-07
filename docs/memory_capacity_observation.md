# Memory capacity observations

The memory page separates current user policy from the automatic-generation
capacity decision. Neither is a promise that a model, network, embedding
provider or background worker is ready. This view does not change admission,
thresholds, retention, quota scope or initialization.

## Read contract

`GET /v1/memory/availability?scope=principal` reads only the authenticated
account. `scope=conversation&conversation_id=...` returns that account's
conversation policy. Administrator-scope and cross-account selectors are not
supported. Unknown query fields are rejected rather than silently ignored.

Authentication and SQLite preparation run off the asynchronous executor.
Settings, quota, pressure and SQLite measurements share one read transaction;
the connection is released before the filesystem observation. Reads never
initialize or repair policy, migrations or missing observations. Missing,
invalid and unknown states remain explicit. Refresh does not start work,
delete content, reset quota or retry a failed mutation.

The version-1 response includes the exact effective policy, separate pressure
and quota decisions, and typed byte measurements. Count/byte/logical-work
values are canonical decimal strings so JavaScript does not round large
integers. An unavailable measurement is not zero. The page rejects malformed
or contradictory responses and a policy that no longer matches the selected
account/conversation/revision. Owner epochs and request order prevent stale
responses or callbacks from reviving a previous account's observations.

## Measurement scopes

- Source quotas retain their existing `memories` source-transcript row and
  UTF-8 payload-byte scope. They do not include facts, preferences or vectors.
  Recorded usage and live source payload bytes are separate observations.
- Spent plus pending background logical work is the existing logical counter,
  not provider-token or currency billing.
- The current pressure producer measures allocated pages in the entire main
  SQLite database. Its soft threshold is not an account quota or a measurement
  of the memory domain or actual filesystem exhaustion.
- Administrators can separately inspect allocated main-database pages,
  memory-domain table/index btrees, SQLite freelist bytes, and space available
  to the process user on the filesystem containing the database path.
- Memory-domain DBSTAT pages include memory table indexes and FTS shadows,
  exclude freelists/WAL/SHM/non-btree overhead, and are not per-account physical
  usage. Builds without DBSTAT report a typed unavailable measurement.
- Filesystem sampling does not fall back to the root filesystem. In-memory
  databases and failed samples remain unavailable. This is a current-path
  diagnostic, not descriptor-pinned write-admission authority. No raw path or
  credential is returned. Global sizes and thresholds are administrator-only.

## Work still required

Observation does not resolve the broader pressure/accounting policy. Before
strengthening manual-write enforcement, define valid memory-domain and host
pressure, preserve content-releasing operations and recovery, distinguish
existing-policy loss from explicit first-install/new-account initialization,
and verify import/correction/undo behavior under actual writer races. Existing
mutation/quota counterexamples remain required tests, not accepted behavior.
