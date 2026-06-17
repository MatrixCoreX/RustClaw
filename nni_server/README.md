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
node nni_server/server.mjs
```

The server stores its tasks, joined devices, and compliance records in the JSON state file configured by `NNI_SERVER_STATE_PATH`.

## API

- `GET /v1/health`
- `POST /v1/nni/server/join/request`
- `POST /v1/nni/server/join/verify`

The request endpoint creates an empty `nni_join` task payload and returns a random `challenge`.
The verify endpoint validates the device signature against the public key recorded by this server.
