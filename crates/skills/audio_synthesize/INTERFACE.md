# audio_synthesize Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the audio_synthesize implementation.

## Capability Summary
- `audio_synthesize` generates speech audio from text input.
- It saves the generated audio file to disk and returns a file marker in `text`.
- It supports voice/format/output path tuning plus optional vendor/model routing.
- It supports Mimo native TTS through `mimo-v2.5-tts` using the chat-completions audio response contract.
- It supports `dry_run=true` for capability validation without calling a provider or writing an audio file.
- It exposes provider-neutral async job actions for polling and cancellation: `audio.poll` and `audio.cancel`.
- Successful responses include machine-readable `extra` metadata such as `provider`, `model`, `voice`, `response_format`, `output_path`, and `outputs`.

## Planner Selection Notes
- For requests that turn text into spoken audio, voice, narration, or TTS output and save or return an audio file, use `audio_synthesize` / planner capability `audio.synthesize`.
- For validation, planning, or dry-run requests that must not call a provider or write a file, use `audio.preview_synthesize`; the skill forces dry-run regardless of any caller-supplied `dry_run` value.
- For existing speech-audio jobs with a `task_id`, use `audio.poll` to inspect status and `audio.cancel` to stop the job. Do not infer job ids from prose; pass structured ids from prior tool evidence or user-provided fields.
- Do not synthesize speech through shell commands or local CLI tools unless the user explicitly requests shell/CLI execution or the configured audio synthesis providers are unavailable and a deliberate local fallback is enabled.
- Preserve requested save locations as `output_path`; the skill returns machine-readable path evidence in `extra.output_path` and `extra.outputs`.

## Config Entry Points
- Main TTS config: `configs/audio.toml` -> `[audio_synthesize]`.
- Shared provider fallback: `configs/config.toml` -> `[llm.<vendor>]`.
- Current repo default: `default_vendor = "minimax"`, `default_model = "speech-2.8-turbo"`, `default_voice = "male-qn-qingse"`.
- For ordinary synthesis requests, omit `vendor` and `model` unless the user explicitly chooses a provider/model; runtime config supplies the defaults.

## Actions
- `synthesize`: generate or plan speech audio from text. This is the default when `action` is omitted.
- `preview_synthesize`: validate and project speech audio output without provider calls or file writes.
- `poll`: inspect a previously accepted async speech-audio job by `task_id`.
- `cancel`: request cancellation for a previously accepted async speech-audio job by `task_id`.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| preview_synthesize | `text` (or `input`) | yes | string | - | Source text to validate; this action always forces dry-run. |
| preview_synthesize | `voice`, `response_format` / `format`, `output_path`, `vendor`, `model` | no | mixed | impl defaults | Preview-only synthesis options; no provider call or file write occurs. |
| synthesize | `text` (or `input`) | yes | string | - | Source text to speak. |
| synthesize | `voice` | no | string | impl default | Voice preset. |
| synthesize | `response_format` or `format` | no | string | impl default | Audio output format (e.g., mp3/wav). |
| synthesize | `output_path` | no | string(path) | auto | Output audio file path. |
| synthesize | `vendor` | no | string | impl default | Backend vendor selector. |
| synthesize | `model` | no | string | impl default | Backend model selector. |
| synthesize | `dry_run` | no | boolean | `false` | Validate and return planned machine output without provider calls or file writes. |
| synthesize | `poll_after_seconds` / `poll_after_ms` | no | integer | 5 seconds | Poll cadence hint for async-preferred dry-run contracts. |
| synthesize | `expires_at` | no | integer(unix seconds) | now + 600 | Expiration timestamp for async-preferred dry-run contracts. |
| poll | `task_id` | yes | string | - | Provider/runtime task id from prior async evidence. |
| poll | `job_id` | no | string | derived | Provider job id or result ref. |
| poll | `output_path` | no | string(path) | auto | Planned or final audio file output path. |
| poll | `poll_after_seconds` / `poll_after_ms` | no | integer | 5 seconds | Next poll cadence hint. |
| poll | `expires_at` | no | integer(unix seconds) | now + 600 | Expiration timestamp for this poll contract. |
| poll | `dry_run` | no | boolean | `false` | Return synthetic adapter evidence without provider calls. |
| poll | `mock_status` | no | string | `running` | Dry-run status fixture such as `running`, `succeeded`, `failed`, `expired`, or `cancelled`. |
| cancel | `task_id` | yes | string | - | Provider/runtime task id from prior async evidence. |
| cancel | `job_id` / `cancel_token` / `cancel_ref` | no | string | derived | Provider cancellation reference. |
| cancel | `dry_run` | no | boolean | `false` | Return synthetic cancellation evidence without provider calls. |

Provider notes:
- `minimax` is the current repo default for ordinary TTS requests; use the runtime default unless the user explicitly asks for another provider.
- `mimo` uses `mimo-v2.5-tts` by default; provider voice metadata is configured in `configs/audio.toml` under `audio_synthesize.mimo_voices`.
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
- `pending_async_job_contract`: async-preferred job contract for dry-run planning; includes `poll_adapter.kind=media_job_poll`.
- `async_contract`: stable alias of the same dry-run async job contract for planner-requested response fields.
- `async_poll_adapter_result`: machine-readable poll adapter result for `poll` actions.
- `async_cancel_adapter_result`: machine-readable cancellation adapter result for `cancel` actions.
- `provider_cancel_contract`: provider-neutral cancellation request evidence for `cancel` actions.
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
{"request_id":"demo-2","status":"ok","text":"AUDIO_SYNTHESIZE_DRY_RUN","extra":{"dry_run":true,"provider":"minimax","model":"speech-2.8-turbo","model_kind":"dry_run","voice":"male-qn-qingse","response_format":"mp3","output_path":"tmp/hello.mp3","outputs":[],"planned_outputs":[{"type":"audio_file","path":"tmp/hello.mp3"}],"pending_async_job_contract":{"poll_adapter":{"kind":"media_job_poll"}},"latency_ms":0},"error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"action":"poll","task_id":"task-123","job_id":"job-123","output_path":"tmp/hello.mp3","dry_run":true,"mock_status":"succeeded"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"AUDIO_TASK:task-123","extra":{"task_id":"task-123","job_id":"job-123","status":"succeeded","async_poll_adapter_result":{"adapter_kind":"media_job_poll","status":"succeeded"}},"error_text":null}
```

### Example 4
Request:
```json
{"request_id":"demo-4","args":{"action":"cancel","task_id":"task-123","job_id":"job-123","dry_run":true}}
```
Response:
```json
{"request_id":"demo-4","status":"ok","text":"AUDIO_TASK_CANCELLED:task-123","extra":{"task_id":"task-123","job_id":"job-123","status":"cancelled","async_cancel_adapter_result":{"adapter_kind":"media_job_poll","status":"cancelled"}},"error_text":null}
```
