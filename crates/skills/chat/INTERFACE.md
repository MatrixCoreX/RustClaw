# chat Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the chat skill implementation.

## Capability Summary
- `chat` provides lightweight conversational generation for joke/chitchat style requests.
- It is text-only and returns one concise assistant reply.

## Actions
- No explicit `action` is required.
- Optional mode/style can be passed via `style|mode` (`chat` or `joke`).

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| chat/joke | `text` | yes | string | - | User input text to respond to. |
| chat/joke | `style` | no | string | `chat` | Reply style (`chat` or `joke`). |
| chat/joke | `system_prompt` | no | string | impl default | Optional override for system instruction. |
| chat/joke | `max_tokens` | no | integer | `256` | Max output tokens for LLM call. |
| chat/joke | `temperature` | no | number | `0.7` | Sampling temperature. |

## Error Contract
- Missing or empty `text` returns `status=error` with readable `error_text`.
- Upstream LLM failures return `status=error` with HTTP status/body summary.
- Empty LLM content returns `status=error` with explicit error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"text":"讲个笑话","style":"joke"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"有一天...","error_text":null}
```
