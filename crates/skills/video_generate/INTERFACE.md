# video_generate Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the video_generate implementation.

## Capability Summary
- `video_generate` creates provider-backed video generation tasks and can optionally wait, retrieve, and save the generated video file.
- It supports async provider jobs: `generate` with `wait_for_completion=false` returns `extra.pending_async_job`, and `poll` returns `extra.async_poll_adapter_result` for clawd checkpoint resume.
- The first live adapter is MiniMax-compatible; other provider slots are available for dry-run, planning, compatible gateways, and future native adapters.
- It supports text-to-video, image-to-video, and first/last-frame video through structured input fields.
- It returns machine-readable task/file metadata in `extra`; success `text` is a file/task marker, not a sentence template.
- It supports `dry_run` so planner and runner paths can be tested without consuming video quota.

## Config Entry Points
- Main video config: `configs/video.toml` -> `[video_generation]`.
- Shared provider fallback: `configs/config.toml` -> `[llm.<vendor>]`.
- Current repo default: `default_vendor = "minimax"`, `default_model = "MiniMax-Hailuo-2.3"`.
- Optional dedicated key: `VIDEO_GENERATION_<VENDOR>_API_KEY`; otherwise use the shared provider key path.
- Non-MiniMax live calls require either a future native adapter or a provider block with `adapter_kind = "minimax_compatible"` for an endpoint that truly follows the MiniMax-compatible contract.

## Actions
- `generate`: create a video task; optionally wait and download the video file.
- `poll`: query an existing provider video task once and return a machine-readable async poll adapter result.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| generate | `prompt` | yes | string | - | Video description and optional camera commands accepted by the selected adapter. |
| generate | `first_frame_image` / `first_frame` / `image` | no | string/object | - | Public URL, data URL, base64 object, or workspace path used as the first frame. |
| generate | `last_frame_image` / `last_frame` | no | string/object | - | Public URL, data URL, base64 object, or workspace path used as the last frame. |
| generate | `duration` | no | integer | `6` | Video duration in seconds; current implementation accepts `6` or `10`. |
| generate | `resolution` | no | string | config default | One of `512P`, `720P`, `768P`, `1080P`; exact support depends on the selected adapter/model. |
| generate | `output_path` | no | string(path) | auto | Workspace output path for downloaded video. |
| generate | `wait_for_completion` | no | boolean | `true` | If false, return the provider task id without polling. |
| generate | `download` | no | boolean | config default | If false, return the completed task/file id without downloading. |
| generate | `dry_run` | no | boolean | `false` | Build request metadata without calling the provider. |
| generate | `vendor` | no | string | config default | Provider key such as `minimax`, `mimo`, `custom`, or another configured vendor. |
| generate | `model` | no | string | config default | Video generation model for the selected provider. |
| generate | `max_poll_seconds` | no | integer | config default | Max polling window for async completion. |
| poll | `task_id` | yes | string | - | Provider video task id returned by `generate`. |
| poll | `job_id` | no | string | derived | RustClaw async job id; default is `provider:video_generate:<vendor>:<task_id>`. |
| poll | `vendor` | no | string | config default | Provider key used to query the task. |
| poll | `model` | no | string | config default | Model metadata preserved in final result. |
| poll | `poll_after_seconds` | no | integer | config poll interval | Suggested next poll delay when the task is still pending. |
| poll | `expires_at` | no | integer(epoch seconds) | derived | Expiry deadline for async resume. |
| poll | `download` | no | boolean | config default | If true and the task succeeded, retrieve and save the video file. |
| poll | `output_path` | no | string(path) | auto | Workspace output path for downloaded video. |
| poll | `dry_run` | no | boolean | `false` | Return a mock adapter result without provider calls. |
| poll | `mock_status` / `mock_file_id` | no | string | - | Dry-run-only provider status/file metadata for tests and smoke checks. |

## Success `extra` (`status=ok`)
- `provider`: resolved backend provider name.
- `model`: resolved model name.
- `model_kind`: adapter/runtime mode, such as `minimax_native` or `unsupported` in dry-run metadata.
- `task_id`: provider video task id when a request is sent.
- `status`: task status when available.
- `file_id`: provider file id when available.
- `output_path`: saved video path when downloaded.
- `outputs`: machine-readable output summary, currently `[{\"type\":\"video_file\",\"path\":\"...\"}]` when downloaded.
- `dry_run`: present and true only for dry runs.
- `pending_async_job`: present when `generate.wait_for_completion=false`; contains `job_id`, `status`, `poll_after_seconds`, `expires_at`, `cancel_ref`, `message_key`, and `poll_adapter`.
- `async_poll_adapter_result`: present for `poll`; contains `job_id`, `status=accepted|running|succeeded|failed|expired`, `poll_after_seconds`, `expires_at`, and `final_result_json` or `failure_result_json` when terminal.

## Error Contract
- Missing/empty `prompt`.
- Missing/empty `task_id` for `poll`.
- Unsupported vendor, duration, resolution, or path outside workspace.
- Missing API key for live generation.
- Provider create/query/retrieve/download failures.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"prompt":"A calm product demo shot [Static shot]","duration":6,"resolution":"768P","output_path":"video/demo.mp4"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"VIDEO_FILE:video/demo.mp4","extra":{"provider":"minimax","model":"MiniMax-Hailuo-2.3","model_kind":"minimax_native","task_id":"106916112212032","status":"Success","file_id":"176844028768320","output_path":"video/demo.mp4","outputs":[{"type":"video_file","path":"video/demo.mp4"}]},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"prompt":"A logo slowly rotates","first_frame_image":{"url":"https://example.com/logo.png"},"dry_run":true}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"VIDEO_GENERATE_DRY_RUN","extra":{"provider":"minimax","model":"MiniMax-Hailuo-2.3","model_kind":"minimax_native","dry_run":true,"request":{"model":"MiniMax-Hailuo-2.3","prompt":"A logo slowly rotates"},"outputs":[]},"error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"action":"poll","task_id":"106916112212032","job_id":"provider:video_generate:minimax:106916112212032","dry_run":true,"mock_status":"Processing"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"VIDEO_TASK:106916112212032","extra":{"provider":"minimax","model":"MiniMax-Hailuo-2.3","model_kind":"minimax_native","task_id":"106916112212032","job_id":"provider:video_generate:minimax:106916112212032","status":"Processing","async_poll_adapter_result":{"job_id":"provider:video_generate:minimax:106916112212032","status":"running","poll_after_seconds":5,"expires_at":1999999999,"message_key":"clawd.task.async_job_pending"}},"error_text":null}
```
