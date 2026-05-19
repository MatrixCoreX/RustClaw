# x Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the X posting skill implementation.

## Capability Summary
- `x` drafts or publishes text posts to X/Twitter-like channels.
- It is safety-first: draft mode is the default and publish must be explicitly requested.

## Config Entry Points
- File config: `configs/x.toml`
- Env overrides: `X_USE_XURL`, `XURL_BIN`, `XURL_APP`, `XURL_AUTH`, `XURL_USERNAME`, `XURL_TIMEOUT_SECONDS`, `X_REQUIRE_EXPLICIT_SEND`, `X_MAX_TEXT_CHARS`
- Runtime dependency: publishing uses a local `xurl` executable and its local OAuth/user login state.
- Setup requirement: complete `xurl auth oauth2` with `tweet.write` scope before real publishing.
- This is not a static API-key-only skill; `xurl_bin` must resolve to an executable command in `PATH` or an executable file path.

## Actions
- No action field is required.
- Behavior is controlled by `send` / `dry_run` flags on a post payload.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| post draft/preview | `text` | yes | string | - | Post content. Must be non-empty. |
| post draft/preview | `dry_run` | no | boolean | `true` | Keep as preview-only by default. |
| post draft/preview | `send` | no | boolean | `false` | Explicitly keep non-publish flow. |
| publish | `text` | yes | string | - | Final post content. |
| publish | `send` | yes | boolean | - | Must be `true` for actual publish. |
| publish | `dry_run` | no | boolean | `false` | Optional explicit publish-mode indicator. |

## Error Contract
- `text` must not be empty.
- Conflicting flags are invalid (`send=true` with `dry_run=true`).
- Publishing requires `use_xurl=true` (or `X_USE_XURL=true`) because non-`xurl` publish mode is not implemented.
- Unsupported extra fields should be rejected by planner/contract.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"text":"Daily market note","dry_run":true}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"x skill dry_run=1, preview post: Daily market note","error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"text":"Daily market note","send":true,"dry_run":false}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"x post success via xurl: id=1234567890 text=Daily market note","error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"text":"Daily market note","send":true,"dry_run":true}}
```
Response:
```json
{"request_id":"demo-3","status":"error","text":"","error_text":"x skill args are invalid: send=true conflicts with dry_run=true"}
```
