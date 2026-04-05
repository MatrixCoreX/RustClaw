# audio_transcribe Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the audio_transcribe implementation.

## Capability Summary
- `audio_transcribe` converts audio input into text transcription.
- It supports local file path input or public audio URL input, plus optional hints and backend model/vendor selection.
- Successful responses include machine-readable `extra` metadata such as `provider`, `model`, `model_kind`, `audio_path`, and `outputs`.

## Actions
- No explicit action is required.
- Behavior is driven by audio source and optional tuning parameters.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| transcribe | `audio.path` or `audio_path` or `path` | conditional | string(path) | - | Local audio file path (`audio.path` preferred). |
| transcribe | `audio.url` or `audio_url` | conditional | string(url) | - | Public audio URL. Some native adapters prefer or require URL input. |
| transcribe | `transcribe_hint` | no | string | - | Prompt/hint to improve recognition quality. |
| transcribe | `vendor` | no | string | impl default | Backend vendor selector. |
| transcribe | `model` | no | string | impl default | Backend model selector. |

Provide one audio source: local path or URL.

## Success `extra` (`status=ok`)
- `provider`: resolved backend provider name
- `model`: resolved model name
- `model_kind`: adapter/runtime mode chosen by implementation
- `audio_path`: original local path or URL string actually used
- `outputs`: machine-readable output summary, currently `[{\"type\":\"text\",\"preview\":\"...\"}]`
- `latency_ms`: reserved latency field

## Error Contract
- Missing audio source.
- Invalid/unreadable local audio path or invalid URL input.
- Compatible adapters that require local file upload return clear path-related errors.
- Native adapters that require public URL input return clear URL/configuration errors.
- Provider/runtime transcription failures should return clear error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"audio":{"path":"recordings/meeting.wav"},"transcribe_hint":"English technical discussion"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"Transcription: ...","extra":{"provider":"openai","model":"gpt-4o-mini-transcribe","model_kind":"compat","audio_path":"recordings/meeting.wav","outputs":[{"type":"text","preview":"Transcription: ..."}],"latency_ms":0},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"audio":{"url":"https://example.com/audio/demo.mp3"},"vendor":"qwen"}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"Transcription: ...","extra":{"provider":"qwen","model":"qwen-asr","model_kind":"native","audio_path":"https://example.com/audio/demo.mp3","outputs":[{"type":"text","preview":"Transcription: ..."}],"latency_ms":0},"error_text":null}
```
