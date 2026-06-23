# NNI Server

Standalone remote NNI server for device join challenge, signature verification, and compliance records.

This service is intentionally separate from `clawd`:

- It is not a Cargo workspace member.
- It is not compiled with `clawd`.
- `clawd` only calls it over HTTP from the device-side proxy flow.
- It uses Node.js built-in modules only; no Python stack and no npm dependencies are required.

## Run

```bash
cd nni_server
npm run start
```

Equivalent environment variables:

```bash
NNI_SERVER_HOST=0.0.0.0 \
NNI_SERVER_PORT=8797 \
NNI_SERVER_STATE_PATH=data/nni-server-state.json \
NNI_SERVER_LOG_PATH=logs/nni-server.log \
NNI_SERVER_PUBLIC_KEY_WHITELIST=<128-hex-pubkey>[,<128-hex-pubkey>...] \
node nni_server/server.mjs
```

The server stores its tasks, joined devices, compliance records, and public-key whitelist in the JSON state file configured by `NNI_SERVER_STATE_PATH`.
Request records stay in this server-side state file for administrator audit and troubleshooting. The service does not expose a public request-record query API.
Runtime events are written as JSONL to `NNI_SERVER_LOG_PATH` (`logs/nni-server.log` by default) so NNI logs stay separate from `claw.log`. Set `NNI_SERVER_LOG_STDOUT=1` only when a supervisor intentionally captures NNI logs elsewhere.

## Public-Key Whitelist

Remote UI join requests are denied unless the device public key is present in `public_key_whitelist`.
An empty whitelist denies all join requests.

The whitelist can be maintained in the state file:

```json
{
  "public_key_whitelist": [
    "0123...128 hex characters..."
  ],
  "tasks": {},
  "devices": {},
  "requests": []
}
```

`NNI_SERVER_PUBLIC_KEY_WHITELIST` can also provide a comma-, semicolon-, whitespace-, or newline-separated list at startup.
Values from the environment are merged with the state file and are persisted the next time the state is saved.

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
