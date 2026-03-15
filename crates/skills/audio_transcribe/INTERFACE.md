# audio_transcribe Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the audio_transcribe implementation.

## Capability Summary
- `audio_transcribe` converts audio input into text transcription.
- It supports optional hints and backend model/vendor selection.

## Actions
- No explicit action is required.
- Behavior is driven by audio source and optional tuning parameters.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| transcribe | `audio.path` or `path` | yes | string(path) | - | Input audio source path (`audio.path` preferred). |
| transcribe | `transcribe_hint` | no | string | - | Prompt/hint to improve recognition quality. |
| transcribe | `vendor` | no | string | impl default | Backend vendor selector. |
| transcribe | `model` | no | string | impl default | Backend model selector. |

## Error Contract
- Missing audio path.
- Invalid/unreadable audio source.
- Provider/runtime transcription failures should return clear error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"audio":{"path":"recordings/meeting.wav"},"transcribe_hint":"English technical discussion"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"Transcription: ...","error_text":null}
```
