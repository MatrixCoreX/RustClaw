# NNI Server

Standalone remote NNI server for device join challenge, signature verification, and compliance records.

This service is intentionally separate from `clawd`:

- It is not a Cargo workspace member.
- It is not compiled with `clawd`.
- `clawd` only calls it over HTTP from the device-side proxy flow.
- It uses Node.js 22.5 or newer and the built-in `node:sqlite` module; no Python stack, external database service, or npm dependency is required.

## Run

```bash
cd nni_server
npm run start
```

Equivalent environment variables:

```bash
NNI_SERVER_HOST=0.0.0.0 \
NNI_SERVER_PORT=8797 \
NNI_SERVER_DATABASE_PATH=data/nni-server.sqlite3 \
NNI_SERVER_STATE_PATH=data/nni-server-state.json \
NNI_SERVER_LOG_PATH=logs/nni-server.log \
NNI_SERVER_PUBLIC_KEY_WHITELIST=<128-hex-pubkey>[,<128-hex-pubkey>...] \
node nni_server/server.mjs
```

The server stores tasks, device aggregates, request records, heartbeat records, and the public-key whitelist in the SQLite database configured by `NNI_SERVER_DATABASE_PATH`. SQLite runs in WAL mode with foreign keys, a busy timeout, full synchronous durability, and indexes for device/public-key and heartbeat-time lookups.

Every signature-verified heartbeat inserts one immutable row into `heartbeat_records`. The row references the normalized `devices` row by integer ID and stores the server-generated `heartbeat_at_unix` value in whole UNIX seconds. Multiple heartbeats received in the same second remain separate rows. Client-provided time is never used as heartbeat time. `devices` also keeps `first_heartbeat_ts`, `last_heartbeat_ts`, and `heartbeat_count` for fast status reads.

`NNI_SERVER_STATE_PATH` is now a legacy import source only. On first database startup, existing JSON tasks, devices, request records, heartbeat histories, and whitelist entries are imported in one transaction. The database records a migration marker, and the server does not write new state back to JSON.

Request records stay in this server-side database for administrator audit and troubleshooting. The service does not expose a public request-record query API.
Runtime events are written as JSONL to `NNI_SERVER_LOG_PATH` (`logs/nni-server.log` by default) so NNI logs stay separate from `claw.log`. Set `NNI_SERVER_LOG_STDOUT=1` only when a supervisor intentionally captures NNI logs elsewhere.

## Public-Key Whitelist

Remote UI join requests are denied unless the device public key is present in the SQLite `public_key_whitelist` table.
An empty whitelist denies all join requests.

`NNI_SERVER_PUBLIC_KEY_WHITELIST` accepts a comma-, semicolon-, whitespace-, or newline-separated list at startup. Valid keys are inserted into the database and remain there until explicitly removed. A key still present in this environment variable will be inserted again on the next restart.

Administrators with the optional `sqlite3` command can inspect or revoke entries without changing application source:

```bash
sqlite3 data/nni-server.sqlite3 \
  "SELECT device_pubkey, created_at_unix FROM public_key_whitelist ORDER BY created_at_unix;"

sqlite3 data/nni-server.sqlite3 \
  "DELETE FROM public_key_whitelist WHERE device_pubkey = '<128-hex-pubkey>';"
```

Legacy JSON whitelist entries are imported once when the SQLite database is first created.

Both join phases enforce the whitelist:

- `POST /v1/nni/server/join/request` rejects unlisted public keys before creating a challenge.
- `POST /v1/nni/server/join/verify` checks again before accepting a signature, so a key removed after challenge creation cannot complete the UI join.

## API

- `GET /v1/health`
- `POST /v1/nni/server/join/request`
- `POST /v1/nni/server/join/verify`
- `POST /v1/nni/server/heartbeat/request`
- `POST /v1/nni/server/heartbeat/verify`

The request endpoint creates an empty `nni_join` task payload and returns a random `challenge`.
The verify endpoint validates the device signature against the public key recorded by this server.
