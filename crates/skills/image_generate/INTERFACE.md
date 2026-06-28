# image_generate Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the image_generate implementation.

## Capability Summary
- `image_generate` creates images from a prompt and optional style/render controls.
- It writes generated assets to an output path and returns file markers in `text`.
- It also supports optional vendor/model routing and response language selection for the human-readable success text.
- It supports `dry_run=true` for capability validation without calling a provider or writing an image file.
- It exposes provider-neutral async job actions for polling and cancellation: `image.poll` and `image.cancel`.
- Successful responses include machine-readable `extra` metadata such as `provider`, `model`, `model_kind`, and `outputs`.

## Planner Selection Notes
- For requests that create a new image from text/style requirements and save or return the result, use `image_generate` / planner capability `image.generate`.
- For existing image-generation jobs with a `task_id`, use `image.poll` to inspect status and `image.cancel` to stop the job. Do not infer job ids from prose; pass structured ids from prior tool evidence or user-provided fields.
- Do not synthesize images through shell drawing commands or local CLI fallbacks unless the user explicitly requests shell/CLI execution or configured image providers are unavailable and an explicit local fallback is enabled.
- Preserve requested save locations as `output_path`; the skill returns machine-readable path evidence in `extra.outputs`.

## Actions
- `generate`: create or plan image generation. This is the default when `action` is omitted.
- `poll`: inspect a previously accepted async image-generation job by `task_id`.
- `cancel`: request cancellation for a previously accepted async image-generation job by `task_id`.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| generate | `prompt` | yes | string | - | Text prompt describing desired image. |
| generate | `size` | no | string | `1024x1024` | Output size hint. |
| generate | `style` | no | string | impl default | Stylistic rendering hint. |
| generate | `quality` | no | string | impl default | Quality/performance tradeoff hint. |
| generate | `n` | no | number | `1` | Number of images to generate. |
| generate | `output_path` | no | string(path) | auto | Save path for generated image(s). |
| generate | `response_language` or `language` | no | string | impl/config default | Language for the human-readable success text. |
| generate | `vendor` | no | string | impl default | Backend vendor selector. |
| generate | `model` | no | string | impl default | Backend model selector. |
| generate | `timeout_seconds` | no | integer | impl/config default | Per-request timeout, clamped by implementation. |
| generate | `dry_run` | no | boolean | `false` | Validate and return planned machine output without provider calls or file writes. |
| generate | `poll_after_seconds` / `poll_after_ms` | no | integer | 5 seconds | Poll cadence hint for async-preferred dry-run contracts. |
| generate | `expires_at` | no | integer(unix seconds) | now + 600 | Expiration timestamp for async-preferred dry-run contracts. |
| poll | `task_id` | yes | string | - | Provider/runtime task id from prior async evidence. |
| poll | `job_id` | no | string | derived | Provider job id or result ref. |
| poll | `output_path` | no | string(path) | auto | Planned or final image output path. |
| poll | `poll_after_seconds` / `poll_after_ms` | no | integer | 5 seconds | Next poll cadence hint. |
| poll | `expires_at` | no | integer(unix seconds) | now + 600 | Expiration timestamp for this poll contract. |
| poll | `dry_run` | no | boolean | `false` | Return synthetic adapter evidence without provider calls. |
| poll | `mock_status` | no | string | `running` | Dry-run status fixture such as `running`, `succeeded`, `failed`, `expired`, or `cancelled`. |
| cancel | `task_id` | yes | string | - | Provider/runtime task id from prior async evidence. |
| cancel | `job_id` / `cancel_token` / `cancel_ref` | no | string | derived | Provider cancellation reference. |
| cancel | `dry_run` | no | boolean | `false` | Return synthetic cancellation evidence without provider calls. |

## Config Entry Points
- `configs/image.toml` -> `[image_generation]` controls provider routing, model defaults, output limits, and `local_fallback_enabled`.
- `IMAGE_GENERATION_LOCAL_FALLBACK=1|true|on|yes` overrides `local_fallback_enabled` for explicit smoke/NL-test continuity when all configured providers fail.
- Provider credentials use dedicated environment variables such as `IMAGE_GENERATION_MINIMAX_API_KEY`; do not reuse chat/model API key names for image generation.

## Success `extra` (`status=ok`)
- `provider`: resolved backend provider name
- `model`: resolved model name
- `model_kind`: adapter/runtime mode chosen by implementation
- `latency_ms`: reserved latency field
- `outputs`: machine-readable output summary, currently `[{\"type\":\"image_file\",\"path\":\"...\"}]`
- `dry_run`: present and `true` for dry-run validation responses.
- `planned_outputs`: planned file outputs for dry-run validation responses.
- `pending_async_job_contract`: async-preferred job contract for dry-run planning; includes `poll_adapter.kind=media_job_poll`.
- `async_poll_adapter_result`: machine-readable poll adapter result for `poll` actions.
- `async_cancel_adapter_result`: machine-readable cancellation adapter result for `cancel` actions.
- `provider_cancel_contract`: provider-neutral cancellation request evidence for `cancel` actions.
- `fallback`: present only when an explicit local fallback produced the file after provider generation failed.

## Error Contract
- Missing or empty `prompt`.
- Invalid option values (`size/style/quality/n`).
- Provider/runtime generation failures should return clear error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"prompt":"Minimal black app icon with claw mark","size":"1024x1024"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"Generated successfully and saved: image/out-1.png\nFILE:image/out-1.png\nEPHEMERAL:IMAGE_SAVED","extra":{"provider":"openai","model":"gpt-image-1","model_kind":"native","latency_ms":0,"outputs":[{"type":"image_file","path":"image/out-1.png"}]},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"prompt":"Minimal black app icon with claw mark","output_path":"tmp/icon.png","dry_run":true}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"IMAGE_GENERATE_DRY_RUN","extra":{"dry_run":true,"provider":"minimax","model":"image-01","model_kind":"dry_run","output_path":"tmp/icon.png","outputs":[],"planned_outputs":[{"type":"image_file","path":"tmp/icon.png"}],"pending_async_job_contract":{"poll_adapter":{"kind":"media_job_poll"}}},"error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"action":"poll","task_id":"task-123","job_id":"job-123","output_path":"tmp/icon.png","dry_run":true,"mock_status":"succeeded"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"IMAGE_TASK:task-123","extra":{"task_id":"task-123","job_id":"job-123","status":"succeeded","async_poll_adapter_result":{"adapter_kind":"media_job_poll","status":"succeeded"}},"error_text":null}
```

### Example 4
Request:
```json
{"request_id":"demo-4","args":{"action":"cancel","task_id":"task-123","job_id":"job-123","dry_run":true}}
```
Response:
```json
{"request_id":"demo-4","status":"ok","text":"IMAGE_TASK_CANCELLED:task-123","extra":{"task_id":"task-123","job_id":"job-123","status":"cancelled","async_cancel_adapter_result":{"adapter_kind":"media_job_poll","status":"cancelled"}},"error_text":null}
```
