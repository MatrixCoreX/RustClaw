# media_download Interface

## Capability Summary

- Download publicly accessible media from Douyin, Kuaishou, Xiaohongshu, TikTok, and YouTube share text or URLs.
- Resolve direct media URLs without downloading.
- Transcribe local video/audio, OCR local images, and prepare local video for X.
- Write generated files only to the runtime-provided task artifact directory by default.
- Never read system-browser cookies. The skill supports public content only and does not bypass DRM, private-content access, paywalls, or platform authorization.
- This document teaches usage only. Host admission and policy grants remain authoritative.

## Natural-Language Routing

- “帮我下载这条抖音/快手/小红书/TikTok/YouTube 视频” -> `download`
- A message consisting only of a supported public URL -> `download`; do not ask whether the user wants resolution or download.
- A complete copied Douyin, Kuaishou, or Xiaohongshu share message containing a supported URL -> pass the complete text as `share` and run `download` immediately.
- Default download delivery is `deliver_to_user=true`: send the downloaded media back to the originating communication channel and expose it in the UI.
- If the user explicitly says not to send the media back (for example, “不要发我”), set `deliver_to_user=false`. Do not remove that phrase from `share`; the downloader still extracts the URL from the complete text. Reply with the saved local path only and do not emit a delivery artifact.
- “只解析这个分享链接的媒体直链，不要下载” -> `resolve`
- “把这个视频转成文字/提取音频” -> `transcribe`
- “识别这些图片里的文字” -> `ocr`
- “检查或转换成可以发到 X/Twitter 的视频” -> `prepare_x`
- Prefer a dedicated built-in media skill when the request is unrelated to this package's supported actions.

## Actions

### `capabilities`

Return supported actions, platforms, security defaults, and optional dependencies. No network or filesystem access is needed.

### `download`

Extract the supported URL from `share` and download public media. `share` may be a URL by itself or the full text copied from Douyin, Kuaishou, or Xiaohongshu. Do not require the user to clean the copied message or resend only the URL. A profile URL can download multiple recent works with `profile_limit`; use `"all"` only when the user explicitly requests every accessible item.

The `download` action only produces the original image/video media. It never adds OCR text, extracted audio, or a video transcript. When the user explicitly asks for text recognition or transcription, use the separate `ocr` or `transcribe` action.

After success, expose every generated artifact through the task artifact contract when `deliver_to_user` is omitted or `true`. The originating communication adapter can send the file back, and the UI can render its preview/download URL. Host channel size limits still apply. When `deliver_to_user=false`, return generated files under `extra.saved_files`, keep `extra.artifacts` empty, and report the saved path without sending the media.

### `resolve`

Resolve downloadable media URLs from `share` without saving the media.

### `transcribe`

Transcribe `input_path`, or only extract WAV audio when `extract_audio_only=true`.

### `ocr`

Run Tesseract OCR for `input_paths` and save one text artifact.

### `prepare_x`

Check or transcode `input_path` for X compatibility. Directories are scanned recursively. Set `check_only=true` to avoid writing converted media.

## Parameter Contract

| Action | Parameter | Required | Type | Default | Description |
|---|---|---:|---|---|---|
| all | `action` | yes | string | - | One of `capabilities`, `download`, `resolve`, `transcribe`, `ocr`, `prepare_x`. |
| `download`, `resolve` | `share` | yes | string | - | A public URL or an unmodified copied share message containing the URL. |
| `download`, `resolve` | `platform` | no | string | `auto` | `auto`, `douyin`, `kuaishou`, `xiaohongshu`, `tiktok`, or `youtube`. |
| `download` | `deliver_to_user` | no | boolean | `true` | Send generated media to the originating channel/UI. Set `false` only when the user explicitly asks not to receive the file; then report its saved path. |
| `download` | `output_name` | no | string | timestamp | Plain filename only; directories are rejected. |
| `download`, `resolve` | `profile_limit` | no | integer or `all` | `20` | Maximum profile items. Use `all` only after explicit user intent. |
| `download`, `resolve` | `profile_interval_seconds` | no | number | `5` | Delay between profile requests, 0-60 seconds. |
| `download`, `resolve` | `network_timeout_seconds` | no | number | `20` | Per-request timeout, 1-120 seconds. |
| `download`, `resolve` | `browser_fallback` | no | boolean | `true` | Allow local Chromium fallback without browser-profile cookies. |
| `download` | `save_meta` | no | boolean | `false` | Save extraction metadata JSON. |
| `download` | `show_info` | no | boolean | `false` | Probe downloaded media information. |
| `transcribe` | `engine` | no | string | `whisper` | `whisper` or `funasr`. |
| `transcribe`, `ocr` | `language` | no | string | action-specific | Spoken language or Tesseract language list. |
| `transcribe` | `input_path` | yes | string | - | Existing local video/audio file. |
| `transcribe` | `extract_audio_only` | no | boolean | `false` | Extract WAV without ASR. |
| `ocr` | `input_paths` | yes | string[] | - | 1-32 existing local image paths. |
| `ocr` | `preprocess` | no | boolean | `true` | Try enhanced image variants when Pillow is available. |
| `ocr` | `psm` | no | integer | `6` | Tesseract page segmentation mode, 0-13. |
| `prepare_x` | `input_path` | yes | string | - | Existing video file or directory. |
| `prepare_x` | `check_only` | no | boolean | `false` | Check compatibility without transcoding. |
| `prepare_x` | `force` | no | boolean | `false` | Transcode even when already compatible. |
| `prepare_x` | `crf` | no | integer | `23` | H.264 quality value, 16-35. |
| mutating actions | `overwrite` | no | boolean | `false` | Replace an existing output in the task artifact directory. |
| non-capability actions | `operation_timeout_seconds` | no | integer | `900` | Wrapper subprocess timeout, 5-3500 seconds. |

## Dependencies and Configuration

- Base Douyin/Kuaishou/Xiaohongshu/TikTok download and URL resolution: Python 3.10+ standard library.
- YouTube download/resolve: `yt-dlp` in `PATH`.
- Media information, audio extraction, and X conversion: `ffmpeg` and `ffprobe` in `PATH`.
- OCR: `tesseract` plus requested language packs; Pillow only improves preprocessing.
- Whisper transcription: `whisper-cli` in `PATH` (or `WHISPER_BIN`/`WHISPER_CPP_BIN`/`WHISPER_CLI`) and a compatible local model selected through `WHISPER_MODEL`/`WHISPER_MODEL_PATH`/`WHISPER_CPP_MODEL`.
- FunASR transcription: an environment where the bundled Python runtime can import FunASR and its models.
- No raw cookies, cookie files, API tokens, credential environment variables, or package-install actions are accepted.

## Success Contract

Success returns `status=ok` and stable `extra` fields:

- `schema_version`: integer, currently `1`.
- `source_skill`: `media_download`.
- `action`: executed action.
- `count`: number of returned URLs or generated artifacts.
- `urls`: direct URLs for `resolve`; otherwise an empty array.
- `artifacts`: generated files with `path`, `filename`, `mime_type`, and `size_bytes`.
- `output_directory`: runtime-provided task artifact directory.
- `diagnostics`: bounded stderr diagnostics; never use this field for routing or success detection.

## Error Contract

- Errors return `status=error`, non-empty `error_text`, and canonical `extra.{schema_version,source_skill,status,error_code,message_key,retryable}`.
- Stable `error_code` values include `invalid_args`, `missing_argument`, `unsupported_action`, `permission_denied`, `not_found`, `dependency_unavailable`, `media_not_found`, `timeout`, `execution_failed`, and `schema_error`.
- Always surface the bounded downloader diagnostic in `error_text` and `extra.diagnostics` when a download fails. The caller must report that readable reason to the originating communication channel or UI; it must not silently convert the failure into success.
- Partial files may appear in `extra.artifacts` after an execution failure. Treat them as partial outputs, not success evidence.
- Do not parse `text`, `error_text`, or `diagnostics` to make program decisions.

## Structured Evidence Contract

- `extra.urls` can provide URL-list evidence for `resolve` after admission validation.
- `extra.count` can provide count evidence.
- `extra.artifacts[*].path` can provide delivery-artifact evidence after the runtime materializes it.
- Share URLs, local paths, OCR text, transcripts, and diagnostics can contain user data. Do not expose them outside the requesting task.

## Request/Response Examples

### Download a public video

Request:
```json
{"request_id":"media-1","args":{"action":"download","share":"https://v.douyin.com/example/","platform":"auto","save_meta":true},"context":{"artifact_output_directory":"/workspace/.agent-runtime/artifacts/task-1","workspace_root":"/workspace","permissions":{"allow_path_outside_workspace":false}},"user_id":1,"chat_id":1}
```

Response:
```json
{"request_id":"media-1","status":"ok","text":"download completed with 2 files.","error_text":null,"extra":{"schema_version":1,"source_skill":"media_download","status":"ok","action":"download","count":2,"urls":[],"artifacts":[{"path":"/workspace/.agent-runtime/artifacts/task-1/video.mp4","filename":"video.mp4","mime_type":"video/mp4","size_bytes":12345},{"path":"/workspace/.agent-runtime/artifacts/task-1/video.json","filename":"video.json","mime_type":"application/json","size_bytes":456}],"output_directory":"/workspace/.agent-runtime/artifacts/task-1","diagnostics":""}}
```

The `share` field can also be the full copied text, for example `"复制这条消息，打开快手看看 https://v.kuaishou.com/example/ 更多内容"`.

### Resolve without downloading

Request:
```json
{"request_id":"media-2","args":{"action":"resolve","share":"https://youtu.be/example"},"context":{"artifact_output_directory":"/workspace/.agent-runtime/artifacts/task-2","workspace_root":"/workspace","permissions":{"allow_path_outside_workspace":false}},"user_id":1,"chat_id":1}
```

Response:
```json
{"request_id":"media-2","status":"ok","text":"resolve completed with 1 URL.","error_text":null,"extra":{"schema_version":1,"source_skill":"media_download","status":"ok","action":"resolve","count":1,"urls":["https://media.example/video"],"artifacts":[],"output_directory":"/workspace/.agent-runtime/artifacts/task-2","diagnostics":""}}
```

### OCR local images

Request:
```json
{"request_id":"media-3","args":{"action":"ocr","input_paths":["uploads/page-1.png","uploads/page-2.png"],"language":"chi_sim+eng"},"context":{"artifact_output_directory":"/workspace/.agent-runtime/artifacts/task-3","workspace_root":"/workspace","permissions":{"allow_path_outside_workspace":false}},"user_id":1,"chat_id":1}
```

Response:
```json
{"request_id":"media-3","status":"ok","text":"ocr completed with 1 file.","error_text":null,"extra":{"schema_version":1,"source_skill":"media_download","status":"ok","action":"ocr","count":1,"urls":[],"artifacts":[{"path":"/workspace/.agent-runtime/artifacts/task-3/page-1_ocr.txt","filename":"page-1_ocr.txt","mime_type":"text/plain","size_bytes":321}],"output_directory":"/workspace/.agent-runtime/artifacts/task-3","diagnostics":""}}
```
