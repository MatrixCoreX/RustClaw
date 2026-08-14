# nni Interface Spec

## Capability Summary

`nni` exposes structured, policy-gated access to the local NNI runtime. It reports device signer
status, controls the heartbeat intent, reads rewards and public Bancor data, and previews Bancor
quotes. It never enables simulated signing, changes remote nodes, returns full keys/signatures, or
executes a Bancor trade. Final user-facing prose is synthesized by the agent in the user's language.

The process is a thin adapter over the authenticated local NNI gateway. It must not read NNI state
files, invoke hardware helpers, or call a remote NNI node directly.

Treat signer facts independently: `simulation_enabled=true` means an explicitly enabled simulated
signer is active even though `hardware_chip_present=false`; `simulation_enable_available=true`
means simulation could be enabled elsewhere but is not active. Local participation eligibility does
not imply remote network authorization.

Action selection semantics:

- `status` is a local device-and-heartbeat summary. It does not query remote network statistics.
- `bancor_account` is the only action for the current signer's POINT/USD balances and that
  signer's own recent trades. Its `limit` bounds the account-owned trade rows. It performs its own
  signer and remote-authorization checks, so call it alone and do not preflight with
  `device_status` or combine it with `bancor_market_trades` for an account or "my trades" request.
- `bancor_market_trades` returns public market-wide trades and must not be presented as the current
  device's own trades.
- `network_stats` is backed by the signed rewards contract and therefore requires an authorized
  signer even though the returned values describe the network.
- Machine timestamps ending in `_ts` or `_unix` have a deterministic companion `_utc` field. Use
  the supplied `_utc` value when presenting a date; do not independently calculate a calendar date
  from the numeric timestamp. If no companion exists, preserve the numeric timestamp.
- A `null` observed field means only that the value was unavailable or not observed in this
  response. Do not infer a storage policy, history-retention rule, implementation detail, or causal
  explanation from `null`; report the field as unavailable when it matters to the request.

## Actions

| Action | Effect | Required parameters | Optional parameters | Signer required |
| --- | --- | --- | --- | --- |
| `status` | observe | - | - | no |
| `device_status` | observe | - | - | no |
| `heartbeat_status` | observe | - | - | no |
| `heartbeat_enable` | mutate | - | - | yes |
| `heartbeat_disable` | mutate | - | - | no |
| `heartbeat_now` | mutate | - | - | yes |
| `network_stats` | observe | - | `limit` | yes |
| `my_rewards` | observe | - | `limit` | yes |
| `bancor_market` | observe | - | - | no |
| `bancor_account` | observe | - | `limit` | yes |
| `bancor_market_trades` | observe | - | `limit` | no |
| `bancor_candles` | observe | - | `interval`, `limit`, `end_time_ts` | no |
| `bancor_quote` | observe | `side`, `pay_amount` | `pay_asset`, `slippage_bps` | no |

`interval` is one of `1m`, `5m`, `15m`, `1h`, `4h`, `1d`, `1w`, `1y`.
`side` is `buy` or `sell`. `pay_amount` is a positive decimal string. `slippage_bps` is bounded by
the Bancor domain validator. Public lists are bounded; market trades return at most 100 rows and
candles at most 300 rows.

## Parameter Contract

| Action | Param | Required | Type | Default | Description |
| --- | --- | --- | --- | --- | --- |
| all | `action` | yes | enum | - | One of the actions listed above; free-text intent is rejected. |
| `network_stats`, `my_rewards`, `bancor_account`, `bancor_market_trades` | `limit` | no | integer | action-specific | Between 1 and 100. |
| `bancor_candles` | `interval` | no | enum | `5m` | One of `1m`, `5m`, `15m`, `1h`, `4h`, `1d`, `1w`, `1y`. |
| `bancor_candles` | `limit` | no | integer | `120` | Between 1 and 300. |
| `bancor_candles` | `end_time_ts` | no | integer | current | Non-negative Unix timestamp. |
| `bancor_quote` | `side` | yes | enum | - | `buy` or `sell`. |
| `bancor_quote` | `pay_amount` | yes | decimal string | - | Positive amount; never pass a JSON float. |
| `bancor_quote` | `pay_asset` | no | enum | derived | `USD` for buy and `POINT` for sell. |
| `bancor_quote` | `slippage_bps` | no | integer | domain default | Between 0 and 10000 basis points. |

## Device State Semantics

- `signer_kind` is exactly `hardware`, `simulated`, or `unavailable`.
- `simulation_enabled=true` means the user explicitly enabled the simulated signer and it is the
  active signing source. It is compatible with `signer_available=true` and
  `hardware_chip_present=false`.
- `simulation_enable_available=true` means simulation could be enabled through the separate
  explicit UI control; it does not mean simulation is currently active.
- `local_participation_eligible` describes local signing eligibility. It does not imply remote
  authorization; interpret `network_authorization` independently.

## Result Contract

Success returns `status=ok`, compact machine JSON in `text`, and the same envelope in `extra`:

```json
{"schema_version":1,"source_skill":"nni","status":"ok","action":"device_status","observed_at_ts":1,"data":{"signer_kind":"unavailable","signer_available":false}}
```

## Error Contract

Errors return `status=error`, a stable token in `error_text`, and canonical fields in `extra`:

```json
{"schema_version":1,"source_skill":"nni","status":"error","action":"heartbeat_enable","error_code":"nni_signature_device_unavailable","message_key":"skill.nni.nni_signature_device_unavailable","retryable":false,"details":{"signer_kind":"unavailable"}}
```

Runtime decisions must consume `extra.error_code`, `extra.retryable`, and structured details. They
must not parse `text` or `error_text` as natural language.

Important error codes include `nni_signature_device_unavailable`,
`nni_signature_helper_unavailable`, `nni_remote_node_unconfigured`,
`nni_device_not_authorized`, `nni_heartbeat_network_unavailable`,
`nni_operation_in_progress`, `nni_argument_invalid`, and `nni_response_contract_invalid`.

## Request/Response Examples

### Example 1: local status

```json
{"request_id":"nni-1","args":{"action":"status"},"user_id":1,"chat_id":2,"context":null}
```

```json
{"request_id":"nni-1","status":"ok","text":"{...}","error_text":null,"extra":{"schema_version":1,"source_skill":"nni","status":"ok","action":"status","observed_at_ts":1,"data":{"device":{"signer_kind":"unavailable","signer_available":false},"heartbeat":{"desired_enabled":false,"effective_state":"disabled"}}}}
```

### Example 2: candles

```json
{"request_id":"nni-2","args":{"action":"bancor_candles","interval":"15m","limit":100},"user_id":1,"chat_id":2,"context":null}
```

### Example 3: read-only quote

```json
{"request_id":"nni-3","args":{"action":"bancor_quote","side":"buy","pay_asset":"USD","pay_amount":"25","slippage_bps":50},"user_id":1,"chat_id":2,"context":null}
```
