# chat Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the chat skill implementation.

## Capability Summary
- `chat` provides lightweight conversational generation for joke/chitchat style requests.
- It is text-only and returns one concise assistant reply.
- It also returns lightweight LLM metadata in `extra.llm`.
- Host/runtime may inject `recent_execution_context`, `_memory.context`, and `_memory.lang_hint` as background context; planner should not fabricate these internal fields on its own.

## Actions
- No explicit `action` is required.
- Optional mode/style can be passed via `style|mode` (`chat` or `joke`).

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| chat/joke | `text` or `prompt` or `input` | yes | string | - | User input text to respond to. |
| chat/joke | `style` or `mode` | no | string | `chat` | Reply style (`chat` or `joke`). |
| chat/joke | `system_prompt` | no | string | impl default | Optional override for system instruction. |
| chat/joke | `max_tokens` | no | integer | impl default | Max output tokens for LLM call; default is computed from style and input length. |
| chat/joke | `temperature` | no | number | `0.7` | Sampling temperature. |

## Host/Internal Context Fields
- `recent_execution_context`: host-injected execution summary used as additional background context.
- `_memory.context`: host-injected memory background context.
- `_memory.lang_hint`: host-injected preferred response language hint.
- Planner should not fabricate these internal fields unless the caller explicitly provides them through a trusted host path.

## Success `extra` (`status=ok`)
- `llm.prompt_name`: logical prompt family used by the skill
- `llm.prompt_source`: prompt source chosen by runtime (`inline_system_prompt` or layered prompt source)
- `llm.model`: resolved chat model
- `llm.style`: normalized style actually used
- `llm.memory_attached`: whether memory context was attached
- `llm.lang_hint`: resolved language hint, if any

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
{"request_id":"demo-1","status":"ok","text":"有一天...","extra":{"llm":{"prompt_name":"chat_skill_prompt","prompt_source":"layered","model":"gpt-4.1-mini","style":"joke","memory_attached":false,"lang_hint":""}},"error_text":null}
```
