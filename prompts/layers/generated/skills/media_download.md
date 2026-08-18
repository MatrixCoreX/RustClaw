<!-- AUTO-GENERATED: sync_skill_docs.py -->
## Role & Boundaries
- You are the `media_download` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `optional_skills/media_download/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
- Download publicly accessible media from Douyin, Kuaishou, Xiaohongshu, TikTok, and YouTube share text or URLs. Douyin and Xiaohongshu image-article posts return every extracted original image plus the platform article by default. Up to nine images are delivered individually; ten or more are placed in one ordered ZIP so channel attachment limits cannot silently drop later images. Normal delivery always includes the complete article inline and as a separate UTF-8 text artifact, independent of article length; a large image-set ZIP also contains the article file as a durable copy.
- Resolve direct media URLs without downloading.
- Provide the local video/audio transcription fallback, OCR local images, and prepare local video for X. Configured remote STT is attempted first; this skill's Whisper/FunASR path is used when the configured STT target is local or the remote attempt fails. For a structured `zh-CN`, `zh-SG`, or `zh-Hans` target, both local ASR engines deterministically normalize traditional Chinese characters to simplified Chinese with the package-locked OpenCC dependency before model review; explicit traditional-Chinese and non-Chinese targets are preserved. Local image OCR runs Tesseract first and then uses the skill's granted internal LLM gateway for layout and typo review; unavailable review falls back to raw OCR text.
- Write user-deliverable files to the runtime-provided task artifact directory. For Douyin/Xiaohongshu profile collection, the host also provides this skill's private storage for resumable, content-addressed checkpoints; the skill never reads the runtime database or another skill's storage.
- Never read system-browser cookies. The skill supports public content only and does not bypass DRM, private-content access, paywalls, or platform authorization.
- This document teaches usage only. Host admission and policy grants remain authoritative.

## Planner Selection Notes (from interface)
- This skill owns immediate delivery of one current share or URL. A copied
  share payload is not a batch-collection request merely because it names a
  supported social platform. Use `media_discovery` only for an explicitly
  requested feed, keyword, batch, scheduled, or continuous collection
  workflow.
- “帮我下载这条抖音/快手/小红书/TikTok/YouTube 视频” -> `download`
- A message consisting only of a supported public URL -> `download`; do not ask whether the user wants resolution or download.
- A complete copied app-share message for any supported platform containing a supported URL -> pass the complete text as `share` and run `download` immediately.
- A supported website share URL -> pass the complete URL as `share` and run `download`. Common website forms include Douyin `/video/<id>` or `/note/<id>`, Kuaishou `/short-video/<id>`, Xiaohongshu `/explore/<note-id>` or `/discovery/item/<note-id>`, TikTok `/@user/video/<id>`, and YouTube `watch`, `shorts`, `live`, or `youtu.be` URLs. Preserve the original query string.
- Xiaohongshu App and website shares are distinct valid inputs: App copy text commonly carries an `xhslink.cn` or `xhslink.com` short link, while website sharing commonly carries a full `xiaohongshu.com` note URL. Do not ask the user to convert one form into the other.
- Input behavior is channel-neutral: the same App share text, short link, or website URL must select the same `download` action from the UI, WeChat, WhatsApp, Telegram, Feishu/Lark, or any other host channel.
- Resource-heavy actions use an explicit host-enforced FIFO per user. A second media task from the same user waits before another runner process is spawned; completion, structured failure, timeout, cancellation, or abnormal process loss releases that user's lane so the next task continues. Different users may run in parallel up to the host's existing global skill concurrency limit. `capabilities` and skills without a `dispatch_queue` declaration keep their prior dispatch behavior.
- `media_download.download` and `media_download.transcribe` have no whole-operation deadline. A compatibility `operation_timeout_seconds` value is ignored for these actions and is not exposed to the planner. Per-request network timeouts, explicit cancellation, durable background polling, and renewable retention remain active.
- Profile collection uses the platform work ID as its stable item identity. After every completed or failed item it atomically advances a monotonic cursor and writes an immutable `partial` checkpoint snapshot in private skill storage. A safe retry restores verified completed artifacts by SHA-256 and processes only remaining IDs; it writes a distinct immutable `complete` snapshot only after every currently listed item and its artifact manifest verify successfully.
- When the user wants the images/video themselves, prefer `media_download.download` over a general browser capability. Use a browser only when the requested output is page text, comments, navigation, or a page summary rather than the media files.
- These are semantic capability rules for the planner, not runtime phrase matching. Select by the current request shape and the newest concrete target.
- Ordinary media download requests must stop after `media_download.download`. Video posts return original video media. Image-article posts from Douyin or Xiaohongshu return the original images and the platform-provided title/body by default. Up to nine images remain individual delivery artifacts; ten or more are preserved in one ZIP in source order. Normal delivery includes the complete body in the skill response and keeps `_article.txt` as a separate delivery artifact; a large image-set ZIP also contains the article. This media-download-only rule does not change delivery behavior for any other skill. The article is first-party post content, not OCR; never recognize text inside images or transcribe video/audio unless the current user explicitly asks for that conversion.
- Default download delivery is `deliver_to_user=true`: send the downloaded media back to the originating communication channel and expose it in the UI.
- If the user explicitly says not to send the media back (for example, “不要发我”), set `deliver_to_user=false`. Do not remove that phrase from `share`; the downloader still extracts the URL from the complete text. Reply with the saved local path only and do not emit a delivery artifact.
- “只解析这个分享链接的媒体直链，不要下载” -> `resolve`
- “把这个视频转成文字” uses the configured-STT-first pipeline and sets `text_conversion_scope=all`. The download step extracts a non-delivered first-frame image for best-effort `image_vision.extract_text`, while the video audio follows the transcription pipeline. In the final model-authored answer, label the two sections in the user's language as video-first-frame text and audio transcript; never imply that the first-frame result covers the whole video. If the first frame has no recognizable text, say so. For audio, first call `audio.preview_transcribe` on the downloaded video path; the audio provider treats MP4 as an audio-bearing input. If preview reports `provider_location=remote`, call `audio.transcribe`; if it reports `local`, or the remote call returns a structured failure, call `media_download.transcribe` on the same video path for local extraction and recognition. Never send the video file itself to image OCR. If the user asks only to extract audio, set `extract_audio_only=true` and keep normal delivery enabled.
- Only when the user explicitly requests image text recognition, prefer `image_vision.extract_text` and pass the downloaded/local image paths as `images`. It recognizes and model-reviews one UTF-8 `.txt` artifact. Use this skill's `ocr` action when multimodal vision is unavailable, returns a structured failure, or the user explicitly asks to start with local Tesseract.
- When one request asks both to download media and convert it to text, set `text_conversion_scope=all` unless the user explicitly narrows the request to only images or only audio. Use `images_only` or `audio_only` only for that explicit restriction; use `none` only when the user explicitly refuses conversion. Then run `media_download.download` and branch on `extra.content_bundle.kind`: pass current-task individual image artifacts to `image_vision.extract_text`; when a large set was packaged, pass the ordered original image paths from `extra.processing_inputs.images` instead of the ZIP. For video, follow the internal-WAV, configured-STT preview, remote-first/local-fallback sequence above. For `image_audio` or `image_audio_article`, an unqualified text-conversion request completes every machine step in `extra.content_bundle.followup_policy.steps`: recognize all ordered images and transcribe `extra.processing_inputs.background_audio`, then synthesize the platform article, image text, and audio transcript without dropping a source. Never send a ZIP, video, or audio path to image OCR. Use paths returned by the successful download step in this task; do not substitute a path recalled from an older task unless the user explicitly refers to that older artifact.
- Before any potentially long media action (`download`, `resolve`, `transcribe`, `ocr`, or `prepare_x`), create a `task_plan` containing every applicable phase, even when the request uses only one skill action. Write every plan title in the user's current language; these model-authored titles are the only text that a communication channel may use for live step updates. Use the stable step IDs emitted by this skill when applicable: `media_precheck`, `download_media`, `resolve_media`, `extract_audio`, `transcribe_speech`, `recognize_images`, and `prepare_media`. Transcript review is performed by the normal model-synthesis phase after STT. The communication adapter may match a progress frame to any plan step by `step_id` even while the persisted plan status is still pending, then the planner updates statuses after the action returns. Never expose or translate a progress `detail_key`, and never invent a canned channel reply when the matching plan title is absent.
- When this skill's local `ocr` fallback is used, model review occurs before delivery. Normal delivery returns the complete reviewed text inline and keeps the same text as a `.txt` artifact. If review fails, the same dual-delivery contract applies to the raw OCR text.
- “检查或转换成可以发到 X/Twitter 的视频” -> `prepare_x`
- Prefer a dedicated built-in media skill when the request is unrelated to this package's supported actions.


## Config Entry Points (from interface)
- No dedicated config entry points declared.

## Actions (from interface)
### `capabilities`

Return supported actions, platforms, security defaults, and optional dependencies. No network or filesystem access is needed.

### `download`

Extract the supported URL from `share` and download public media. `share` may be a URL by itself or complete copied App share text from any supported platform. App short links and full website share URLs are equally valid, and the complete input must be preserved across UI and communication-channel entry points. Xiaohongshu accepts both App share text containing `xhslink.cn`/`xhslink.com` and website share URLs under `xiaohongshu.com`. Do not require the user to clean the copied message or resend only the URL. A profile URL can download multiple recent works with `profile_limit`; use `"all"` only when the user explicitly requests every accessible item.

The `download` action produces original image/video media. For a Douyin or Xiaohongshu image-article post, it also extracts the exact platform-provided title/body without requiring a separate request. A Douyin image post can additionally carry background audio; when its exact payload exposes `music.play_url` or `music.playUrl`, the downloader preserves that audio as `<output-stem>_background_audio.<ext>` with `artifact_role=background_audio` instead of discarding it. The Douyin path prefers exact structured payloads and falls back to the rendered exact-item page when public payloads expose images but omit the article field. It retries a missing article and refuses partial image-post delivery if the complete platform article still cannot be verified; it never silently reports an image-only success for that condition. Up to nine images are returned individually. Ten or more images are stored in source order in a single `image_bundle.zip`; the originals remain in the invocation directory and their ordered descriptors are returned under `extra.processing_inputs.images` for an explicitly requested OCR continuation. With normal delivery enabled, return the complete normalized body through `text` and `extra.article_delivery`, and retain `<output-stem>_article.txt` as a delivery artifact regardless of text length. Whenever a large image ZIP is created, it also includes the platform article file. It never treats the copied share-message preview as the full article and never adds OCR text or a transcript by itself. When the same request explicitly asks to convert the downloaded work to text, inspect `extra.content_bundle.kind`: individual images or `extra.processing_inputs.images` go to `image_vision.extract_text`, video goes through configured STT, and an image-plus-audio bundle requires both image recognition and background-audio transcription before synthesis. A ZIP, video, or audio file must never go to `ocr`.

For Douyin video posts, preserve an ordinary MP4 with embedded audio as-is. When the browser exposes separate adaptive video and audio streams, match only streams from the same CDN path group and combine them into one MP4 before delivery. Existing live-photo composition remains separate. A genuinely silent ordinary video may still succeed, but a known video-only adaptive stream must not be reported as a successful original video when its matching audio cannot be obtained or combined.

For a video bundle with `text_conversion_scope=all`, the download step extracts one non-delivered first-frame PNG under `extra.processing_inputs.video_first_frame`. `extra.content_bundle.followup_policy` requires both `video_first_frame` recognition and `video_audio` transcription. Its `result_label_kinds` and `source_label_requirement` require the final model-authored answer to identify `video_first_frame_text` and `audio_transcript` separately in the user's language. The first-frame label is mandatory because it is not whole-video visual recognition; the audio label is mandatory so a transcript is never presented as unqualified source text. An explicit `audio_only` request keeps only audio transcription; an explicit `images_only` request keeps only first-frame recognition.

For an image-plus-audio bundle, `extra.content_bundle.kind` is `image_audio` or `image_audio_article`; `audio_count` reports the preserved background-audio stream. `extra.processing_inputs` contains the ordered images and exact `background_audio` descriptor. With `text_conversion_scope=all`, its `followup_policy` declares `activation_requirement=required`, `completion_requirement=all_components`, one `image_vision.extract_text` step, and an audio route that first calls `audio.preview_transcribe` and then requires the selected `audio.transcribe` or `media_download.transcribe` completion capability. `recommended_capability_pointer` reads the preview's structured route without parsing prose. The three synthesis sources are `platform_article`, `image_text`, and `audio_transcript`. These fields are the continuation contract; the caller must not finish after only one component succeeds or after preview alone. `images_only` and `audio_only` produce a single explicitly selected step so the omitted component is not run.

After success, expose every generated delivery artifact through the task artifact contract when `deliver_to_user` is omitted or `true`. Large image sets replace individual delivery artifacts with one ZIP, while the structured `content_bundle.image_count` and `processing_inputs.image_count` retain the complete logical count. Profile collection also emits `profile_downloads.json` with `artifact_role=profile_manifest`, stable IDs, high-water cursor, per-file sizes/SHA-256 values, and explicit `state=complete`; a partial run remains an error and cannot masquerade as successful completion. The originating communication adapter can send the file back, and the UI can render its preview/download URL. Host channel size limits still apply. When `deliver_to_user=false`, do not package the set: return every generated file under `extra.saved_files`, keep `extra.artifacts` empty, and report the saved path without sending the media.

### `resolve`

Resolve downloadable media URLs from `share` without saving the media.

### `transcribe`

Transcribe `input_path` with this skill's local Whisper/FunASR fallback, or only extract WAV audio when `extract_audio_only=true`. For ordinary transcription, do not select local ASR first: preview the configured STT through `audio.preview_transcribe`, use remote STT when available, and select this action only when the configured target is local or the remote attempt fails. Local success returns the complete raw transcript in `extra.transcription_review`; the shared host finalizer then corrects recognition homophones, typos, punctuation, and broken sentences without summarizing or adding facts, and translates the complete corrected transcript when the response language differs from the source. `response_language` overrides the task locale; otherwise the runtime-provided task locale is authoritative.

The intermediate WAV and raw local transcript remain under `extra.saved_files` and are not delivery artifacts during normal transcription. When `extract_audio_only=true` and `deliver_to_user=false`, `extra.processing_outputs.extracted_audio` carries the exact WAV descriptor and `extra.followup_policy` binds its exact path to `audio.preview_transcribe`; the same path is also the declared input for the local fallback. The caller must consume that machine contract instead of guessing an artifact directory. `extra.transcription_review` is a machine contract for the shared finalizer, not user-visible text. After model review, normal delivery includes the complete reviewed transcript inline and as the UTF-8 `transcript.txt` artifact regardless of text length. Only `extract_audio_only=true` with normal delivery enabled sends WAV to the user.

### `ocr`

Run Tesseract OCR for image-only `input_paths`, then call the granted internal LLM gateway inside this skill to restore sentence boundaries, paragraph layout, punctuation, and highly certain recognition errors. The review merges visual soft wraps caused only by image width and preserves line breaks for real paragraphs, headings, lists, tables, code, verse, and other line-oriented structures. When `language` is omitted or `auto`, local OCR uses every installed Tesseract recognition language without script-specific scoring or short-number deletion. The complete review is Unicode-safe and preserves source languages, facts, names, numeric tokens, ordering, and uncertainty; it never summarizes or invents content. If any chunk fails or violates the language-neutral integrity gate, the raw OCR result is retained and delivered instead of failing the action. A successful review keeps the unmodified source in a non-delivered `raw_artifact` for audit. Multiple inputs remain in source order without image labels. Video/audio inputs are rejected before dispatch. With normal delivery enabled, return the complete final text inline and retain `image_text_ocr.txt` as a delivery artifact regardless of text length.

### `prepare_x`

Check or transcode `input_path` for X compatibility. Directories are scanned recursively. Set `check_only=true` to avoid writing converted media.

## Parameter Contract (from interface)
| Action | Parameter | Required | Type | Default | Description |
|---|---|---:|---|---|---|
| all | `action` | yes | string | - | One of `capabilities`, `download`, `resolve`, `transcribe`, `ocr`, `prepare_x`. |
| `download`, `resolve` | `share` | yes | string | - | A public URL or an unmodified copied share message containing the URL. |
| `download`, `resolve` | `platform` | no | string | `auto` | `auto`, `douyin`, `kuaishou`, `xiaohongshu`, `tiktok`, or `youtube`. |
| `download` | `text_conversion_scope` | no | string | omitted/conditional | Use `all` for an unqualified “转文字” request, `images_only` or `audio_only` only when the user explicitly restricts the request, and `none` only for an explicit no-conversion request. |
| `download` | `deliver_to_user` | no | boolean | `true` | Send generated media to the originating channel/UI. Set `false` only when the user explicitly asks not to receive the file; then report its saved path. |
| `download` | `output_name` | no | string | timestamp | Plain filename only; directories are rejected. |
| `download`, `resolve` | `profile_limit` | no | integer or `all` | `20` | Maximum profile items. Use `all` only after explicit user intent. |
| `download`, `resolve` | `profile_interval_seconds` | no | number | `5` | Delay between profile requests, 0-60 seconds. |
| `download`, `resolve` | `network_timeout_seconds` | no | number | `20` | Per-request timeout, 1-120 seconds. |
| `download`, `resolve` | `browser_fallback` | no | boolean | `true` | Allow local Chromium fallback without browser-profile cookies. |
| `download` | `save_meta` | no | boolean | `false` | Save extraction metadata JSON. |
| `download` | `show_info` | no | boolean | `false` | Probe downloaded media information. |
| `transcribe` | `engine` | no | string | `whisper` | Uses the configured local whisper.cpp CLI/model by default. `funasr` is the privately installed alternative. |
| `transcribe`, `ocr` | `language` | no | string | action-specific | Spoken language or Tesseract language list. OCR defaults to `auto`, which uses installed recognition data. |
| `transcribe` | `response_language` | no | string | task locale | Target language for the complete model-reviewed transcript. Use only when the user explicitly requests a target language; otherwise omit it. |
| `transcribe` | `deliver_to_user` | no | boolean | `true` | Set `false` for the internal WAV extraction step before configured STT preview; keep `true` for a user-requested audio-only result. |
| `transcribe` | `input_path` | yes | string | - | Existing local video/audio file. |
| `transcribe` | `extract_audio_only` | no | boolean | `false` | Extract WAV without ASR. |
| `ocr` | `input_paths` | yes | string[] | - | One or more existing local image paths with a supported image extension. No arbitrary image-count ceiling is applied; video/audio paths are invalid and must use `transcribe`. |
| `ocr` | `output_name` | no | string | `image_text_ocr.txt` | Plain `.txt` filename. The default name identifies local OCR output. |
| `ocr` | `deliver_to_user` | no | boolean | `true` | Send the generated text file to the originating channel/UI. Set false only when the user explicitly asks not to receive it. |
| `ocr` | `preprocess` | no | boolean | `true` | Try enhanced image variants when Pillow is available. |
| `ocr` | `psm` | no | integer | `6` | Tesseract page segmentation mode, 0-13. |
| `prepare_x` | `input_path` | yes | string | - | Existing video file or directory. |
| `prepare_x` | `check_only` | no | boolean | `false` | Check compatibility without transcoding. |
| `prepare_x` | `force` | no | boolean | `false` | Transcode even when already compatible. |
| `prepare_x` | `crf` | no | integer | `23` | H.264 quality value, 16-35. |
| mutating actions | `overwrite` | no | boolean | `false` | Replace an existing output in the task artifact directory. |
| `resolve`, `ocr`, `prepare_x` | `operation_timeout_seconds` | no | integer | none | Optional user-explicit wrapper subprocess deadline, 5-2592000 seconds. It is not available for `download` or `transcribe`; both have no whole-operation deadline. Large explicit values are enforced through bounded wait slices so platform poll limits cannot overflow. |

## Error Contract (from interface)
- Errors return `status=error`, non-empty `error_text`, and canonical `extra.{schema_version,source_skill,status,error_code,message_key,retryable}`.
- Stable `error_code` values include `invalid_args`, `missing_argument`, `unsupported_action`, `permission_denied`, `not_found`, `dependency_unavailable`, `media_not_found`, `artifact_packaging_failed`, `timeout`, `execution_failed`, and `schema_error`.
- Always surface the bounded downloader diagnostic in `error_text` and `extra.diagnostics` when a download fails. The caller must report that readable reason to the originating communication channel or UI; it must not silently convert the failure into success.
- Before dispatch, errors declare `failure_phase=pre_dispatch` and `side_effect_applied=false`. If an executed media command fails, the adapter removes files newly created in the invocation-private artifact directory and verifies the directory snapshot; successful cleanup declares `failure_phase=execution_no_effect` and `side_effect_applied=false`, allowing the host to terminalize the failed task and deliver its readable error to UI or the originating communication channel.
- If output cleanup cannot prove that the invocation left no effect, the error declares `failure_phase=execution_partial` and `side_effect_applied=true`; any listed `extra.artifacts` are partial outputs and never success evidence.
- A profile run with one or more remaining failed IDs returns `execution_failed`; its task artifact directory is rolled back, while verified resumable blobs and an immutable `partial` snapshot remain only in this skill's private storage for a safe retry. Corrupt or conflicting cached artifacts fail explicitly instead of being silently replayed.
- Do not parse `text`, `error_text`, or `diagnostics` to make program decisions.

## Structured Evidence Contract (from interface)
- `extra.urls` can provide URL-list evidence for `resolve` after admission validation.
- `extra.count` can provide count evidence.
- `extra.artifacts[*].path` can provide delivery-artifact evidence after the runtime materializes it.
- Share URLs, local paths, OCR text, transcripts, and diagnostics can contain user data. Do not expose them outside the requesting task.

## Request/Response Examples (from interface)
### Download a Douyin image-article post

Request:
```json
{"request_id":"media-1","args":{"action":"download","share":"复制打开抖音，看看这个图文作品 https://v.douyin.com/example/","platform":"auto"},"context":{"artifact_output_directory":"/workspace/.agent-runtime/artifacts/task-1","workspace_root":"/workspace","permissions":{"allow_path_outside_workspace":false}},"user_id":1,"chat_id":1}
```

Response:
```json
{"request_id":"media-1","status":"ok","text":"Example platform article body.","error_text":null,"extra":{"schema_version":1,"source_skill":"media_download","status":"ok","action":"download","count":3,"urls":[],"artifacts":[{"path":"/workspace/.agent-runtime/artifacts/task-1/note_01.webp","filename":"note_01.webp","mime_type":"image/webp","size_bytes":12345,"artifact_role":"original_image"},{"path":"/workspace/.agent-runtime/artifacts/task-1/note_02.webp","filename":"note_02.webp","mime_type":"image/webp","size_bytes":23456,"artifact_role":"original_image"},{"path":"/workspace/.agent-runtime/artifacts/task-1/note_article.txt","filename":"note_article.txt","mime_type":"text/plain","size_bytes":30,"artifact_role":"article_text","content_source":"platform_post"}],"content_bundle":{"schema_version":1,"kind":"image_article","image_count":2,"video_count":0,"article_count":1,"other_file_count":0,"inline_article_count":1},"article_delivery":{"mode":"inline_and_artifact","content_source":"platform_post","character_count":30,"text":"Example platform article body.","artifact_path":"/workspace/.agent-runtime/artifacts/task-1/note_article.txt","artifact_filename":"note_article.txt"},"delivery":{"intent":"artifact","deliver_to_user":true},"output_directory":"/workspace/.agent-runtime/artifacts/task-1","diagnostics":""}}
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
{"request_id":"media-3","status":"ok","text":"Reviewed OCR text.","error_text":null,"extra":{"schema_version":1,"source_skill":"media_download","status":"ok","action":"ocr","count":1,"urls":[],"artifacts":[{"path":"/workspace/.agent-runtime/artifacts/task-3/image_text_ocr.txt","filename":"image_text_ocr.txt","mime_type":"text/plain","size_bytes":18,"artifact_role":"recognized_text","recognition_source":"local_ocr","recognition_engine":"tesseract"}],"recognition":{"source":"local_ocr","engine":"tesseract"},"recognition_delivery":{"mode":"inline_and_artifact","source":"local_ocr","engine":"tesseract","character_count":18,"text":"Reviewed OCR text.","artifact_path":"/workspace/.agent-runtime/artifacts/task-3/image_text_ocr.txt","artifact_filename":"image_text_ocr.txt"},"delivery":{"intent":"artifact","deliver_to_user":true},"output_directory":"/workspace/.agent-runtime/artifacts/task-3","diagnostics":""}}
```

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
- For setup or configuration questions about this skill, treat the config entry points section as the grounding source for where changes actually live.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Interpret Chinese colloquial phrasing by capability semantics and requested task shape, not by a fixed phrase list.
- Judge Chinese delivery intent semantically: if the user asks to receive a file/result rather than inline body text, plan toward delivery without depending on fixed wording.
- Preserve Chinese brevity and format constraints as final output contracts when the skill can support them; do not convert those constraints into token-level matching rules.
- Treat Chinese style constraints as audience/tone constraints for the eventual explanation, not as skill-selection shortcuts.
- Resolve Chinese deictic references only from immediate, concrete, type-compatible context; do not guess unsupported targets or invent missing args just to force a skill call.
