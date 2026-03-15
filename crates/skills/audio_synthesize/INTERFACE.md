# audio_synthesize Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the audio_synthesize implementation.

## Capability Summary
- `audio_synthesize` generates speech audio from text input.
- It supports voice/format/output path tuning plus optional vendor/model routing.

## Actions
- No explicit action is required.
- Behavior is controlled by text input and synthesis options.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| synthesize | `text` (or `input`) | yes | string | - | Source text to speak. |
| synthesize | `voice` | no | string | impl default | Voice preset. |
| synthesize | `response_format` or `format` | no | string | impl default | Audio output format (e.g., mp3/wav). |
| synthesize | `output_path` | no | string(path) | auto | Output audio file path. |
| synthesize | `vendor` | no | string | impl default | Backend vendor selector. |
| synthesize | `model` | no | string | impl default | Backend model selector. |

## Error Contract
- Missing/empty text input.
- Invalid option values or unsupported format/voice/model.
- Provider/runtime synthesis failures should return clear error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"text":"Hello from RustClaw","voice":"alloy","format":"mp3","output_path":"tmp/hello.mp3"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"audio synthesized: tmp/hello.mp3","error_text":null}
```
