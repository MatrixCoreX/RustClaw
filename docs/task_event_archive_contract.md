# Task Event Archive And Replay Contract

RustClaw separates live event delivery from durable replay:

- `task_event_stream` is the bounded hot suffix used for low-latency SSE.
- `task_event_archive` is the append-only redacted event record.
- `task_event_snapshots` stores periodic projections tied to exact source
  sequence ranges and hashes.
- `task_event_artifacts` stores payloads that exceed the inline event budget.

## Event Admission

Every admitted event has:

- an event and payload schema version;
- a monotonically increasing task-local `seq`;
- an event hash and previous event hash;
- task/thread/parent/child machine references;
- redaction metadata and artifact references;
- a redacted payload or a persisted large-payload artifact reference.

Claim-owned events are appended only while the exact `(lease_owner,
claim_attempt)` remains active. Event admission writes the hot row and archive
row in one SQLite transaction before notifying live SSE subscribers.

## Snapshots

A snapshot is written every 256 archived events and on `task_final`. It records:

- `source_event_range.start_seq/end_seq/event_count`;
- a SHA-256 digest over the ordered source event hashes;
- a snapshot hash;
- event type counts and latest machine task/execution state;
- the archive redaction policy.

Snapshots are replay indexes and integrity evidence. They do not replace source
events and do not contain raw prompts, raw provider responses, credentials, or
unredacted user-visible payloads.

## Replay

`GET /v1/tasks/{task_id}/events` reads the archive in bounded pages while using
the hot broadcast channel only as a wake-up signal. A client starting at cursor
zero can therefore read more than the hot 1,024-event suffix without stalling.

- `archive_replay` means an old hot cursor was recovered from durable archive.
- `cursor_expired` is emitted only when the archive itself has a real prefix
  gap; its payload reports the available range and replay source.
- `follow=false` drains every currently archived page and then closes.
- `follow=true` drains the archive, then waits for new notifications.

Browser task traces and teaching mode consume these versioned events. `clawcli
events/watch` keeps raw machine event access, and `clawcli replay export`
includes archived event sequence/hash/payload data in a redacted recorded-only
bundle.

## Retention And Deletion

The archive follows task retention. Cleanup removes hot events, archive rows,
snapshots, and event artifacts only after their owning task row has been
deleted by the configured task retention policy. Replay exports are explicit
operator artifacts and are not used as runtime permission, routing, or task
state.
