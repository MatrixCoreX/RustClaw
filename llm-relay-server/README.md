# Managed LLM Relay

Standalone OpenAI-compatible chat relay. It keeps the upstream provider credential on the server,
admits devices by Slot 0 public key, automatically issues a unique credential after one signed
challenge, and persists per-device UTC-day usage in its own SQLite database.

## Security model

- The process binds to `127.0.0.1:8796` by default. nginx is the public TLS boundary.
- Only Slot 0 public keys in the relay-owned allowlist can enroll. Physical and configured simulated
  signing devices use the same P-256 proof contract.
- Raw relay keys are returned once to the enrolling device and are never stored. SQLite stores the
  Slot 0 public key, key ID, and an HMAC-SHA256 digest protected by `RELAY_KEY_PEPPER`.
- The default allowance is 100 dispatched upstream requests per device key per UTC day.
- `/v1/models` and `/v1/quota` require authentication but do not consume model-call allowance.
- Request bodies, responses, credentials, and tool arguments are not written to relay logs.
- The nginx origin audit log records request IDs, paths, timing, source addresses, and AOP status,
  but excludes query strings, cookies, authorization headers, and request bodies.
- Caller-provided upstream URLs, headers, credentials, and unknown routing fields are rejected.
- Both buffered and streaming upstream responses have a strict byte limit so a provider or proxy
  cannot force the relay to retain unbounded response data.
- nginx exposes only `/health` and `/v1/*`; readiness details and administration routes remain on
  the loopback listener. Forwarded client-address headers are removed at this trust boundary.

## Required environment

```text
RELAY_KEY_PEPPER=<at-least-32-random-bytes>
RELAY_DATABASE_PATH=/var/lib/llm-relay/relay.db
RELAY_LISTEN_ADDR=127.0.0.1:8796
RELAY_UPSTREAM_BASE_URL=https://api.minimaxi.com/v1
RELAY_UPSTREAM_API_KEY=<server-side-provider-key>
RELAY_UPSTREAM_MODEL=MiniMax-M3
RELAY_UPSTREAM_VENDOR=minimax
RELAY_PUBLIC_MODEL=minimax
RELAY_REQUESTS_PER_DAY=100
RELAY_REQUESTS_PER_MINUTE=20
RELAY_TOKENS_PER_DAY=100000000
RELAY_MAX_TOKENS_PER_REQUEST=16384
RELAY_MAX_INFLIGHT=16
RELAY_MAX_INFLIGHT_PER_KEY=4
RELAY_UPSTREAM_TIMEOUT_SECONDS=180
RELAY_MAX_UPSTREAM_RESPONSE_BYTES=16777216
```

Startup fails when the pepper, upstream key, database, or bind policy is invalid. An empty device
allowlist is valid during initial provisioning. Public binding additionally requires
`RELAY_ALLOW_PUBLIC_BIND=true`; production should not use it.

## Device-key administration

The admin CLI reads only `RELAY_KEY_PEPPER`, `RELAY_DATABASE_PATH`, and the default daily limit.
Client keys cannot be issued through the CLI: enrollment must prove possession of the allowlisted
Slot 0 private key.

```bash
llm-relay-server device allow --label device-name --device-pubkey SLOT0_PUBKEY --daily-limit 100
llm-relay-server device list
llm-relay-server device revoke SLOT0_PUBKEY
llm-relay-server key issue-admin --label website-admin
llm-relay-server key list
llm-relay-server key revoke KEY_ID
```

`issue-admin` creates a separate service credential with read/write usage-administration scopes.
It cannot call models or chat completions and is excluded from device counts and usage pages. Keep
it only in the website backend environment. The loopback-only administration API provides paged
allowlist and enrollment state at `GET /internal/admin/device-allowlist`, paged device usage at
`GET /internal/admin/usage`, and updates an active limit at
`PUT /internal/admin/devices/:device_pubkey/daily-limit`; every limit change is written to the immutable
`relay_admin_audit` table.

On first use, an allowlisted installation calls `POST /v1/device-key/request`, signs the returned
canonical challenge with Slot 0, and submits it to `POST /v1/device-key/verify`. The raw relay key
is returned once and stored in the installation's private credential broker. Normal model calls use
that bearer key and do not sign every request. A 401 response permits one automatic re-enrollment;
revoked devices cannot obtain a replacement.

## API

```bash
curl https://llm.example.test/health

curl https://llm.example.test/v1/models \
  -H 'Authorization: Bearer DEVICE_KEY'

curl https://llm.example.test/v1/quota \
  -H 'Authorization: Bearer DEVICE_KEY'

curl https://llm.example.test/v1/chat/completions \
  -H 'Authorization: Bearer DEVICE_KEY' \
  -H 'Content-Type: application/json' \
  -d '{"model":"minimax","messages":[{"role":"user","content":"Hello"}]}'
```

Both JSON and SSE (`stream=true`) Chat Completions responses are supported. Public model aliases
are replaced with the server-owned upstream model before dispatch and restored in responses.
`/health/live`, `/health/ready`, and `/internal/admin/*` are loopback-only endpoints and must not be
published by the reverse proxy.

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Deployment templates are under `deploy/`. Install `nginx-llm-relay-logging.conf` in nginx's `http`
context before enabling `nginx-llm-relay.conf`. The service unit uses a dedicated unprivileged account,
a private environment file, and a dedicated `/var/lib/llm-relay` data directory. Build the release
binary on a compatible build host; do not install a Rust toolchain on a small production server.
