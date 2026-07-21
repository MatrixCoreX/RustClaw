# music_generate Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the music_generate implementation.

## Capability Summary
- `music_generate` creates provider-backed music/audio files from a musical prompt, lyrics, or instrumental request.
- The first live adapter is MiniMax-compatible; other provider slots are available for dry-run, planning, compatible gateways, and future native adapters.
- It supports `music-2.6`-style prompt, lyrics, instrumental, and cover-model fields through structured input.
- It saves generated audio files to disk and returns machine-readable output metadata in `extra`.
- It supports `dry_run` so planner and runner paths can be tested without consuming music quota.
- It exposes provider-neutral long-tail task actions: `generate`, `poll`, and `cancel`. `generate` dry-run returns `extra.pending_async_job_contract`; `poll` returns `extra.async_poll_adapter_result`; `cancel` returns `extra.async_cancel_adapter_result`.

## Config Entry Points
- Main music config: `configs/music.toml` -> `[music_generation]`.
- Shared provider fallback: `configs/config.toml` -> `[llm.<vendor>]`.
- Current repo default: `default_vendor = "minimax"`, `default_model = "music-2.6"`.
- Optional dedicated key: `MUSIC_GENERATION_<VENDOR>_API_KEY`; otherwise use the shared provider key path.
- Non-MiniMax live calls require either a future native adapter or a provider block with `adapter_kind = "minimax_compatible"` for an endpoint that truly follows the MiniMax-compatible contract.

## Actions
- `generate`: generate a song or instrumental audio file.
- `preview_generate`: validate and project music generation without provider calls or file writes; this action always forces dry-run.
- `poll`: poll a provider music generation job through the `media_job_poll` adapter contract.
- `cancel`: cancel a provider music generation job when a native provider cancel adapter exists; dry-run returns the cancellation contract.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| preview_generate | `prompt` / `description` or `lyrics` | conditional | string | - | Music input to validate; this action always forces dry-run. |
| preview_generate | generation options | no | mixed | config defaults | Uses the same bounded input options as `generate` without provider calls or file writes. |
| generate | `prompt` / `description` | conditional | string | - | Music style, mood, and scenario. Required for instrumental or lyrics-optimizer requests. |
| generate | `lyrics` | conditional | string | - | Song lyrics. Required unless `lyrics_optimizer=true` or `is_instrumental=true`. |
| generate | `lyrics_optimizer` | no | boolean | auto | When true and lyrics are empty, the selected adapter may generate lyrics from `prompt`. |
| generate | `is_instrumental` | no | boolean | `false` | Generate instrumental music without vocals. |
| generate | `format` / `response_format` | no | string | config default | `mp3`, `wav`, or `flac`. |
| generate | `output_path` | no | string(path) | auto | Workspace output path for generated audio. |
| generate | `audio_url` / `audio_base64` / `cover_feature_id` | no | string | - | Cover-generation inputs when using cover models. |
| generate | `poll_after_seconds` / `poll_after_ms` | no | integer | `5s` | Dry-run async contract polling hint. |
| generate | `expires_at` | no | integer | now+600s | Dry-run async contract deadline timestamp. |
| generate | `dry_run` | no | boolean | `false` | Build request metadata without calling the provider. |
| generate | `vendor` | no | string | config default | Provider key such as `minimax`, `mimo`, `custom`, or another configured vendor. |
| generate | `model` | no | string | config default | Music generation model for the selected provider. |
| poll | `task_id` | yes | string | - | Provider task identifier. |
| poll | `job_id` | no | string | provider-derived | Stable runtime async job id. |
| poll | `poll_after_seconds` / `poll_after_ms` | no | integer | `5s` | Reschedule interval. |
| poll | `expires_at` | no | integer | now+600s | Deadline timestamp. |
| poll | `output_path` | no | string(path) | auto | Planned output path for successful result projection. |
| poll | `mock_status` / `mock_file_id` | no | string | - | Dry-run status and file id projection fields. |
| poll | `dry_run` | no | boolean | `false` | Return a structured adapter projection without calling a provider. |
| cancel | `task_id` | yes | string | - | Provider task identifier. |
| cancel | `job_id` / `cancel_token` / `cancel_ref` | no | string | provider-derived | Runtime cancel reference. |
| cancel | `dry_run` | no | boolean | `false` | Return a structured cancellation projection without calling a provider. |
| all | `vendor` / `model` | no | string | config default | Provider and model selection tokens. |

## Success `extra` (`status=ok`)
- `provider`: resolved backend provider name.
- `model`: resolved model name.
- `model_kind`: adapter/runtime mode, such as `minimax_native` or `unsupported` in dry-run metadata.
- `adapter_kind`: async adapter family token, currently `media_job_poll` for dry-run and poll/cancel metadata.
- `output_path`: saved or planned audio path.
- `outputs`: machine-readable output summary, currently `[{"type":"audio_file","path":"..."}]` for completed files.
- `planned_outputs`: planned file outputs for dry-run validation responses.
- `audio_format`: normalized output format.
- `trace_id` and `extra_info`: provider metadata when returned.
- `dry_run`: present and true only for dry runs.
- `pending_async_job_contract`: dry-run-only preview of the machine fields a live async-capable adapter would place in `pending_async_job`; it is intentionally not named `pending_async_job`.
- `async_contract`: stable alias of the same dry-run async job contract for planner-requested response fields.
- `async_poll_adapter_result`: present for `poll`; contains `job_id`, `result_ref`, `status=accepted|running|succeeded|failed|expired|cancelled`, `poll_after_seconds`, `poll_after_ms`, `expires_at`, `message_key`, `retryable`, and terminal `final_result_json`, `failure_result_json`, or `cancellation_result_json`.
- `async_cancel_adapter_result`: present for `cancel`; contains `job_id`, `cancel_ref`, `cancel_token`, `status=cancelled|requires_provider_adapter`, `message_key`, `retryable`, and either `cancellation_result_json` or `provider_cancel_contract`.

## Error Contract
- Missing required `prompt`/`lyrics` combination.
- Missing required `task_id` for `poll` or `cancel`.
- Unsupported action token.
- Unsupported vendor, invalid output path, or path outside workspace.
- Missing API key for live generation.
- Provider generation/download/write failures.
- Live provider poll/cancel without a native adapter returns structured adapter-missing metadata instead of a natural-language decision.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"prompt":"Soulful blues, rainy night, slow tempo","lyrics":"[Verse]\nRain on the window\n[Chorus]\nMidnight keeps singing","format":"mp3","output_path":"music/demo.mp3"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"MUSIC_FILE:music/demo.mp3","extra":{"provider":"minimax","model":"music-2.6","model_kind":"minimax_native","output_path":"music/demo.mp3","outputs":[{"type":"audio_file","path":"music/demo.mp3"}],"audio_format":"mp3"},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"prompt":"Warm ambient piano loop","is_instrumental":true,"dry_run":true}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"MUSIC_GENERATE_DRY_RUN","extra":{"provider":"minimax","model":"music-2.6","model_kind":"minimax_native","adapter_kind":"media_job_poll","dry_run":true,"request":{"model":"music-2.6","prompt":"Warm ambient piano loop","is_instrumental":true},"planned_outputs":[{"type":"audio_file","path":"music/download/music-1999999999.mp3"}],"pending_async_job_contract":{"job_id":"provider:music_generate:minimax:dry_run","provider":"minimax","status":"accepted","poll_after_seconds":5,"poll_after_ms":5000,"expires_at":1999999999,"cancel_ref":"provider:music_generate:minimax:dry_run","cancel_token":"provider:music_generate:minimax:dry_run","result_ref":"provider:music_generate:minimax:dry_run","message_key":"clawd.task.async_job_pending","retryable":true,"poll_adapter":{"kind":"media_job_poll","skill_name":"music_generate","args":{"action":"poll","task_id":"dry_run","dry_run":true}}},"outputs":[]},"error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"action":"poll","task_id":"provider-task-1","job_id":"provider:music_generate:minimax:provider-task-1","dry_run":true,"mock_status":"succeeded","output_path":"music/demo.mp3"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"MUSIC_TASK:provider-task-1","extra":{"provider":"minimax","model":"music-2.6","model_kind":"minimax_native","task_id":"provider-task-1","job_id":"provider:music_generate:minimax:provider-task-1","status":"succeeded","async_poll_adapter_result":{"schema_version":1,"adapter_kind":"media_job_poll","status":"succeeded","job_id":"provider:music_generate:minimax:provider-task-1","result_ref":"provider:music_generate:minimax:provider-task-1","message_key":"clawd.task.async_job_completed","retryable":false,"final_result_json":{"source":"music_generate_poll_adapter","output_path":"music/demo.mp3","outputs":[{"type":"audio_file","path":"music/demo.mp3"}]}}},"error_text":null}
```

### Example 4
Request:
```json
{"request_id":"demo-4","args":{"action":"cancel","task_id":"provider-task-1","job_id":"provider:music_generate:minimax:provider-task-1","dry_run":true}}
```
Response:
```json
{"request_id":"demo-4","status":"ok","text":"MUSIC_TASK_CANCELLED:provider-task-1","extra":{"provider":"minimax","model":"music-2.6","model_kind":"minimax_native","task_id":"provider-task-1","job_id":"provider:music_generate:minimax:provider-task-1","status":"cancelled","dry_run":true,"async_cancel_adapter_result":{"schema_version":1,"adapter_kind":"media_job_poll","status":"cancelled","job_id":"provider:music_generate:minimax:provider-task-1","result_ref":"provider:music_generate:minimax:provider-task-1","cancel_ref":"provider:music_generate:minimax:provider-task-1","cancel_token":"provider:music_generate:minimax:provider-task-1","message_key":"clawd.task.cancelled","retryable":false,"cancellation_result_json":{"source":"music_generate_cancel_adapter","status":"cancelled"}}},"error_text":null}
```
