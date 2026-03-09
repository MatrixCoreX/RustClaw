# x Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the X posting skill implementation.

## Capability Summary
- `x` drafts or publishes text posts to X/Twitter-like channels.
- It is safety-first: draft mode is the default and publish must be explicitly requested.

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
- Unsupported extra fields should be rejected by planner/contract.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"text":"Daily market note","dry_run":true}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"draft prepared","error_text":null}
```
