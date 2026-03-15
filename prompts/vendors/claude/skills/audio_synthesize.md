<!-- AUTO-GENERATED: sync_skill_docs.py -->


Vendor tuning for Claude models:
- Treat each skill description as a binding operational contract.
- Use only declared capabilities and keep args minimal and explicit.
- Prefer the narrowest tool/skill that can complete the subtask correctly.
- Do not inject unrelated context into skill args unless explicitly required.
- Optimize for precise planner/parser compatibility.

## Role & Boundaries
- You are the `audio_synthesize` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `crates/skills/audio_synthesize/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
- `audio_synthesize` generates speech audio from text input.
- It supports voice/format/output path tuning plus optional vendor/model routing.

## Actions (from interface)
- No explicit action is required.
- Behavior is controlled by text input and synthesis options.

## Parameter Contract (from interface)
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| synthesize | `text` (or `input`) | yes | string | - | Source text to speak. |
| synthesize | `voice` | no | string | impl default | Voice preset. |
| synthesize | `response_format` or `format` | no | string | impl default | Audio output format (e.g., mp3/wav). |
| synthesize | `output_path` | no | string(path) | auto | Output audio file path. |
| synthesize | `vendor` | no | string | impl default | Backend vendor selector. |
| synthesize | `model` | no | string | impl default | Backend model selector. |

## Error Contract (from interface)
- Missing/empty text input.
- Invalid option values or unsupported format/voice/model.
- Provider/runtime synthesis failures should return clear error text.

## Request/Response Examples (from interface)
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"text":"Hello from RustClaw","voice":"alloy","format":"mp3","output_path":"tmp/hello.mp3"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"audio synthesized: tmp/hello.mp3","error_text":null}
```

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
