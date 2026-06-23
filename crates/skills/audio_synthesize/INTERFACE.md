# audio_synthesize Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the audio_synthesize implementation.

## Capability Summary
- `audio_synthesize` generates speech audio from text input.
- It saves the generated audio file to disk and returns a file marker in `text`.
- It supports voice/format/output path tuning plus optional vendor/model routing.
- It supports Mimo native TTS through `mimo-v2.5-tts` using the chat-completions audio response contract.
- It supports `dry_run=true` for capability validation without calling a provider or writing an audio file.
- Successful responses include machine-readable `extra` metadata such as `provider`, `model`, `voice`, `response_format`, `output_path`, and `outputs`.

## Planner Selection Notes
- For requests that turn text into spoken audio, voice, narration, or TTS output and save or return an audio file, use `audio_synthesize` / planner capability `audio.synthesize`.
- Do not synthesize speech through shell commands or local CLI tools unless the user explicitly requests shell/CLI execution or the configured audio synthesis providers are unavailable and a deliberate local fallback is enabled.
- Preserve requested save locations as `output_path`; the skill returns machine-readable path evidence in `extra.output_path` and `extra.outputs`.

## Config Entry Points
- Main TTS config: `configs/audio.toml` -> `[audio_synthesize]`.
- Shared provider fallback: `configs/config.toml` -> `[llm.<vendor>]`.
- Current repo default: `default_vendor = "minimax"`, `default_model = "speech-2.8-turbo"`, `default_voice = "male-qn-qingse"`.
- For ordinary synthesis requests, omit `vendor` and `model` unless the user explicitly chooses a provider/model; runtime config supplies the defaults.

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
| synthesize | `dry_run` | no | boolean | `false` | Validate and return planned machine output without provider calls or file writes. |

Provider notes:
- `minimax` is the current repo default for ordinary TTS requests; use the runtime default unless the user explicitly asks for another provider.
- `mimo` uses `mimo-v2.5-tts` by default, with voices such as `mimo_default`, `Mia`, `Chloe`, `冰糖`, `茉莉`, `苏打`, and `白桦`.
- Mimo native TTS currently returns chat-completions audio data; use `mp3`, `wav`, or `pcm16` according to the requested file/container format.
- Qwen native TTS remains supported, but external account billing errors should surface as provider failures rather than being hidden.

## Success `extra` (`status=ok`)
- `provider`: resolved backend provider name
- `model`: resolved model name
- `model_kind`: adapter/runtime mode chosen by implementation
- `voice`: resolved voice preset actually used
- `response_format`: normalized output audio format
- `output_path`: saved audio file path
- `outputs`: machine-readable output summary, currently `[{\"type\":\"audio_file\",\"path\":\"...\"}]`
- `dry_run`: present and `true` for dry-run validation responses.
- `planned_outputs`: planned file outputs for dry-run validation responses.
- `latency_ms`: reserved latency field

## Error Contract
- Missing/empty text input.
- Invalid option values or unsupported format/voice/model.
- Provider/runtime synthesis failures should return clear error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"text":"Hello from RustClaw","format":"mp3","output_path":"tmp/hello.mp3"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"VOICE_FILE:tmp/hello.mp3","extra":{"provider":"minimax","model":"speech-2.8-turbo","model_kind":"native","voice":"male-qn-qingse","response_format":"mp3","output_path":"tmp/hello.mp3","outputs":[{"type":"audio_file","path":"tmp/hello.mp3"}],"latency_ms":0},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"text":"Hello from RustClaw","format":"mp3","output_path":"tmp/hello.mp3","dry_run":true}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"AUDIO_SYNTHESIZE_DRY_RUN","extra":{"dry_run":true,"provider":"minimax","model":"speech-2.8-turbo","model_kind":"dry_run","voice":"male-qn-qingse","response_format":"mp3","output_path":"tmp/hello.mp3","outputs":[],"planned_outputs":[{"type":"audio_file","path":"tmp/hello.mp3"}],"latency_ms":0},"error_text":null}
```
